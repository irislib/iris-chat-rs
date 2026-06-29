import Foundation
import Network

final class IrisNearbyLanService: NSObject, NetServiceDelegate {
    private static let serviceType = "_iris-chat._tcp"
    private static let netServiceType = "_iris-chat._tcp."

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
    private let queue = DispatchQueue(label: "fi.siriusbusiness.irischat.nearby.lan")
    private let bodyLengthFromHeader: (Data) -> Int
    private let onFrame: (String, Data) -> Void
    private let onStatus: (String) -> Void

    private var listener: NWListener?
    private var browser: NWBrowser?
    private var netService: NetService?
    private var connections: [String: ConnectionSlot] = [:]
    private var endpointIDs: Set<String> = []
    private var enabled = false

    // Cold-start retry window: NWListener / NWBrowser / NetService.publish
    // can each transiently fire .failed during the first second or two
    // after launch (network stack still warming up, local-network
    // permission resolving, …). Reporting "Local network failed"
    // immediately is what made users see "Wi-Fi failed" on every cold
    // start and need to toggle Nearby off+on to recover. Retry silently
    // up to `maxSettleRetries` times within `settleWindow` before
    // surfacing a generic failure.
    private static let settleWindow: DispatchTimeInterval = .seconds(6)
    private static let settleRetryDelay: DispatchTimeInterval = .milliseconds(750)
    private static let maxSettleRetries: Int = 4
    private static let retryableFailureStatuses: Set<String> = [
        "Local network failed",
        "Local network unavailable"
    ]
    private var settleDeadline: DispatchTime?
    private var settleRetries: Int = 0
    private var startGeneration: UInt64 = 0
    private var lastStatus = "Off"
    private var servicePublished = false
    private var browserReady = false

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
        super.init()
    }

    func start() {
        queue.async { [weak self] in
            guard let self else { return }
            if self.enabled {
                if Self.retryableFailureStatuses.contains(self.lastStatus) {
                    self.restartOnQueue()
                }
                return
            }
            self.enabled = true
            self.startGeneration &+= 1
            self.startOnQueue(generation: self.startGeneration)
        }
    }

    func restart() {
        queue.async { [weak self] in
            self?.restartOnQueue()
        }
    }

    private func restartOnQueue() {
        enabled = true
        startGeneration &+= 1
        cancelRuntime()
        startOnQueue(generation: startGeneration)
    }

    private func startOnQueue(generation: UInt64) {
        settleDeadline = .now() + Self.settleWindow
        settleRetries = 0
        servicePublished = false
        browserReady = false
        updateStatus("Starting")
        startListener(generation: generation)
        startBrowser(generation: generation)
    }

    private func markStartupStableIfReady() {
        guard servicePublished, browserReady else { return }
        settleDeadline = nil
        settleRetries = 0
    }

    /// Report a failure status, or — if we're still inside the cold-start
    /// settle window — tear the listener/browser down and reattempt
    /// silently. Permission errors (`No local network access`) bypass
    /// the retry path; they need user action, not patience.
    private func reportFailureOrRetry(_ status: String, generation: UInt64) {
        guard enabled, generation == startGeneration else { return }
        let isPermissionDenied = status == "No local network access"
        if !isPermissionDenied,
           let deadline = settleDeadline,
           DispatchTime.now() < deadline,
           settleRetries < Self.maxSettleRetries
        {
            settleRetries += 1
            startGeneration &+= 1
            let retryGeneration = startGeneration
            cancelRuntime()
            updateStatus("Starting")
            queue.asyncAfter(deadline: .now() + Self.settleRetryDelay) { [weak self] in
                guard let self, self.enabled, self.startGeneration == retryGeneration else { return }
                self.startListener(generation: retryGeneration)
                self.startBrowser(generation: retryGeneration)
            }
            return
        }
        settleDeadline = nil
        updateStatus(status)
    }

    func stop() {
        queue.async { [weak self] in
            guard let self else { return }
            self.enabled = false
            self.startGeneration &+= 1
            self.settleDeadline = nil
            self.settleRetries = 0
            self.cancelRuntime()
            self.updateStatus("Off")
        }
    }

    private func cancelRuntime() {
        listener?.cancel()
        browser?.cancel()
        let service = netService
        listener = nil
        browser = nil
        netService = nil
        servicePublished = false
        browserReady = false
        endpointIDs.removeAll()
        for slot in connections.values {
            slot.connection.cancel()
        }
        connections.removeAll()
        DispatchQueue.main.async {
            service?.stop()
        }
    }

    func send(_ frame: Data, excludingPeerID: String?, onlyPeerID: String? = nil) {
        queue.async { [weak self] in
            guard let self, self.enabled else { return }
            for slot in self.connections.values {
                if let excludingPeerID, slot.peerID == excludingPeerID {
                    continue
                }
                if let onlyPeerID, slot.peerID != onlyPeerID {
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

    func peerIDForConnection(_ connectionID: String) -> String? {
        queue.sync {
            connections[connectionID]?.peerID
        }
    }

    func peerIDs() -> Set<String> {
        queue.sync {
            Set(connections.values.compactMap(\.peerID))
        }
    }

    private func startListener(generation: UInt64) {
        do {
            let listener = try NWListener(using: tcpParameters(), on: .any)
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
                guard self.enabled, generation == self.startGeneration else { return }
                switch state {
                case .ready:
                    guard let port = listener.port else {
                        self.reportFailureOrRetry("Local network failed", generation: generation)
                        return
                    }
                    self.publishService(port: port.rawValue, generation: generation)
                    self.updateStatus(self.connections.isEmpty ? "Visible" : "Connected")
                case .failed(let error):
                    self.reportFailureOrRetry(Self.failureStatus(for: error), generation: generation)
                case .cancelled:
                    break
                default:
                    break
                }
            }
            listener.start(queue: queue)
            self.listener = listener
        } catch {
            reportFailureOrRetry(Self.failureStatus(for: error, fallback: "Local network unavailable"), generation: generation)
        }
    }

    private func publishService(port: UInt16, generation: UInt64) {
        let previous = netService
        let service = NetService(
            domain: "local.",
            type: Self.netServiceType,
            name: peerID,
            port: Int32(port)
        )
        service.includesPeerToPeer = false
        service.delegate = self
        service.schedule(in: .main, forMode: .common)
        guard generation == startGeneration else { return }
        netService = service
        DispatchQueue.main.async {
            previous?.stop()
            service.publish()
        }
    }

    func netServiceDidPublish(_ sender: NetService) {
        irisDebugLog("Iris nearby LAN: published \(sender.name).\(sender.type) port \(sender.port)")
        queue.async { [weak self] in
            guard let self, self.enabled, sender === self.netService else { return }
            self.servicePublished = true
            self.markStartupStableIfReady()
        }
    }

    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        irisDebugLog("Iris nearby LAN: publish failed \(errorDict)")
        queue.async { [weak self] in
            guard let self, sender === self.netService else { return }
            self.reportFailureOrRetry("Local network failed", generation: self.startGeneration)
        }
    }

    private func startBrowser(generation: UInt64) {
        let browser = NWBrowser(for: .bonjour(type: Self.serviceType, domain: nil), using: tcpParameters())
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            guard let self, self.enabled, generation == self.startGeneration else { return }
            for result in results {
                self.connectIfNeeded(to: result.endpoint)
            }
        }
        browser.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            guard self.enabled, generation == self.startGeneration else { return }
            switch state {
            case .ready:
                self.browserReady = true
                self.markStartupStableIfReady()
            case .failed(let error):
                self.reportFailureOrRetry(Self.failureStatus(for: error), generation: generation)
            default:
                break
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
        let connection = NWConnection(to: endpoint, using: tcpParameters())
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
        lastStatus = status
        DispatchQueue.main.async {
            self.onStatus(status)
        }
    }

    private static func failureStatus(for error: Error, fallback: String = "Local network failed") -> String {
        isLocalNetworkPermissionError(error) ? "No local network access" : fallback
    }

    private static func isLocalNetworkPermissionError(_ error: Error) -> Bool {
        if let nwError = error as? NWError {
            switch nwError {
            case .posix(let code):
                if code == .EACCES || code == .EPERM {
                    return true
                }
            default:
                break
            }
        }
        if let posixError = error as? POSIXError,
           posixError.code == .EACCES || posixError.code == .EPERM {
            return true
        }
        let text = "\(String(describing: error)) \(error.localizedDescription)"
        return text.localizedCaseInsensitiveContains("PolicyDenied") ||
            text.localizedCaseInsensitiveContains("policy denied") ||
            text.contains("-65570")
    }

    private func tcpParameters() -> NWParameters {
        let parameters = NWParameters.tcp
        parameters.includePeerToPeer = false
        parameters.prohibitedInterfaceTypes = [.cellular]
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
