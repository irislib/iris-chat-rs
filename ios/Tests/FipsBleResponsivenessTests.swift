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
    func testDisableDuringSlowStartKeepsUIResponsiveAndClosesTheBridge() async {
        let started = expectation(description: "bridge constructor started")
        let closed = expectation(description: "disabled bridge closed")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle {
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
        await lifecycle.disableAndWait()
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
        let lifecycle = IrisFipsBleLifecycle {
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
    func testShutdownRejectsLaterEnableRequests() async {
        let started = expectation(description: "bridge started")
        let closed = expectation(description: "bridge closed for shutdown")
        let probe = BleLifecycleProbe()
        let lifecycle = IrisFipsBleLifecycle {
            probe.record("start")
            started.fulfill()
            return BleTestSession { probe.record("close"); closed.fulfill() }
        }
        lifecycle.setEnabled(true)
        await fulfillment(of: [started], timeout: 2)
        await lifecycle.shutdown()
        await fulfillment(of: [closed], timeout: 2)
        lifecycle.setEnabled(true)
        await lifecycle.disableAndWait()
        XCTAssertNil(lifecycle.debugSnapshot())
        XCTAssertEqual(probe.events, ["start", "close"])
    }
}

private final class BleTestSession: IrisFipsBleSession, @unchecked Sendable {
    private let onClose: @Sendable () -> Void
    init(onClose: @escaping @Sendable () -> Void) { self.onClose = onClose }
    func close() { onClose() }
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
