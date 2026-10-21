#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class BackgroundPushLifecycleTests: XCTestCase {
    @MainActor
    func testColdBackgroundStartupProtectsConstructionAndWaitsForPostRestoreBarrier() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let trace = StartupOrderTrace()
        let draining = expectation(description: "post-restore storage barrier")
        let finished = expectation(description: "startup allowance released")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onDispatch = { action in
            if case .restoreAccountBundle = action { trace.record("restore") }
        }
        rust.onNextPrepareForSuspend {
            trace.record("barrier")
            draining.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        let manager = AppManager(
            secretStore: startupSecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            rustFactory: {
                XCTAssertEqual(trace.events, ["allowance"])
                trace.record("construct")
                return rust
            },
            startupActivityFactory: {
                trace.record("allowance")
                return IrisStartupBackgroundActivity(initiallyBackgrounded: true) {
                    trace.record("finish")
                    finished.fulfill()
                }
            }
        )
        // No appBackgrounded or scene callback: UIKit's initial state is enough.
        await fulfillment(of: [draining], timeout: 2)
        XCTAssertFalse(manager.appSceneIsActive)
        XCTAssertEqual(trace.events, ["allowance", "construct", "restore", "barrier"])
        manager.apply(update: .fullState(makeAppState(rev: 1, account: makeAccount())), generation: 0)
        XCTAssertNotNil(manager.state.account)
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertFalse(trace.events.contains("finish"), "an account snapshot is not a storage acknowledgement")
        gate.signal()
        await fulfillment(of: [finished], timeout: 2)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        XCTAssertFalse(rust.dispatchedActions.contains(.appForegrounded))
    }

    @MainActor
    func testInactiveConstructionResamplesBackgroundBeforeStartupFinishesWithoutSceneCallback() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        var applicationIsBackgrounded = false
        var allowanceFinished = false
        let draining = expectation(description: "background discovered after construction")
        let finished = expectation(description: "resampled startup completes")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onNextPrepareForSuspend {
            draining.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        let manager = AppManager(
            rust: rust, secretStore: startupSecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                IrisStartupBackgroundActivity(initiallyBackgrounded: false,
                    isBackgrounded: { applicationIsBackgrounded }) {
                    allowanceFinished = true
                    finished.fulfill()
                }
            }
        )
        XCTAssertTrue(manager.appSceneIsActive)
        applicationIsBackgrounded = true
        // didFinishLaunching can reveal background after AppDelegate construction;
        // neither the scene nor applicationDidEnterBackground has called the manager.
        await fulfillment(of: [draining], timeout: 2)
        XCTAssertFalse(manager.appSceneIsActive)
        XCTAssertFalse(allowanceFinished)
        gate.signal()
        await fulfillment(of: [finished], timeout: 2)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        XCTAssertFalse(rust.dispatchedActions.contains(.appForegrounded))
    }

    @MainActor
    func testLateStartupRestoreRequiresANewBarrierAfterPriorSuspension() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let initialStartup = expectation(description: "initial foreground startup")
        let manager = AppManager(
            rust: rust, secretStore: startupSecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                IrisStartupBackgroundActivity(initiallyBackgrounded: false) { initialStartup.fulfill() }
            }
        )
        await fulfillment(of: [initialStartup], timeout: 2)
        let previousBarrier = manager.appBackgrounded()
        XCTAssertNotNil(previousBarrier)
        await previousBarrier?.value
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        rust.clearDispatchedActions()

        let trace = StartupOrderTrace()
        let nextBarrier = expectation(description: "fresh barrier after late restore")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onDispatch = { action in
            if case .restoreAccountBundle = action { trace.record("restore") }
        }
        rust.onNextPrepareForSuspend {
            trace.record("barrier")
            nextBarrier.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        let activity = IrisStartupBackgroundActivity(initiallyBackgrounded: true) { trace.record("finish") }
        // Exercise the actual deferred-startup continuation after an older flush.
        let completion = Task { await manager.completeStartup(activity) }
        await fulfillment(of: [nextBarrier], timeout: 2)
        XCTAssertEqual(trace.events, ["restore", "barrier"])
        gate.signal()
        await completion.value
        XCTAssertEqual(trace.events, ["restore", "barrier", "finish"])
        XCTAssertEqual(rust.prepareForSuspendCallCount, 2)
        XCTAssertFalse(rust.dispatchedActions.contains(.appForegrounded))
    }

    @MainActor
    func testForegroundDuringStartupBarrierResumesAfterTheLateFlush() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let draining = expectation(description: "startup storage barrier")
        let finished = expectation(description: "startup protection finished")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onNextPrepareForSuspend {
            draining.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        let manager = AppManager(
            rust: rust, secretStore: startupSecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory, environment: [:],
            startupActivityFactory: {
                IrisStartupBackgroundActivity(initiallyBackgrounded: true) { finished.fulfill() }
            }
        )
        await fulfillment(of: [draining], timeout: 2)
        manager.appForegrounded()
        gate.signal()
        await fulfillment(of: [finished], timeout: 2)
        XCTAssertTrue(manager.appSceneIsActive)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        XCTAssertEqual(rust.dispatchedActions.filter { $0 == .appForegrounded }.count, 2,
                       "unlock resumes immediately and again after a late suspension acknowledgement")
    }

    @MainActor
    func testBackgroundPushWaitsForStorageEvenAfterPreviousSuspend() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAccount()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 dataDir: directory, environment: [:])
        let initialSuspend = expectation(description: "initial suspension")
        rust.onNextPrepareForSuspend { initialSuspend.fulfill() }
        manager.appBackgrounded()
        await fulfillment(of: [initialSuspend], timeout: 2)

        let draining = expectation(description: "push storage draining")
        let gate = DispatchSemaphore(value: 0)
        rust.onNextPrepareForSuspend {
            draining.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
        }
        rust.onDispatch = { action in
            if case .ingestMobilePushPayload = action {
                rust.emit(.fullState(makeAppState(rev: 2, account: makeAccount())))
            }
        }
        var finished = false
        let handling = Task {
            _ = await manager.receiveBackgroundPush(userInfo: ["chat_id": "test"])
            finished = true
        }
        await fulfillment(of: [draining], timeout: 2)
        XCTAssertFalse(finished, "the system's background allowance must cover the final storage drain")
        gate.signal()
        await handling.value
        XCTAssertEqual(rust.prepareForSuspendCallCount, 2)
    }
    @MainActor
    func testColdBackgroundPushSuspendsWithoutASceneTransition() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAccount()))
        let startupFinished = expectation(description: "startup finished before the push")
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                                 dataDir: directory, environment: [:],
                                 startupActivityFactory: {
                                     IrisStartupBackgroundActivity(initiallyBackgrounded: false) {
                                         startupFinished.fulfill()
                                     }
                                 })
        await fulfillment(of: [startupFinished], timeout: 2)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 0)
        rust.onDispatch = { action in
            if case .ingestMobilePushPayload = action {
                rust.emit(.fullState(makeAppState(rev: 2, account: makeAccount())))
            }
        }
        _ = await manager.receiveBackgroundPush(userInfo: ["chat_id": "test"], applicationState: .background)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        XCTAssertFalse(manager.appSceneIsActive)
        XCTAssertFalse(rust.dispatchedActions.contains(.appForegrounded))
    }

    @MainActor
    func testForegroundPushDoesNotSuspendTheVisibleApp() async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAccount()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 dataDir: directory, environment: [:])
        rust.onDispatch = { action in
            if case .ingestMobilePushPayload = action {
                rust.emit(.fullState(makeAppState(rev: 2, account: makeAccount())))
            }
        }
        _ = await manager.receiveBackgroundPush(userInfo: ["chat_id": "test"], applicationState: .active)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 0)
        XCTAssertTrue(manager.appSceneIsActive)
    }

    private func startupSecretStore() -> InMemorySecretStore {
        InMemorySecretStore(bundle: StoredAccountBundle(
            ownerNsec: "test-owner", ownerPubkeyHex: "test-owner-id", deviceNsec: "test-device"
        ))
    }
}

private final class StartupOrderTrace: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String] = []
    var events: [String] { lock.withLock { values } }
    func record(_ event: String) { lock.withLock { values.append(event) } }
}
#endif
