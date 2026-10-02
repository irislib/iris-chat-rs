#if os(iOS) || os(macOS)
import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class FipsBleResponsivenessTests: XCTestCase {
    @MainActor
    func testDisableDuringSlowStartKeepsUIResponsiveAndClosesTheBridge() async throws {
        let started = expectation(description: "bridge constructor started")
        let closed = expectation(description: "disabled bridge closed")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            XCTAssertFalse(Thread.isMainThread)
            probe.record("start")
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            return BleTestSession {
                XCTAssertFalse(Thread.isMainThread)
                probe.record("close")
                closed.fulfill()
            }
        }

        lifecycle.setEnabled(true)
        await fulfillment(of: [started], timeout: 2)
        for _ in 0..<20 { lifecycle.setEnabled(true) }
        lifecycle.setEnabled(false)
        // This main-actor continuation must run while construction is blocked.
        XCTAssertNil(lifecycle.debugSnapshot())
        gate.signal()
        await fulfillment(of: [closed], timeout: 2)
        try await lifecycle.disableAndWait()
        XCTAssertNil(lifecycle.debugSnapshot())
        XCTAssertEqual(probe.events, ["start", "close"])
        await lifecycle.shutdown()
    }

    @MainActor
    func testRestartWaitsForSlowCloseWithoutBlockingTheUI() async {
        let firstStarted = expectation(description: "first bridge started")
        let closeStarted = expectation(description: "first bridge close started")
        let restarted = expectation(description: "replacement bridge started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            XCTAssertFalse(Thread.isMainThread)
            let first = probe.events.isEmpty
            probe.record(first ? "start 1" : "start 2")
            if first { firstStarted.fulfill() } else { restarted.fulfill() }
            return BleTestSession {
                XCTAssertFalse(Thread.isMainThread)
                if first {
                    closeStarted.fulfill()
                    XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
                    probe.record("close 1")
                }
            }
        }

        lifecycle.setEnabled(true)
        await fulfillment(of: [firstStarted], timeout: 2)
        lifecycle.setEnabled(false)
        await fulfillment(of: [closeStarted], timeout: 2)
        for _ in 0..<20 { lifecycle.setEnabled(true) }
        XCTAssertEqual(probe.events, ["start 1"])
        gate.signal()
        await fulfillment(of: [restarted], timeout: 2)
        XCTAssertEqual(probe.events, ["start 1", "close 1", "start 2"])
        await lifecycle.shutdown()
    }

    @MainActor
    func testLanChangeClosesOldBridgeBeforeApplyingLatestConfiguration() async {
        let firstStarted = expectation(description: "first bridge started")
        let closeStarted = expectation(description: "old bridge detach started")
        let replacementStarted = expectation(description: "replacement bridge started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            XCTAssertFalse(Thread.isMainThread)
            let number = probe.events.filter { $0.hasPrefix("start") }.count + 1
            probe.record("start \(number)")
            if number == 1 { firstStarted.fulfill() }
            if number == 2 { replacementStarted.fulfill() }
            return BleTestSession {
                XCTAssertFalse(Thread.isMainThread)
                if number == 1 {
                    closeStarted.fulfill()
                    XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
                }
                probe.record("close \(number)")
            }
        }

        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        await fulfillment(of: [firstStarted], timeout: 2)
        for _ in 0..<20 { lifecycle.setEnabled(true, nearbyLanEnabled: false) }
        await lifecycle.settle()
        XCTAssertEqual(probe.events, ["start 1"])

        lifecycle.setEnabled(true, nearbyLanEnabled: true)
        await fulfillment(of: [closeStarted], timeout: 2)
        // A rapid second toggle must not create a bridge before old detach ends.
        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        XCTAssertEqual(probe.events, ["start 1"])
        gate.signal()
        await fulfillment(of: [replacementStarted], timeout: 2)
        for _ in 0..<20 { lifecycle.setEnabled(true, nearbyLanEnabled: false) }
        await lifecycle.settle()
        XCTAssertEqual(probe.events, ["start 1", "close 1", "start 2"])
        await lifecycle.shutdown()
        XCTAssertEqual(probe.events, ["start 1", "close 1", "start 2", "close 2"])
    }

    @MainActor
    func testLanChangeDuringSlowStartReplacesStaleBridge() async {
        let firstStarted = expectation(description: "old configuration starts")
        let replacementStarted = expectation(description: "new configuration starts")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            let number = probe.events.filter { $0.hasPrefix("start") }.count + 1
            probe.record("start \(number)")
            if number == 1 {
                firstStarted.fulfill()
                XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            } else {
                replacementStarted.fulfill()
            }
            return BleTestSession { probe.record("close \(number)") }
        }

        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        await fulfillment(of: [firstStarted], timeout: 2)
        lifecycle.setEnabled(true, nearbyLanEnabled: true)
        gate.signal()
        await fulfillment(of: [replacementStarted], timeout: 2)
        await lifecycle.settle()
        XCTAssertEqual(probe.events, ["start 1", "close 1", "start 2"])
        lifecycle.setEnabled(true, nearbyLanEnabled: true)
        await lifecycle.settle()
        XCTAssertEqual(probe.events, ["start 1", "close 1", "start 2"])
        await lifecycle.shutdown()
    }

    @MainActor
    func testFailedDetachRetainsOwnerAndRetriesEvenIfLanSettingReverts() async {
        enum DetachFailure: Error { case pending }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            let number = probe.events.filter { $0.hasPrefix("start") }.count + 1
            probe.record("start \(number)")
            return BleTestSession {
                probe.record("detach \(number)")
                if number == 1, probe.events.filter({ $0 == "detach 1" }).count == 1 {
                    throw DetachFailure.pending
                }
            }
        }
        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        await lifecycle.settle()
        lifecycle.setEnabled(true, nearbyLanEnabled: true)
        await lifecycle.settle()
        XCTAssertEqual(probe.events, ["start 1", "detach 1"])
        XCTAssertNotNil(lifecycle.debugSnapshot(), "old owner remains available for teardown retry")
        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        await lifecycle.settle()
        XCTAssertEqual(probe.events, ["start 1", "detach 1", "detach 1", "start 2"])
        await lifecycle.shutdown()
    }

    @MainActor
    func testDisableWaitRejectsRetainedBridgeUntilDetachIsAcknowledged() async throws {
        enum DetachFailure: Error { case pending }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            probe.record("start")
            return BleTestSession {
                probe.record("detach")
                if probe.events.filter({ $0 == "detach" }).count == 1 {
                    throw DetachFailure.pending
                }
            }
        }
        lifecycle.setEnabled(true)
        await lifecycle.settle()
        do {
            try await lifecycle.disableAndWait()
            XCTFail("Queue settlement must not authorize a replacement while detach is pending")
        } catch IrisFipsBleLifecycle.DisableError.bridgeStillAttached {
            XCTAssertNotNil(lifecycle.debugSnapshot())
        }
        XCTAssertEqual(probe.events, ["start", "detach"])
        try await lifecycle.disableAndWait()
        XCTAssertNil(lifecycle.debugSnapshot())
        XCTAssertEqual(probe.events, ["start", "detach", "detach"])
        await lifecycle.shutdown()
    }

    @MainActor
    func testUpdateDuringFailedDetachIsRetriedWithoutAnotherStateCallback() async {
        enum DetachFailure: Error { case pending }
        let closing = expectation(description: "detach started")
        let replaced = expectation(description: "latest request applied")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            let number = probe.events.filter { $0.hasPrefix("start") }.count + 1
            probe.record("start \(number)")
            if number == 2 { replaced.fulfill() }
            return BleTestSession {
                probe.record("detach \(number)")
                if number == 1, probe.events.filter({ $0 == "detach 1" }).count == 1 {
                    closing.fulfill()
                    XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
                    throw DetachFailure.pending
                }
            }
        }
        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        await lifecycle.settle()
        lifecycle.setEnabled(true, nearbyLanEnabled: true)
        await fulfillment(of: [closing], timeout: 2)
        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        gate.signal()
        await fulfillment(of: [replaced], timeout: 2)
        XCTAssertEqual(probe.events, ["start 1", "detach 1", "detach 1", "start 2"])
        await lifecycle.shutdown()
    }

    @MainActor
    func testCoreShutdownReleasesPlatformAfterFailedDetachWithoutReplacement() async {
        enum DetachFailure: Error { case pending }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            probe.record("start")
            return BleTestSession(onStopPlatform: { probe.record("platform stopped") }) {
                probe.record("detach")
                throw DetachFailure.pending
            }
        }
        lifecycle.setEnabled(true, nearbyLanEnabled: false)
        await lifecycle.settle()
        await lifecycle.shutdown()
        XCTAssertEqual(probe.events, ["start", "detach"])
        await lifecycle.coreDidShutdown()
        lifecycle.setEnabled(true, nearbyLanEnabled: true)
        await lifecycle.settle()
        XCTAssertNil(lifecycle.debugSnapshot())
        XCTAssertEqual(probe.events, ["start", "detach", "platform stopped"])
    }

    @MainActor
    func testAbandonmentRetainsSessionUntilTerminalCoreShutdownFinishes() async {
        let shutdownStarted = expectation(description: "terminal core shutdown started")
        let platformStopped = expectation(description: "platform stopped after native shutdown")
        let sessionReleased = expectation(description: "abandoned session released")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        var lifecycle: IrisFipsBleLifecycle? = IrisFipsBleLifecycle(shutdownCore: {
            XCTAssertFalse(Thread.isMainThread)
            probe.record("shutdown started")
            shutdownStarted.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            probe.record("shutdown finished")
        }) {
            probe.record("start")
            return BleTestSession(
                onStopPlatform: {
                    probe.record("platform stopped")
                    platformStopped.fulfill()
                },
                onDeinit: {
                    probe.record("session released")
                    sessionReleased.fulfill()
                }
            ) {
                XCTFail("Final abandonment must own terminal cleanup, not an unowned detach retry")
            }
        }

        lifecycle?.setEnabled(true, nearbyLanEnabled: false)
        await lifecycle?.settle()
        lifecycle = nil
        await fulfillment(of: [shutdownStarted], timeout: 2)
        XCTAssertEqual(probe.events, ["start", "shutdown started"])
        gate.signal()
        await fulfillment(of: [platformStopped, sessionReleased], timeout: 2)
        XCTAssertEqual(probe.events, [
            "start", "shutdown started", "shutdown finished", "platform stopped", "session released",
        ])
    }

    @MainActor
    func testShutdownRejectsLaterEnableRequests() async throws {
        let started = expectation(description: "bridge started")
        let closed = expectation(description: "bridge closed for shutdown")
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle(shutdownCore: {}) {
            probe.record("start")
            started.fulfill()
            return BleTestSession { probe.record("close"); closed.fulfill() }
        }
        lifecycle.setEnabled(true)
        await fulfillment(of: [started], timeout: 2)
        await lifecycle.shutdown()
        await fulfillment(of: [closed], timeout: 2)
        lifecycle.setEnabled(true)
        try await lifecycle.disableAndWait()
        XCTAssertNil(lifecycle.debugSnapshot())
        XCTAssertEqual(probe.events, ["start", "close"])
    }
}

private final class BleTestSession: IrisFipsBleSession, @unchecked Sendable {
    private let onClose: @Sendable () throws -> Void
    private let onStopPlatform: @Sendable () -> Void
    private let onDeinit: @Sendable () -> Void
    init(
        onStopPlatform: @escaping @Sendable () -> Void = {},
        onDeinit: @escaping @Sendable () -> Void = {},
        onClose: @escaping @Sendable () throws -> Void
    ) {
        self.onStopPlatform = onStopPlatform
        self.onDeinit = onDeinit
        self.onClose = onClose
    }
    func close() throws { try onClose() }
    func stopPlatform() { onStopPlatform() }
    deinit { onDeinit() }
    func debugSnapshot() -> IrisFipsBleDebugSnapshot {
        IrisFipsBleDebugSnapshot(connectionCount: 0, bytesReceivedCount: 0, writeCompletedCount: 0)
    }
}

private final class BleLifecycleProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []
    var events: [String] { lock.withLock { recorded } }
    func record(_ event: String) { lock.withLock { recorded.append(event) } }
}
#endif
