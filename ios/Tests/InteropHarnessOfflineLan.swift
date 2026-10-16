import Darwin
import Foundation
import SQLite3
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

/// Only instantiated by the explicitly selected XCTest action in an isolated
/// account. A bound, non-listening socket keeps the failed HTTP endpoint reserved.
final class OfflineLanFixture {
    static let byteCount = 5 * 1_024 * 1_024
    static let filename = "offline-lan.bin"
    let directory: URL
    let httpURL: String
    let configuredAtMS: Int64
    private var socketFD: Int32

    init(directory: URL) throws {
        self.directory = directory
        let descriptor = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw HarnessError.unexpected("failed HTTP endpoint socket") }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let named = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                getsockname(descriptor, $0, &length)
            }
        }
        guard bound == 0, named == 0 else {
            Darwin.close(descriptor)
            throw HarnessError.unexpected("could not reserve failed HTTP endpoint")
        }
        httpURL = "http://127.0.0.1:\(UInt16(bigEndian: address.sin_port))"
        configuredAtMS = Int64(Date().timeIntervalSince1970 * 1_000)
        socketFD = descriptor
    }

    func close() {
        if socketFD >= 0 { Darwin.close(socketFD); socketFD = -1 }
    }

    deinit { close() }
}

extension InteropHarnessTests {
    func prepareOfflineLanFixture(
        action: String, runID: String, dataDir: URL, env: [String: String]
    ) throws -> OfflineLanFixture? {
        guard action.hasPrefix("offline_lan_attachment_") else { return nil }
        guard env["IRIS_IOS_HARNESS_OFFLINE_LAN"] == "1",
              env["IRIS_IOS_HARNESS_USE_APP_STORAGE"] == "0",
              env["IRIS_IOS_HARNESS_USE_KEYCHAIN"] != "1",
              env["IRIS_IOS_HARNESS_SERVICE"] == nil,
              env["IRIS_IOS_HARNESS_RESET"] != "1",
              runID.range(of: "^offline-lan-[A-Za-z0-9-]{8,80}$", options: .regularExpression) != nil else {
            throw HarnessError.unexpected("offline LAN requires explicit isolated harness storage")
        }
        guard FileManager.default.fileExists(atPath: dataDir.appendingPathComponent("account-secret.json").path) else {
            throw HarnessError.unexpected("bootstrap the isolated account and accepted chat first")
        }
        if action != "offline_lan_attachment_prepare" {
            guard offlineLanPreferencesPersisted(dataDir: dataDir) else {
                throw HarnessError.unexpected("run offline_lan_attachment_prepare, then restart the fixture")
            }
        }
        let directory = dataDir.appendingPathComponent("offline-lan-fixture", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // Saved public endpoints cannot survive the transition. Keep the exact
        // isolated-account file for diagnosis instead of deleting its contents.
        let recent = dataDir.appendingPathComponent("fips-recent-peers.json")
        if FileManager.default.fileExists(atPath: recent.path) {
            try FileManager.default.moveItem(
                at: recent, to: directory.appendingPathComponent("recent-peers-\(UUID().uuidString).json")
            )
        }
        let fixture = try OfflineLanFixture(directory: directory)
        let configDir = directory.appendingPathComponent("hashtree", isDirectory: true)
        try FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        let config = """
        [server]
        bind_address = "127.0.0.1:0"
        [blossom]
        servers = []
        read_servers = ["\(fixture.httpURL)"]
        write_servers = ["\(fixture.httpURL)"]
        """
        try config.write(to: configDir.appendingPathComponent("config.toml"), atomically: true, encoding: .utf8)
        let overrides = [
            "HTREE_CONFIG_DIR": configDir.path,
            "HTREE_LOCAL_DAEMON_ONLY": "0",
            "IRIS_FIPS_WEBSOCKET_SEED_URLS": "",
            "IRIS_CHAT_FIPS_ENABLE_WEBRTC": "0",
            "IRIS_CHAT_FIPS_STATIC_PEERS": "",
            "IRIS_CHAT_FIPS_ROUTED_PEERS": "",
            "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR": "",
            "IRIS_CHAT_FIPS_UDP_BIND_ADDR": "",
            "IRIS_CHAT_SAME_HOST_HASHTREE": "0",
            // Enables bounded observation, not an alternate transport or store.
            "IRIS_CHAT_OFFLINE_LAN_FIXTURE": "1",
        ]
        for (key, value) in overrides { setenv(key, value, 1) }
        status("offline_lan_configured_at_ms", String(fixture.configuredAtMS))
        status("offline_lan_http_endpoint", fixture.httpURL)
        return fixture
    }

    func offlineLanPreferencesPersisted(dataDir: URL) -> Bool {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dataDir.appendingPathComponent("core.sqlite3").path,
                              &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return false
        }
        defer { sqlite3_close(db) }
        var statement: OpaquePointer?
        let query = """
            SELECT nostr_relay_urls_json, nearby_enabled, nearby_lan_enabled, nearby_bluetooth_enabled
            FROM preferences WHERE id = 1
        """
        guard sqlite3_prepare_v2(db, query, -1, &statement, nil) == SQLITE_OK else { return false }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_step(statement) == SQLITE_ROW,
              let raw = sqlite3_column_text(statement, 0),
              let data = String(cString: raw).data(using: .utf8),
              let relays = try? JSONSerialization.jsonObject(with: data) as? [String] else { return false }
        return relays.isEmpty && sqlite3_column_int(statement, 1) == 1 &&
            sqlite3_column_int(statement, 2) == 1 && sqlite3_column_int(statement, 3) == 0
    }

    func offlineLanOpenExistingChat(manager: AppManager, env: [String: String]) async throws -> String {
        _ = try await ensureLoggedIn(manager: manager, env: env)
        let chatID = try requiredEnv("IRIS_IOS_HARNESS_CHAT_ID", env: env)
        let peer = try requiredEnv("IRIS_IOS_HARNESS_PEER_OWNER_HEX", env: env)
        guard let thread = manager.state.chatList.first(where: { sameIdentifier($0.chatId, chatID) }),
              !thread.isRequest else {
            throw HarnessError.unexpected("an existing accepted chat is required")
        }
        manager.dispatch(.openChat(chatId: chatID))
        _ = try await waitForCurrentChat(manager: manager, chatID: chatID, timeout: 30)
        guard let current = manager.state.currentChat,
              current.participants.contains(where: { !$0.isLocalOwner && sameIdentifier($0.ownerPubkeyHex, peer) }),
              let debug = await liveRuntimeDebugSnapshot(manager: manager),
              runtimeDebugHasPeerRoster(debug, peerOwnerHex: peer) else {
            throw HarnessError.unexpected("serving peer must already be a known member of the chat")
        }
        return chatID
    }
}
