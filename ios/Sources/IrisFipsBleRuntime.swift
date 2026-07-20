#if os(iOS) || os(macOS)
import Foundation
import FipsBle

struct IrisFipsBleDebugSnapshot {
    let connectionCount: Int
    let bytesReceivedCount: Int
    let writeCompletedCount: Int
}

/// Thin Iris mapper between generated UniFFI types and the reusable FIPS Apple adapter.
final class IrisFipsBleRuntime {
    private let bridge: FfiFipsBle
    private let platform: AppleFipsBlePlatform
    private let runner: FipsBleCommandRunner
    private let commandQueue = DispatchQueue(label: "fi.siriusbusiness.irischat.fips-ble.commands")
    private let eventQueue = DispatchQueue(label: "fi.siriusbusiness.irischat.fips-ble.events")
    private let stateLock = NSLock()
    private var stopped = false
    private var connectionCount = 0
    private var bytesReceivedCount = 0
    private var writeCompletedCount = 0

    init(app: FfiApp) {
        bridge = FfiFipsBle(app: app)
        platform = AppleFipsBlePlatform()
        runner = FipsBleCommandRunner(platform: platform)
        platform.eventSink = { [weak self] event in
            self?.eventQueue.async { [weak self] in
                guard let self, !isStopped else { return }
                recordDebugEvent(event)
                let accepted = bridge.emit(event: event.rustEvent)
#if DEBUG
                NSLog("Iris FIPS BLE: %@ accepted=%@", event.debugSummary, String(accepted))
#endif
            }
        }
        commandQueue.async { [weak self] in self?.pumpCommands() }
    }

    func debugSnapshot() -> IrisFipsBleDebugSnapshot {
        stateLock.lock()
        defer { stateLock.unlock() }
        return IrisFipsBleDebugSnapshot(
            connectionCount: connectionCount,
            bytesReceivedCount: bytesReceivedCount,
            writeCompletedCount: writeCompletedCount
        )
    }

    func close() {
        stateLock.lock()
        let wasStopped = stopped
        stopped = true
        stateLock.unlock()
        if !wasStopped {
            runner.close()
            bridge.detach()
        }
    }

    private var isStopped: Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return stopped
    }

    private func recordDebugEvent(_ event: FipsBle.HostBleEvent) {
        stateLock.lock()
        defer { stateLock.unlock() }
        switch event {
        case .connected, .incomingConnection:
            connectionCount += 1
        case .bytesReceived:
            bytesReceivedCount += 1
        case .writeCompleted:
            writeCompletedCount += 1
        default:
            break
        }
    }

    private func pumpCommands() {
        while !isStopped {
            guard let command = bridge.nextCommand(timeoutMs: 1_000) else {
                // A timeout is normally one second. If Rust has closed the
                // channel, keep this command pump from becoming a busy loop.
                Thread.sleep(forTimeInterval: 0.05)
                continue
            }
            let platformCommand = command.platformCommand
#if DEBUG
            NSLog("Iris FIPS BLE: command %@", platformCommand.debugSummary)
#endif
            runner.submit(platformCommand)
        }
    }
}

private extension FipsBleCommand {
    var platformCommand: FipsBle.HostBleCommand {
        switch self {
        case let .listen(requestId, preferredPsm):
            return .listen(requestId: requestId, preferredPsm: preferredPsm)
        case .stopListening:
            return .stopListening
        case let .startAdvertising(requestId, bootstrap):
            return .startAdvertising(requestId: requestId, bootstrap: bootstrap)
        case let .stopAdvertising(requestId):
            return .stopAdvertising(requestId: requestId)
        case let .startScanning(requestId):
            return .startScanning(requestId: requestId)
        case .stopScanning:
            return .stopScanning
        case let .connect(requestId, peerToken, psm):
            return .connect(requestId: requestId, peerToken: peerToken, psm: psm)
        case let .write(requestId, connectionId, bytes):
            return .write(requestId: requestId, connectionId: connectionId, bytes: bytes)
        case let .close(connectionId):
            return .close(connectionId: connectionId)
        }
    }
}

private extension FipsBle.HostBleCommand {
    var debugSummary: String {
        switch self {
        case .listen:
            return "listen"
        case .stopListening:
            return "stop listening"
        case let .startAdvertising(_, bootstrap):
            return "start advertising bootstrap_bytes=\(bootstrap.count)"
        case .stopAdvertising:
            return "stop advertising"
        case .startScanning:
            return "start scanning"
        case .stopScanning:
            return "stop scanning"
        case .connect:
            return "connect"
        case let .write(_, connectionId, bytes):
            return "write connection=\(connectionId) bytes=\(bytes.count)"
        case let .close(connectionId):
            return "close connection=\(connectionId)"
        }
    }
}

private extension FipsBle.HostBleEvent {
    var debugSummary: String {
        switch self {
        case let .listening(_, psm):
            return "listening psm=\(psm)"
        case .advertisingStarted:
            return "advertising started"
        case .advertisingStopped:
            return "advertising stopped"
        case .scanningStarted:
            return "scanning started"
        case .peerDiscovered:
            return "peer discovered"
        case let .connected(_, _, _, sendSegmentMtu, receiveSegmentMtu):
            return "connected send_mtu=\(sendSegmentMtu) receive_mtu=\(receiveSegmentMtu)"
        case let .incomingConnection(_, _, sendSegmentMtu, receiveSegmentMtu):
            return "incoming connected send_mtu=\(sendSegmentMtu) receive_mtu=\(receiveSegmentMtu)"
        case let .bytesReceived(_, bytes):
            return "received bytes=\(bytes.count)"
        case .writeCompleted:
            return "write completed"
        case let .disconnected(_, reason):
            return "disconnected reason=\(reason ?? "none")"
        case let .failed(_, message):
            return "failed message=\(message)"
        }
    }

    var rustEvent: FipsBleEvent {
        switch self {
        case let .listening(requestId, psm):
            return .listening(requestId: requestId, psm: psm)
        case let .advertisingStarted(requestId):
            return .advertisingStarted(requestId: requestId)
        case let .advertisingStopped(requestId):
            return .advertisingStopped(requestId: requestId)
        case let .scanningStarted(requestId):
            return .scanningStarted(requestId: requestId)
        case let .peerDiscovered(peerToken, bootstrap):
            return .peerDiscovered(peerToken: peerToken, bootstrap: bootstrap)
        case let .connected(requestId, connectionId, peerToken, sendSegmentMtu, receiveSegmentMtu):
            return .connected(
                requestId: requestId,
                connectionId: connectionId,
                peerToken: peerToken,
                sendSegmentMtu: sendSegmentMtu,
                receiveSegmentMtu: receiveSegmentMtu
            )
        case let .incomingConnection(connectionId, peerToken, sendSegmentMtu, receiveSegmentMtu):
            return .incomingConnection(
                connectionId: connectionId,
                peerToken: peerToken,
                sendSegmentMtu: sendSegmentMtu,
                receiveSegmentMtu: receiveSegmentMtu
            )
        case let .bytesReceived(connectionId, bytes):
            return .bytesReceived(connectionId: connectionId, bytes: bytes)
        case let .writeCompleted(requestId):
            return .writeCompleted(requestId: requestId)
        case let .disconnected(connectionId, reason):
            return .disconnected(connectionId: connectionId, reason: reason)
        case let .failed(requestId, message):
            return .failed(requestId: requestId, message: message)
        }
    }
}
#endif
