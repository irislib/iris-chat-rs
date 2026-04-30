import Foundation
import Network
import Darwin

final class IrisNearbyLanService {
    private static let serviceType = "_iris-chat._tcp"

    private final class ConnectionSlot {
        let id: String
        let connection: NWConnection
        let endpointID: String?
        var assembler: IrisNearbyLanFrameAssembler
        var peerID: String?

        init(
            id: String,
            connection: NWConnection,
            endpointID: String?,
            assembler: IrisNearbyLanFrameAssembler
        ) {
            self.id = id
            self.connection = connection
            self.endpointID = endpointID
            self.assembler = assembler
        }
    }

    private let peerID: String
    private let queue = DispatchQueue(label: "to.iris.chat.nearby.lan")
    private let bodyLengthFromHeader: (Data) -> Int
    private let onFrame: (String, Data) -> Void
    private let onStatus: (String) -> Void

    private var listener: NWListener?
    private var browser: NWBrowser?
    private var connections: [String: ConnectionSlot] = [:]
    private var endpointIDs: Set<String> = []
    private var privateLocalHost: NWEndpoint.Host?
    private var enabled = false

    init(
        peerID: String,
        bodyLengthFromHeader: @escaping (Data) -> Int,
        onFrame: @escaping (String, Data) -> Void,
        onStatus: @escaping (String) -> Void
    ) {
        self.peerID = peerID
        self.bodyLengthFromHeader = bodyLengthFromHeader
        self.onFrame = onFrame
        self.onStatus = onStatus
    }

    func start() {
        queue.async { [weak self] in
            guard let self, !self.enabled else { return }
            guard let localHost = Self.privateLocalHost() else {
                self.updateStatus("Local network unavailable")
                return
            }
            self.privateLocalHost = localHost
            self.enabled = true
            self.updateStatus("Starting")
            self.startListener(localHost: localHost)
            self.startBrowser()
        }
    }

    func stop() {
        queue.async { [weak self] in
            guard let self else { return }
            self.enabled = false
            self.listener?.cancel()
            self.browser?.cancel()
            self.listener = nil
            self.browser = nil
            self.privateLocalHost = nil
            self.endpointIDs.removeAll()
            for slot in self.connections.values {
                slot.connection.cancel()
            }
            self.connections.removeAll()
            self.updateStatus("Off")
        }
    }

    func send(_ frame: Data, excludingPeerID: String?) {
        queue.async { [weak self] in
            guard let self, self.enabled else { return }
            for slot in self.connections.values {
                if let excludingPeerID, slot.peerID == excludingPeerID {
                    continue
                }
                slot.connection.send(content: frame, completion: .contentProcessed { error in
                    if error != nil {
                        self.close(slot.id)
                    }
                })
            }
        }
    }

    func markPeer(connectionID: String, peerID: String) {
        queue.async { [weak self] in
            guard let self else { return }
            self.connections[connectionID]?.peerID = peerID
            self.updateStatus(self.connections.isEmpty ? "Visible" : "Connected")
        }
    }

    func hasPeer(_ peerID: String) -> Bool {
        queue.sync {
            connections.values.contains { $0.peerID == peerID }
        }
    }

    func peerIDs() -> Set<String> {
        queue.sync {
            Set(connections.values.compactMap(\.peerID))
        }
    }

