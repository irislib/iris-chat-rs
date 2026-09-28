#if os(iOS)
import Foundation

/// Connects the demo's two real messaging engines inside this process. It uses
/// the existing host-transport boundary; it does not advertise over Bluetooth.
final class IosDemoLocalLink: @unchecked Sendable {
    private let bridges: [FfiFipsBle]
    private let queue = DispatchQueue(label: "iris.demo.link")
    private let lock = NSLock()
    private var stopped = false
    private var scanning: Set<Int> = []
    private var advertisements: [Int: Data] = [:]
    private var connections: Set<UInt64> = []
    private var nextConnection: UInt64 = 1

    init(first: FfiApp, second: FfiApp) throws {
        let firstBridge = try FfiFipsBle(app: first)
        do { bridges = [firstBridge, try FfiFipsBle(app: second)] }
        catch { firstBridge.detach(); throw error }
        for side in 0...1 {
            DispatchQueue(label: "iris.demo.link.reader.\(side)").async { [weak self] in
                self?.readCommands(side: side)
            }
        }
    }

    func close() {
        lock.lock()
        let alreadyStopped = stopped
        stopped = true
        lock.unlock()
        guard !alreadyStopped else { return }
        queue.sync { bridges.forEach { $0.detach() } }
    }

    private var isStopped: Bool {
        lock.lock()
        defer { lock.unlock() }
        return stopped
    }

    private func readCommands(side: Int) {
        while !isStopped {
            guard let command = bridges[side].nextCommand(timeoutMs: 1_000) else {
                Thread.sleep(forTimeInterval: 0.05)
                continue
            }
            queue.async { [weak self] in
                guard let self, !self.isStopped else { return }
                self.handle(command, side: side)
            }
        }
    }

    private func discover(_ side: Int) {
        guard scanning.contains(side), let bytes = advertisements[1 - side] else { return }
        _ = bridges[side].emit(event: .peerDiscovered(peerToken: "demo-\(1 - side)", bootstrap: bytes))
    }

    private func handle(_ command: FipsBleCommand, side: Int) {
        let local = bridges[side]
        let remote = bridges[1 - side]
        switch command {
        case let .listen(id, psm):
            _ = local.emit(event: .listening(requestId: id, psm: psm == 0 ? 129 : psm))
        case .stopListening:
            break
        case let .startAdvertising(id, bytes):
            advertisements[side] = bytes
            _ = local.emit(event: .advertisingStarted(requestId: id))
            discover(1 - side)
        case let .stopAdvertising(id):
            advertisements.removeValue(forKey: side)
            _ = local.emit(event: .advertisingStopped(requestId: id))
        case let .startScanning(id):
            scanning.insert(side)
            _ = local.emit(event: .scanningStarted(requestId: id))
            discover(side)
        case .stopScanning:
            scanning.remove(side)
        case let .connect(id, token, _):
            guard token == "demo-\(1 - side)", advertisements[1 - side] != nil else {
                _ = local.emit(event: .failed(requestId: id, message: "Demo peer is unavailable"))
                return
            }
            let connection = nextConnection
            nextConnection &+= 1
            connections.insert(connection)
            _ = remote.emit(event: .incomingConnection(connectionId: connection, peerToken: "demo-\(side)", sendSegmentMtu: 4096, receiveSegmentMtu: 4096))
            _ = local.emit(event: .connected(requestId: id, connectionId: connection, peerToken: token, sendSegmentMtu: 4096, receiveSegmentMtu: 4096))
        case let .write(id, connection, bytes):
            guard connections.contains(connection) else {
                _ = local.emit(event: .failed(requestId: id, message: "Demo connection closed"))
                return
            }
            _ = remote.emit(event: .bytesReceived(connectionId: connection, bytes: bytes))
            _ = local.emit(event: .writeCompleted(requestId: id))
        case let .close(connection):
            guard connections.remove(connection) != nil else { return }
            for bridge in bridges {
                _ = bridge.emit(event: .disconnected(connectionId: connection, reason: "Demo connection closed"))
            }
        }
    }
}
#endif
