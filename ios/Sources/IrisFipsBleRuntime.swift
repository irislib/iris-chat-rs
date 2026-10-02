#if os(iOS) || os(macOS)
import Foundation
import FipsBle

struct IrisFipsBleDebugSnapshot {
    let connectionCount: Int
    let bytesReceivedCount: Int
    let writeCompletedCount: Int
}

protocol IrisFipsBleSession: AnyObject, Sendable {
    func close() throws
    func stopPlatform()
    func debugSnapshot() -> IrisFipsBleDebugSnapshot
}

extension IrisFipsBleSession {
    func stopPlatform() {}
}

/// Only desired state is changed by UI callbacks. Blocking bridge work runs on
/// one worker, which rechecks that state after every constructor/close returns.
/// LAN changes need a new endpoint; detach its single-use BLE bridge before reattaching.
final class IrisFipsBleLifecycle: @unchecked Sendable {
    enum DisableError: Error { case bridgeStillAttached }

    private let queue = DispatchQueue(label: "fi.siriusbusiness.irischat.fips-ble.lifecycle")
    private let lock = NSLock()
    private let shutdownCore: @Sendable () -> Void
    private let makeSession: @Sendable () -> IrisFipsBleSession?
    private var desiredEnabled = false
    private var desiredNearbyLanEnabled = false
    private var sessionNearbyLanEnabled = false
    private var sessionNeedsDetach = false
    private var terminated = false
    private var scheduled = false
    private var requestGeneration: UInt64 = 0
    private var session: IrisFipsBleSession?

    init(
        shutdownCore: @escaping @Sendable () -> Void,
        makeSession: @escaping @Sendable () -> IrisFipsBleSession?
    ) {
        self.shutdownCore = shutdownCore
        self.makeSession = makeSession
    }

    func setEnabled(_ enabled: Bool, nearbyLanEnabled: Bool? = nil) {
        lock.withLock {
            guard !terminated else { return }
            requestGeneration &+= 1
            desiredEnabled = enabled
            if let nearbyLanEnabled { desiredNearbyLanEnabled = nearbyLanEnabled }
            scheduleLocked()
        }
    }

    func disableAndWait() async throws {
        setEnabled(false)
        await settle()
        guard lock.withLock({ session == nil && !desiredEnabled }) else {
            throw DisableError.bridgeStillAttached
        }
    }

    func shutdown() async {
        lock.withLock {
            requestGeneration &+= 1
            terminated = true
            desiredEnabled = false
            scheduleLocked()
        }
        await settle()
    }

    func coreDidShutdown() async {
        await withCheckedContinuation { continuation in
            queue.async {
                let previous = self.lock.withLock { () -> IrisFipsBleSession? in
                    guard self.terminated else { return nil }
                    defer { self.session = nil }
                    return self.session
                }
                previous?.stopPlatform()
                continuation.resume()
            }
        }
    }

    func debugSnapshot() -> IrisFipsBleDebugSnapshot? {
        let current = lock.withLock { session }
        return current?.debugSnapshot()
    }

    private func scheduleLocked() {
        guard !scheduled else { return }
        scheduled = true
        queue.async { self.reconcile() }
    }

    func settle() async {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume() }
        }
    }

    private func reconcile() {
        while true {
            lock.lock()
            let generation = requestGeneration
            if (!desiredEnabled || sessionNeedsDetach || desiredNearbyLanEnabled != sessionNearbyLanEnabled),
               let previous = session {
                sessionNeedsDetach = true
                lock.unlock()
                do {
                    try previous.close()
                } catch {
                    // Keep the old owner until its native detach is acknowledged.
                    // A later update can retry; never attach a replacement early.
                    NSLog("Iris FIPS BLE could not detach: %@", "\(error)")
                    lock.lock()
                    if generation != requestGeneration {
                        lock.unlock()
                        continue
                    }
                    scheduled = false
                    lock.unlock()
                    return
                }
                lock.withLock {
                    session = nil
                    sessionNeedsDetach = false
                }
            } else if desiredEnabled, session == nil {
                let nearbyLanEnabled = desiredNearbyLanEnabled
                lock.unlock()
                let created = makeSession()
                lock.lock()
                session = created
                sessionNearbyLanEnabled = nearbyLanEnabled
                if created == nil {
                    // Preserve requests received while construction was blocked.
                    if generation != requestGeneration {
                        lock.unlock()
                        continue
                    }
                    // With no new request, wait for a later state update to retry.
                    scheduled = false
                    lock.unlock()
                    return
                }
                lock.unlock()
            } else {
                scheduled = false
                lock.unlock()
                return
            }
        }
    }

    deinit {
        if let session {
            let shutdownCore = shutdownCore
            // The lifecycle can no longer own a retry after deallocation.
            // Retain the session/pump through terminal native shutdown instead.
            queue.async {
                shutdownCore()
                session.stopPlatform()
            }
        }
    }
}

/// Thin Iris mapper between generated UniFFI types and the reusable FIPS Apple adapter.
final class IrisFipsBleRuntime: IrisFipsBleSession, @unchecked Sendable {
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
    // Explicit diagnostics for signed development/release builds; no payloads.
    private let traceEnabled: Bool = {
#if DEBUG
        return true
#else
        return ProcessInfo.processInfo.environment["IRIS_FIPS_BLE_TRACE"] == "1"
#endif
    }()

    init?(app: FfiApp) {
        do {
            bridge = try FfiFipsBle(app: app)
        } catch {
            NSLog("Iris FIPS BLE could not start: %@", "\(error)")
            return nil
        }
        platform = AppleFipsBlePlatform()
        runner = FipsBleCommandRunner(platform: platform)
        platform.eventSink = { [weak self] event in
            self?.eventQueue.async { [weak self] in
                guard let self, !isStopped else { return }
                recordDebugEvent(event)
                let accepted = bridge.emit(event: event.rustEvent)
                if traceEnabled {
                    NSLog("Iris FIPS BLE: %@ accepted=%@", event.debugSummary, String(accepted))
                }
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

    func close() throws {
        guard !isStopped else { return }
        // FIPS shutdown requests advertising/scan cleanup through this pump.
        // Keep it and the event sink alive until the core acknowledges detach.
        try bridge.detach()
        stopPlatform()
    }

    func stopPlatform() {
        let wasStopped = stateLock.withLock {
            let previous = stopped
            stopped = true
            return previous
        }
        if !wasStopped { runner.close() }
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
            if traceEnabled {
                NSLog("Iris FIPS BLE: command %@", platformCommand.debugSummary)
            }
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
