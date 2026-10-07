#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class BackgroundPushLifecycleTests: XCTestCase {
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
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 dataDir: directory, environment: [:])
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

}
#endif
