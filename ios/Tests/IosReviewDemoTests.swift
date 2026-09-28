#if os(iOS)
import AVFoundation
import XCTest
@testable import IrisChat

@MainActor
final class IosReviewDemoTests: XCTestCase {
    func testDemoCreatesUniqueIdentityAndRealPersistentSampleMessages() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let secretStore = InMemorySecretStore()
        let manager = AppManager(secretStore: secretStore,
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                                 dataDir: directory,
                                 environment: ["IRIS_UI_TEST_RUN_ID": UUID().uuidString])
        defer { manager.logout(); try? FileManager.default.removeItem(at: directory) }
        manager.createAccount(name: "  appstoredemousermode  ")
        try await eventually { manager.state.account != nil && !IosReviewDemo.needsPreparation(in: directory) && !manager.reviewDemoPreparing }
        XCTAssertTrue(manager.isReviewDemo)
        XCTAssertFalse(manager.reviewDemoFailed)
        XCTAssertNotNil(secretStore.bundle)
        let owner = try XCTUnwrap(manager.state.account?.publicKeyHex)
        let sample = try XCTUnwrap(manager.state.chatList.first { $0.displayName == IosReviewDemo.sampleName })
        manager.dispatch(.openChat(chatId: sample.chatId))
        try await eventually { manager.state.currentChat?.chatId == sample.chatId }
        let messages = try XCTUnwrap(manager.state.currentChat?.messages)
        XCTAssertTrue(messages.contains { $0.body == IosReviewDemo.welcome && !$0.isOutgoing })
        XCTAssertTrue(messages.contains { $0.isOutgoing })
        XCTAssertTrue(messages.contains { $0.body == IosReviewDemo.twoDeviceInstructions })
        XCTAssertTrue(manager.state.chatList.contains { $0.kind == .group && $0.displayName == "Sample group" })
        let audio = try XCTUnwrap(messages.flatMap(\.attachments).first { $0.isAudio })
        let audioData = await manager.downloadAttachment(audio)
        let player = try AVAudioPlayer(data: XCTUnwrap(audioData))
        XCTAssertGreaterThan(player.duration, 5)

        manager.logout()
        try await eventually { manager.state.account == nil }
        XCTAssertFalse(manager.isReviewDemo)
        XCTAssertFalse(IosReviewDemo.isEnabled(in: directory))
        manager.createAccount(name: IosReviewDemo.username)
        try await eventually { manager.state.account != nil && !IosReviewDemo.needsPreparation(in: directory) && !manager.reviewDemoPreparing }
        XCTAssertNotEqual(manager.state.account?.publicKeyHex, owner, "Every activation must generate a fresh identity")
    }

    func testNormalNameDoesNotEnableDemo() {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), dataDir: directory, environment: [:])
        manager.createAccount(name: "Alex")
        XCTAssertFalse(manager.isReviewDemo)
        XCTAssertTrue(rust.dispatchedActions.contains(.createAccount(name: "Alex")))
    }

    private func eventually(_ ready: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(45)
        while !ready() {
            guard Date() < deadline else { XCTFail("Demo did not become ready"); throw TestError.timeout }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }
    private enum TestError: Error { case timeout }
}
#endif
