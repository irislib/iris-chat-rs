#if os(iOS)
import XCTest
@testable import IrisChat

final class CallLifecycleTests: XCTestCase {
    func testCallKitHasRequiredBackgroundModes() {
        let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String] ?? []
        XCTAssertTrue(modes.contains("voip"), "CallKit rejects transactions without the VoIP background mode")
        XCTAssertTrue(modes.contains("audio"), "Ongoing calls need background audio when the phone locks")
    }

    @MainActor
    func testCallKeepsConnectionInBackgroundAndSuspendsAfterEnd() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let active = makeAppState(rev: 1, call: connectedCall())
        let rust = MockRustApp(state: active)
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                                 dataDir: directory, environment: [:])
        manager.appBackgrounded()
        XCTAssertEqual(rust.prepareForSuspendCallCount, 0)

        let suspended = expectation(description: "suspends after the background call ends")
        rust.onNextPrepareForSuspend { suspended.fulfill() }
        var ended = connectedCall()
        ended.phase = "ended"
        rust.emit(.fullState(makeAppState(rev: 2, call: ended)))
        await fulfillment(of: [suspended], timeout: 2)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
        manager.appBackgrounded()
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)
    }

    @MainActor
    func testEndingCallWhileInactiveDoesNotSuspendForegroundApp() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp(state: makeAppState(rev: 1, call: connectedCall()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                                 dataDir: directory, environment: [:])
        manager.appInactive()
        rust.emit(.fullState(makeAppState(rev: 2)))
        await Task.yield()
        XCTAssertNil(manager.state.call)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 0)
    }

    @MainActor
    func testLateSuspendCompletionResumesAnAlreadyUnlockedApp() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 dataDir: directory, environment: [:])
        let started = expectation(description: "background flush started")
        let release = DispatchSemaphore(value: 0)
        rust.onNextPrepareForSuspend {
            started.fulfill()
            _ = release.wait(timeout: .now() + 5)
        }
        manager.appBackgrounded()
        await fulfillment(of: [started], timeout: 2)
        manager.appForegrounded()
        let resumed = expectation(description: "resume after late flush")
        rust.onDispatch = { action in
            if case .appForegrounded = action { resumed.fulfill() }
        }
        release.signal()
        await fulfillment(of: [resumed], timeout: 2)
        XCTAssertEqual(rust.dispatchedActions.filter { $0 == .appForegrounded }.count, 2)
    }

    private func connectedCall() -> CallSnapshot {
        CallSnapshot(outgoing: false, targetBitrateBps: 2_000_000, keyFrameGeneration: 0,
                     mediaConnected: false, maxBitrateBps: 2_000_000,
                     callId: "lifecycle-test", chatId: "peer", peerName: "Alex", phase: "connected",
                     video: false, videoCapable: false, muted: false, remoteVideo: false, remoteMuted: false,
                     startedAtSecs: 1, connectedAtSecs: 2, endReason: nil)
    }
}
#endif
