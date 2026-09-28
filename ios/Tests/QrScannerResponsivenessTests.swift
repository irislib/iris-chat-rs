#if os(iOS)
import Foundation
import XCTest
@testable import IrisChat

final class QrScannerResponsivenessTests: XCTestCase {
    @MainActor
    func testSlowSessionStartAndStopLeaveMainActorResponsive() async {
        let started = expectation(description: "session start began")
        let stopping = expectation(description: "session stop began")
        let stopped = expectation(description: "session stop finished")
        let startGate = DispatchSemaphore(value: 0)
        let stopGate = DispatchSemaphore(value: 0)
        let firstStop = QrTestStopGuard()
        defer { startGate.signal(); stopGate.signal() }
        let lifecycle = IrisQrScannerLifecycle(
            requestAccess: { true },
            startSession: {
                XCTAssertFalse(Thread.isMainThread)
                started.fulfill()
                XCTAssertEqual(startGate.wait(timeout: .now() + 3), .success)
            },
            stopSession: {
                // The capture worker ignores repeated stops after it is no longer running.
                guard firstStop.claim() else { return }
                XCTAssertFalse(Thread.isMainThread)
                stopping.fulfill()
                XCTAssertEqual(stopGate.wait(timeout: .now() + 3), .success)
                stopped.fulfill()
            }
        )

        await lifecycle.activate()
        await fulfillment(of: [started], timeout: 2)
        lifecycle.deactivate()
        startGate.signal()
        await fulfillment(of: [stopping], timeout: 2)
        // Both main-actor continuations run before the corresponding operation unblocks.
        stopGate.signal()
        await fulfillment(of: [stopped], timeout: 2)
    }

    @MainActor
    func testPermissionGrantedAfterDismissalDoesNotStartTheSession() async throws {
        let requested = expectation(description: "camera permission requested")
        let started = DispatchSemaphore(value: 0)
        let queue = DispatchQueue(label: "to.iris.chat.qr-permission-test")
        var decision: CheckedContinuation<Bool, Never>?
        let lifecycle = IrisQrScannerLifecycle(
            queue: queue,
            requestAccess: {
                await withCheckedContinuation { continuation in
                    decision = continuation
                    requested.fulfill()
                }
            },
            startSession: { started.signal() },
            stopSession: {}
        )
        let activation = Task { await lifecycle.activate() }
        await fulfillment(of: [requested], timeout: 2)
        lifecycle.deactivate()
        try XCTUnwrap(decision).resume(returning: true)
        await activation.value
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume() }
        }

        XCTAssertFalse(lifecycle.isActive)
        XCTAssertEqual(started.wait(timeout: .now()), .timedOut)
    }
}

private final class QrTestStopGuard: @unchecked Sendable {
    private let lock = NSLock()
    private var claimed = false

    func claim() -> Bool {
        lock.withLock {
            guard !claimed else { return false }
            claimed = true
            return true
        }
    }
}
#endif
