#if os(iOS)
import XCTest
@testable import IrisChat

final class CallLifecycleTests: XCTestCase {
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

    private func connectedCall() -> CallSnapshot {
        CallSnapshot(outgoing: false, targetBitrateBps: 2_000_000, keyFrameGeneration: 0,
                     mediaConnected: false, maxBitrateBps: 2_000_000,
                     callId: "lifecycle-test", chatId: "peer", peerName: "Alex", phase: "connected",
                     video: false, videoCapable: false, muted: false, remoteVideo: false, remoteMuted: false,
                     startedAtSecs: 1, connectedAtSecs: 2, endReason: nil)
    }
}
#endif
