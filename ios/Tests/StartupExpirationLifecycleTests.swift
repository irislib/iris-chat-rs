#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class StartupExpirationLifecycleTests: XCTestCase {
    @MainActor
    func testDelayedSuspendWorkerRetainsRequestEpochAfterForeground() async throws {
        let rust = MockRustApp()
        let runner = SuspendPreparationRunner(rust: rust)
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let completed = expectation(description: "delayed worker completed")
        DispatchQueue.global(qos: .utility).async {
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
            runner.prepareForSuspend()
            completed.fulfill()
        }
        try rust.dispatch(action: .appForegrounded)
        XCTAssertEqual(rust.startupRestoreEpoch(), 1)
        gate.signal()
        await fulfillment(of: [completed], timeout: 2)
        XCTAssertEqual(rust.requestedSuspendEpochs, [0],
                       "a delayed worker cannot acquire a newer foreground cancellation token")
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
    }

    @MainActor
    func testMissingEpochUsesAcknowledgementOnlySuspend() {
        let rust = MockRustApp()
        rust.setStartupRestoreEpochAvailable(false)
        let runner = SuspendPreparationRunner(rust: rust)
        rust.setStartupRestoreEpochAvailable(true)
        runner.prepareForSuspend()
        XCTAssertEqual(rust.requestedSuspendEpochs, [nil])
        XCTAssertTrue(rust.startupRestoreCancellationEpochs.isEmpty)
    }

    @MainActor
    func testStartupExpirationBypassesBlockedBarrierAndKeepsRestorationPending() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let secretStore = InMemorySecretStore(bundle: StoredAccountBundle(
            ownerNsec: "test-owner", ownerPubkeyHex: "test-owner-id", deviceNsec: "test-device"
        ))
        let draining = expectation(description: "native barrier is waiting")
        let startupFinished = expectation(description: "startup continuation finished")
        let allowanceEnded = expectation(description: "finite startup allowance ended")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onNextPrepareForSuspend {
            draining.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        var expire: (@Sendable () -> Void)?
        var endCount = 0
        var startupCompleted = false
        let manager = AppManager(
            rust: rust, secretStore: secretStore,
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                let cancellation = IrisStartupRestoreCancellation()
                let allowance = IrisSuspendBackgroundTask(startupCancellation: cancellation, begin: {
                    expire = $0
                    return .init(rawValue: 720)
                }, end: { _ in endCount += 1; allowanceEnded.fulfill() })
                return IrisStartupBackgroundActivity(initiallyBackgrounded: true,
                    startupCancellation: cancellation) {
                    allowance.finish()
                    startupCompleted = true
                    startupFinished.fulfill()
                }
            }
        )
        manager.beginSuspendBackgroundTask = { Self.validSuspendAllowance($0) }
        await fulfillment(of: [draining], timeout: 2)
        expire?()
        XCTAssertEqual(rust.acceptedStartupRestoreCancellationEpochs, [0])
        await fulfillment(of: [allowanceEnded], timeout: 2)
        XCTAssertFalse(startupCompleted, "expiration is not the native acknowledgement")
        XCTAssertTrue(manager.bootstrapInFlight)
        XCTAssertNil(manager.state.account)
        XCTAssertNil(manager.state.toast)
        XCTAssertEqual(secretStore.load()?.deviceNsec, "test-device")
        XCTAssertEqual(rust.dispatchedActions.filter {
            if case .restoreAccountBundle = $0 { return true }; return false
        }.count, 1)
        gate.signal()
        await fulfillment(of: [startupFinished], timeout: 2)
        XCTAssertEqual(endCount, 1)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        XCTAssertTrue(manager.bootstrapInFlight, "a drain does not settle a deferred restore")
        // Rust owns the exact deferred action and retry. The shell remains
        // loading until a genuine restored-account snapshot is delivered.
        manager.appForegrounded()
        manager.apply(update: .fullState(makeAppState(rev: 1, account: makeAccount())), generation: 0)
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertNotNil(manager.state.account)
    }

    @MainActor
    func testSuspendExpirationKeepsOldEpochWhenForegroundRacesTheBarrier() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let startupFinished = expectation(description: "foreground startup finished")
        let manager = AppManager(
            rust: rust, secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                IrisStartupBackgroundActivity(initiallyBackgrounded: false) { startupFinished.fulfill() }
            }
        )
        await fulfillment(of: [startupFinished], timeout: 2)
        var expire: (@Sendable () -> Void)?
        var ends = 0
        let allowanceEnded = expectation(description: "suspend allowance expired")
        manager.beginSuspendBackgroundTask = { cancellation in
            IrisSuspendBackgroundTask(startupCancellation: cancellation, begin: {
                expire = $0
                return .init(rawValue: 721)
            }, end: { _ in ends += 1; allowanceEnded.fulfill() })
        }
        let draining = expectation(description: "suspend barrier waiting")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onNextPrepareForSuspend {
            draining.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        let waiting = manager.appBackgrounded()
        await fulfillment(of: [draining], timeout: 2)
        manager.appForegrounded()
        expire?()
        XCTAssertEqual(rust.startupRestoreCancellationEpochs, [0])
        XCTAssertTrue(rust.acceptedStartupRestoreCancellationEpochs.isEmpty)
        await fulfillment(of: [allowanceEnded], timeout: 2)
        gate.signal()
        await waiting?.value
        XCTAssertEqual(ends, 1)
        XCTAssertTrue(manager.appSceneIsActive)
        XCTAssertEqual(rust.dispatchedActions.filter { $0 == .appForegrounded }.count, 2)
    }

    @MainActor
    func testStartupExpirationBeforeClientBindingIsAppliedBeforeRestoreDispatch() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let finished = expectation(description: "early-expired startup finished")
        rust.onDispatch = { [weak rust] action in
            if case .restorePendingDeviceLink = action {
                XCTAssertEqual(rust?.acceptedStartupRestoreCancellationEpochs, [0])
            }
        }
        let manager = AppManager(
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(link: StoredPendingDeviceLink(
                deviceNsec: "test-device", approvalBootstrapJson: "{}"
            )),
            dataDir: directory, environment: [:],
            rustFactory: {
                XCTAssertTrue(rust.startupRestoreCancellationEpochs.isEmpty,
                              "the latch cannot interrupt constructor work before a client exists")
                return rust
            },
            startupActivityFactory: {
                let cancellation = IrisStartupRestoreCancellation()
                let allowance = IrisSuspendBackgroundTask(startupCancellation: cancellation, begin: {
                    $0()
                    return .init(rawValue: 722)
                }, end: { _ in })
                return IrisStartupBackgroundActivity(initiallyBackgrounded: true,
                    startupCancellation: cancellation) {
                    allowance.finish()
                    finished.fulfill()
                }
            }
        )
        manager.beginSuspendBackgroundTask = { Self.validSuspendAllowance($0) }
        await fulfillment(of: [finished], timeout: 2)
        XCTAssertEqual(rust.startupRestoreCancellationEpochs, [0])
        XCTAssertTrue(manager.bootstrapInFlight)
        XCTAssertEqual(rust.dispatchedActions.filter {
            if case .restorePendingDeviceLink = $0 { return true }; return false
        }.count, 1)
    }

    @MainActor
    func testDeniedStartupAllowanceCancelsOnlyWhenActuallyBackgrounded() async {
        // Include inactive/foreground construction that becomes backgrounded
        // before its deferred startup continuation, without a scene callback.
        for (initial, later) in [(true, true), (false, true), (false, false)] {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: directory) }
            let rust = MockRustApp()
            var applicationIsBackgrounded = initial
            let finished = expectation(description: "denied startup finishes \(initial)-\(later)")
            rust.onDispatch = { [weak rust] action in
                if case .restoreAccountBundle = action {
                    XCTAssertEqual(rust?.acceptedStartupRestoreCancellationEpochs, later ? [0] : [],
                                   "background denial must be applied before restore dispatch")
                }
            }
            let manager = AppManager(
                rust: rust,
                secretStore: InMemorySecretStore(bundle: StoredAccountBundle(
                    ownerNsec: "test-owner", ownerPubkeyHex: "test-owner-id", deviceNsec: "test-device"
                )),
                pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                dataDir: directory, environment: [:],
                startupActivityFactory: {
                    let cancellation = IrisStartupRestoreCancellation()
                    let allowance = IrisSuspendBackgroundTask(startupCancellation: cancellation,
                        begin: { _ in .invalid }, end: { _ in XCTFail("invalid identifier ended") })
                    return IrisStartupBackgroundActivity(initiallyBackgrounded: initial,
                        isBackgrounded: { applicationIsBackgrounded }, startupCancellation: cancellation,
                        isAllowanceActive: { allowance.isActive }) {
                        allowance.finish()
                        finished.fulfill()
                    }
                }
            )
            manager.beginSuspendBackgroundTask = { Self.validSuspendAllowance($0) }
            applicationIsBackgrounded = later
            await fulfillment(of: [finished], timeout: 2)
            XCTAssertEqual(rust.startupRestoreCancellationEpochs, later ? [0] : [])
            XCTAssertEqual(rust.prepareForSuspendCallCount, later ? 1 : 0)
            XCTAssertTrue(manager.bootstrapInFlight)
        }
    }

    @MainActor
    func testDeniedSuspendAllowanceCancelsStartupBeforeWorkerRuns() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let finished = expectation(description: "foreground startup finished")
        let manager = AppManager(
            rust: rust, secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                IrisStartupBackgroundActivity(initiallyBackgrounded: false) { finished.fulfill() }
            }
        )
        await fulfillment(of: [finished], timeout: 2)
        manager.beginSuspendBackgroundTask = { cancellation in
            IrisSuspendBackgroundTask(startupCancellation: cancellation,
                begin: { _ in .invalid }, end: { _ in XCTFail("invalid identifier ended") })
        }
        let waiting = manager.appBackgrounded()
        XCTAssertEqual(rust.acceptedStartupRestoreCancellationEpochs, [0])
        await waiting?.value
        XCTAssertEqual(rust.requestedSuspendEpochs, [0])
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
    }

    @MainActor
    private static func validSuspendAllowance(_ cancellation: IrisStartupRestoreCancellation) -> IrisSuspendBackgroundTask {
        IrisSuspendBackgroundTask(startupCancellation: cancellation,
            begin: { _ in .init(rawValue: 729) }, end: { _ in })
    }
}
#endif