    private func startListener(localHost: NWEndpoint.Host) {
        do {
            let parameters = tcpParameters(localHost: localHost)
            let listener = try NWListener(using: parameters)
            listener.service = NWListener.Service(name: peerID, type: Self.serviceType)
            listener.newConnectionHandler = { [weak self] connection in
                guard let self else {
                    connection.cancel()
                    return
                }
                guard self.isPrivateRemoteEndpoint(connection.endpoint) else {
                    connection.cancel()
                    return
                }
                self.add(connection)
            }
            listener.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    self.updateStatus(self.connections.isEmpty ? "Visible" : "Connected")
                case .failed:
                    self.updateStatus("Local network failed")
                case .cancelled:
                    break
                default:
                    break
                }
            }
            listener.start(queue: queue)
            self.listener = listener
        } catch {
            updateStatus("Local network unavailable")
        }
    }

    private func startBrowser() {
        let browser = NWBrowser(for: .bonjour(type: Self.serviceType, domain: nil), using: tcpParameters())
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            guard let self, self.enabled else { return }
            for result in results {
                self.connectIfNeeded(to: result.endpoint)
            }
        }
        browser.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            if case .failed = state {
                self.updateStatus("Local network failed")
            }
        }
        browser.start(queue: queue)
        self.browser = browser
    }

    private func connectIfNeeded(to endpoint: NWEndpoint) {
        if isOwnService(endpoint) {
            return
        }
        let endpointID = String(describing: endpoint)
        guard endpointIDs.insert(endpointID).inserted else {
            return
        }
        let connection = NWConnection(to: endpoint, using: tcpParameters(localHost: privateLocalHost))
        add(connection, endpointID: endpointID)
    }

    private func add(_ connection: NWConnection, endpointID: String? = nil) {
        let id = UUID().uuidString.lowercased()
        let slot = ConnectionSlot(
            id: id,
            connection: connection,
            endpointID: endpointID,
            assembler: IrisNearbyLanFrameAssembler(bodyLengthFromHeader: bodyLengthFromHeader)
        )
        connections[id] = slot
        connection.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready:
                self.receive(on: id)
                self.updateStatus(self.connections.isEmpty ? "Visible" : "Connected")
            case .failed, .cancelled:
                self.close(id)
            default:
                break
            }
        }
        connection.start(queue: queue)
    }

    private func receive(on connectionID: String) {
        guard let slot = connections[connectionID] else { return }
        slot.connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, complete, error in
            guard let self else { return }
            if let data, !data.isEmpty, let slot = self.connections[connectionID] {
                let frames = slot.assembler.append(data)
                for frame in frames {
                    DispatchQueue.main.async {
                        self.onFrame(connectionID, frame)
                    }
                }
            }
            if complete || error != nil {
                self.close(connectionID)
                return
            }
            self.receive(on: connectionID)
        }
    }

    private func close(_ connectionID: String) {
        guard let slot = connections.removeValue(forKey: connectionID) else { return }
        if let endpointID = slot.endpointID {
            endpointIDs.remove(endpointID)
        }
        slot.connection.cancel()
        updateStatus(enabled ? (connections.isEmpty ? "Visible" : "Connected") : "Off")
    }

    private func updateStatus(_ status: String) {
        DispatchQueue.main.async {
            self.onStatus(status)
        }
    }

    private func tcpParameters(localHost: NWEndpoint.Host? = nil) -> NWParameters {
        let parameters = NWParameters.tcp
        parameters.includePeerToPeer = true
        parameters.prohibitedInterfaceTypes = [.cellular]
        if let localHost {
            parameters.requiredLocalEndpoint = .hostPort(
                host: localHost,
                port: NWEndpoint.Port(rawValue: 0)!
            )
        }
        return parameters
    }

    private func isOwnService(_ endpoint: NWEndpoint) -> Bool {
        if case let .service(name, type, _, _) = endpoint {
            return name == peerID && type == Self.serviceType
        }
        return false
    }

    private func isPrivateRemoteEndpoint(_ endpoint: NWEndpoint) -> Bool {
        guard case let .hostPort(host, _) = endpoint else {
            return false
        }
        return Self.isPrivateHost(String(describing: host))
    }

    private static func isPrivateHost(_ rawHost: String) -> Bool {
        let host = rawHost
            .trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
            .lowercased()
        if host == "localhost" || host == "::1" {
            return true
        }
        if host.hasPrefix("fe80:") || host.hasPrefix("fc") || host.hasPrefix("fd") {
            return true
        }
        let parts = host.split(separator: ".").compactMap { Int($0) }
        guard parts.count == 4 else {
            return false
        }
        let first = parts[0]
        let second = parts[1]
        return first == 10 ||
            first == 127 ||
            (first == 169 && second == 254) ||
            (first == 172 && (16...31).contains(second)) ||
            (first == 192 && second == 168)
    }

    private static func privateLocalHost() -> NWEndpoint.Host? {
        var interfaces: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&interfaces) == 0, let firstInterface = interfaces else {
            return nil
        }
        defer { freeifaddrs(interfaces) }

        var fallback: String?
        var cursor: UnsafeMutablePointer<ifaddrs>? = firstInterface
        while let interface = cursor {
            cursor = interface.pointee.ifa_next

            let flags = interface.pointee.ifa_flags
            guard (flags & UInt32(IFF_UP)) != 0,
                  (flags & UInt32(IFF_LOOPBACK)) == 0,
                  let address = interface.pointee.ifa_addr else {
                continue
            }

            let family = Int32(address.pointee.sa_family)
            guard family == AF_INET || family == AF_INET6 else {
                continue
            }

            var hostBuffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            let result = getnameinfo(
                address,
                socklen_t(address.pointee.sa_len),
                &hostBuffer,
                socklen_t(hostBuffer.count),
                nil,
                0,
                NI_NUMERICHOST
            )
            guard result == 0 else { continue }

            let host = String(cString: hostBuffer)
            let normalized = host.lowercased()
            guard isPrivateHost(normalized),
                  normalized != "::1",
                  !normalized.hasPrefix("127.") else {
                continue
            }
            if family == AF_INET && !normalized.hasPrefix("169.254.") {
                return NWEndpoint.Host(host)
            }
            if fallback == nil {
                fallback = host
            }
        }

        return fallback.map { NWEndpoint.Host($0) }
    }
}

private struct IrisNearbyLanFrameAssembler {
    private static let headerSize = 13

    private let bodyLengthFromHeader: (Data) -> Int
    private var buffer = Data()

    init(bodyLengthFromHeader: @escaping (Data) -> Int) {
        self.bodyLengthFromHeader = bodyLengthFromHeader
    }

    mutating func append(_ chunk: Data) -> [Data] {
        buffer.append(chunk)
        var frames: [Data] = []
        while buffer.count >= Self.headerSize {
            let length = bodyLengthFromHeader(Data(buffer.prefix(Self.headerSize)))
            if length <= 0 {
                buffer.removeFirst()
                continue
            }
            let frameLength = Self.headerSize + length
            guard buffer.count >= frameLength else { break }
            frames.append(Data(buffer.prefix(frameLength)))
            buffer.removeFirst(frameLength)
        }
        return frames
    }
}
