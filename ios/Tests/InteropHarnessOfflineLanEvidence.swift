import Darwin
import CoreFoundation
import Foundation
import Network
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

struct OfflineLanWiFiAddress: Codable, Sendable {
    let interface: String
    let address: String
    let netmask: String

    func contains(remote: String) -> Bool {
        func ipv4(_ value: String) -> UInt32? {
            var result = in_addr()
            return inet_pton(AF_INET, value, &result) == 1 ? UInt32(bigEndian: result.s_addr) : nil
        }
        guard let local = ipv4(address), let mask = ipv4(netmask), let peer = ipv4(remote),
              mask != 0, peer != local,
              peer >> 24 != 127, peer >> 16 != 0xA9FE,
              (peer >> 24 == 10 || peer >> 20 == 0xAC1 || peer >> 16 == 0xC0A8) else { return false }
        return (peer & mask) == (local & mask)
    }
}

struct OfflineLanReadEvidence {
    let raw: JsonObject
    let epoch: String
    let reads: [String: UInt64]
    let peers: [JsonObject]

    static func isInitializing(_ raw: JsonObject) -> Bool {
        Set(raw.keys) == Set(["schema", "valid"]) &&
            raw["schema"] as? Int == 1 && raw["valid"] as? Bool == false
    }

    init(_ raw: JsonObject, httpURL: String) throws {
        guard raw["schema"] as? Int == 1, raw["valid"] as? Bool == true,
              raw["isolation_valid"] as? Bool == true,
              raw["blossom_read_servers"] as? [String] == [httpURL],
              raw["blossom_write_servers"] as? [String] == [httpURL],
              let epoch = raw["epoch"] as? String, !epoch.isEmpty,
              let counts = raw["reads"] as? JsonObject,
              let peers = raw["peers"] as? [JsonObject] else {
            throw HarnessError.unexpected("authoritative offline LAN diagnostics unavailable or isolation invalid")
        }
        var values: [String: UInt64] = [:]
        for key in ["local_hits", "local_misses", "peer_blocks", "peer_bytes", "http_blocks", "http_bytes"] {
            guard let number = counts[key] as? NSNumber,
                  CFGetTypeID(number) != CFBooleanGetTypeID(),
                  let value = UInt64(number.stringValue) else {
                throw HarnessError.unexpected("missing or invalid attachment counter: \(key)")
            }
            values[key] = value
        }
        guard values["http_blocks"] == 0, values["http_bytes"] == 0 else {
            throw HarnessError.unexpected("HTTP served attachment data during the offline fixture")
        }
        self.raw = raw
        self.epoch = epoch
        self.reads = values
        self.peers = peers
    }

    func verifyPeers(expected: String, wifi: [OfflineLanWiFiAddress], required: Bool) throws {
        if required && peers.isEmpty { throw HarnessError.unexpected("expected LAN peer is not connected") }
        for peer in peers {
            guard peer["npub"] as? String == expected, peer["transport"] as? String == "udp",
                  let address = peer["address"] as? String,
                  let endpoint = URLComponents(string: "udp://\(address)"),
                  let host = endpoint.host, let port = endpoint.port, (1...65535).contains(port),
                  wifi.contains(where: { $0.contains(remote: host) }) else {
                throw HarnessError.unexpected("unexpected peer or transport outside the Wi-Fi subnet")
            }
        }
    }

    func verifyRead(after: OfflineLanReadEvidence, warm: Bool) throws {
        guard epoch == after.epoch else { throw HarnessError.unexpected("attachment observation epoch changed") }
        var delta: [String: UInt64] = [:]
        for (key, value) in reads {
            guard let next = after.reads[key], next >= value else {
                throw HarnessError.unexpected("attachment counter regressed")
            }
            delta[key] = next - value
        }
        if warm {
            guard (delta["local_hits"] ?? 0) > 0, delta["peer_blocks"] == 0,
                  delta["peer_bytes"] == 0, delta["http_blocks"] == 0 else {
                throw HarnessError.unexpected("restart read did not use only durable local ciphertext")
            }
        } else {
            guard (delta["local_misses"] ?? 0) > 0, (delta["peer_blocks"] ?? 0) >= 2,
                  (delta["peer_bytes"] ?? 0) >= UInt64(OfflineLanFixture.byteCount) else {
                throw HarnessError.unexpected("cold multi-chunk attachment was not fetched from the peer")
            }
        }
    }
}

extension InteropHarnessTests {
    func offlineLanWiFiAddresses() async throws -> [OfflineLanWiFiAddress] {
        let names: [String] = await withCheckedContinuation { continuation in
            let monitor = NWPathMonitor(requiredInterfaceType: .wifi)
            monitor.pathUpdateHandler = { path in
                monitor.pathUpdateHandler = nil
                monitor.cancel()
                continuation.resume(returning: path.availableInterfaces.filter { $0.type == .wifi }.map(\.name))
            }
            monitor.start(queue: DispatchQueue(label: "iris.harness.offline-lan-wifi"))
        }
        var first: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&first) == 0 else { throw HarnessError.unexpected("Wi-Fi interface query failed") }
        defer { freeifaddrs(first) }
        var pointer = first
        var result: [OfflineLanWiFiAddress] = []
        while let current = pointer {
            let entry = current.pointee
            pointer = entry.ifa_next
            let name = String(cString: entry.ifa_name)
            guard names.contains(name), let address = entry.ifa_addr, let mask = entry.ifa_netmask,
                  address.pointee.sa_family == UInt8(AF_INET), entry.ifa_flags & UInt32(IFF_UP) != 0,
                  entry.ifa_flags & UInt32(IFF_LOOPBACK | IFF_POINTOPOINT) == 0 else { continue }
            func string(_ value: UnsafeMutablePointer<sockaddr>) -> String? {
                var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                guard getnameinfo(value, socklen_t(value.pointee.sa_len), &buffer, socklen_t(buffer.count),
                                  nil, 0, NI_NUMERICHOST) == 0 else { return nil }
                return String(cString: buffer)
            }
            if let ip = string(address), let netmask = string(mask) {
                result.append(OfflineLanWiFiAddress(interface: name, address: ip, netmask: netmask))
            }
        }
        guard !result.isEmpty else { throw HarnessError.unexpected("physical Wi-Fi IPv4 interface unavailable") }
        status("offline_lan_wifi", String(decoding: try JSONEncoder().encode(result), as: UTF8.self))
        return result
    }

    func offlineLanEvidence(manager: AppManager, fixture: OfflineLanFixture, label: String) async throws -> OfflineLanReadEvidence {
        var observed: Result<OfflineLanReadEvidence, Error>?
        let query = Task { @MainActor in
            while !Task.isCancelled {
                do {
                    let bundle = try jsonObjectFromString(await manager.supportBundleJsonAsync())
                    guard !Task.isCancelled else { return }
                    guard var raw = bundle["offline_lan_attachment"] as? JsonObject else {
                        throw HarnessError.unexpected("offline LAN diagnostic hook is not integrated")
                    }
                    if OfflineLanReadEvidence.isInitializing(raw) {
                        try await Task.sleep(nanoseconds: 200_000_000)
                        continue
                    }
                    guard let services = bundle["fips_services"] as? JsonObject,
                          services["valid"] as? Bool == true,
                          let transport = bundle["fips_transport"] as? JsonObject,
                          transport["valid"] as? Bool == true else {
                        throw HarnessError.unexpected("authoritative offline transport counters unavailable")
                    }
                    // Keep only the same-query carrier diagnostics, never the
                    // full support bundle or private account/protocol records.
                    raw["fips_services"] = services
                    raw["fips_transport"] = transport
                    observed = .success(try OfflineLanReadEvidence(raw, httpURL: fixture.httpURL))
                    return
                } catch {
                    observed = .failure(error)
                    return
                }
            }
        }
        defer { query.cancel() }
        let result: Result<OfflineLanReadEvidence, Error> = try await waitFor(
            label: "offline LAN probe initialization", timeout: 10
        ) { observed }
        let evidence = try result.get()
        let encoded = try JSONSerialization.data(withJSONObject: evidence.raw, options: [.sortedKeys])
        try encoded.write(to: fixture.directory.appendingPathComponent("\(label)-\(UUID().uuidString).json"), options: .atomic)
        status("offline_lan_\(label)", String(decoding: encoded, as: UTF8.self))
        status("offline_lan_\(label)_at_ms", String(Int64(Date().timeIntervalSince1970 * 1_000)))
        return evidence
    }
}
