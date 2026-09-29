import XCTest
import UserNotifications
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class NotificationTapRoutingTests: XCTestCase {
    @MainActor
    func testColdTapWaitsForAuthorizedAccountAndLatestChatWins() async {
        let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(bundle: .init(
            ownerNsec: "nsec1owner", ownerPubkeyHex: "owner", deviceNsec: "nsec1device")),
            dataDir: dataDir, environment: [:])
        manager.handleNotificationTap(chatID: "chat-first")
        manager.handleNotificationTap(chatID: "group:latest")
        XCTAssertTrue(openedChats(rust).isEmpty)
        rust.emit(.fullState(makeAppState(rev: 1, account: account(authorization: .awaitingApproval))))
        let waiting = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(waiting)
        XCTAssertTrue(openedChats(rust).isEmpty)
        rust.emit(.fullState(makeAppState(rev: 2, account: account())))
        let opened = await waitUntil { self.openedChats(rust) == ["group:latest"] }
        XCTAssertTrue(opened)
        rust.emit(.fullState(makeAppState(rev: 3, account: account())))
        let updated = await waitUntil { manager.state.rev == 3 }
        XCTAssertTrue(updated)
        XCTAssertEqual(openedChats(rust), ["group:latest"])
    }

    @MainActor
    func testWarmTapsOpenTheirOwnChatWithoutAnsweringCalls() async {
        let rust = MockRustApp(state: makeAppState(rev: 1, account: account()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), environment: [:])
        manager.handleNotificationTap(chatID: "chat-one")
        manager.handleNotificationTap(chatID: "chat-two")
        let opened = await waitUntil { self.openedChats(rust) == ["chat-one", "chat-two"] }
        XCTAssertTrue(opened)
        XCTAssertFalse(rust.dispatchedActions.contains {
            switch $0 { case .answerCall, .answerCallWithVoice: return true; default: return false }
        })
    }

    @MainActor
    func testLogoutClearsDeferredTapAndIgnoresTapsDuringReset() async {
        let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(bundle: .init(
            ownerNsec: "nsec1owner", ownerPubkeyHex: "owner", deviceNsec: "nsec1device")),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: dataDir, environment: [:])
        manager.handleNotificationTap(chatID: "old-chat")
        manager.logout()
        manager.handleNotificationTap(chatID: "during-logout")
        let reset = await waitUntil { !manager.bootstrapInFlight && rust.shutdownCallCount == 1 }
        XCTAssertTrue(reset)
        rust.clearDispatchedActions()
        rust.emit(.fullState(makeAppState(rev: 1, account: account())))
        let updated = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(updated)
        XCTAssertTrue(openedChats(rust).isEmpty)
    }

    @MainActor
    func testLoggedOutAndEmptyNotificationTargetsAreIgnored() {
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), environment: [:])
        manager.handleNotificationTap(chatID: "old-chat")
        manager.handleNotificationTap(chatID: "  ")
        XCTAssertTrue(openedChats(rust).isEmpty)
    }

    func testPostedNotificationCarriesItsOwnChatID() {
        let first = SystemDesktopNotificationPoster.content(chatID: "chat-one", title: "One", body: "First")
        let second = SystemDesktopNotificationPoster.content(chatID: "group:two", title: "Two", body: "Second")
        XCTAssertEqual(first.userInfo["chatId"] as? String, "chat-one")
        XCTAssertEqual(second.userInfo["chatId"] as? String, "group:two")
    }

    private func account(authorization: DeviceAuthorizationState = .authorized) -> AccountSnapshot {
        .init(publicKeyHex: "owner", npub: "npub-owner", displayName: "Alice", pictureUrl: nil, about: nil,
              devicePublicKeyHex: "device", deviceNpub: "npub-device", hasOwnerSigningAuthority: true,
              authorizationState: authorization)
    }

    private func openedChats(_ rust: MockRustApp) -> [String] {
        rust.dispatchedActions.compactMap { if case let .openChat(chatId) = $0 { return chatId }; return nil }
    }
}

#if os(macOS)
final class MacNotificationDelegateTests: XCTestCase {
    @MainActor
    func testColdStartQueuesLatestTapUntilManagerConfiguration() {
        var activations = 0
        var opened: [String] = []
        let delegate = MacUserNotificationDelegate { activations += 1 }
        delegate.handleResponse(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: ["chatId": "first"])
        delegate.handleResponse(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: ["chatId": "group:second"])
        XCTAssertTrue(delegate.hasPendingTap)
        XCTAssertEqual(activations, 0)
        delegate.configure { opened.append($0) }
        delegate.configure { opened.append($0) }
        XCTAssertEqual(opened, ["group:second"])
        XCTAssertEqual(activations, 1)
        XCTAssertFalse(delegate.hasPendingTap)
    }

    @MainActor
    func testWarmMessageAndCallTapsNavigateButDismissDoesNothing() {
        var activations = 0
        var opened: [String] = []
        let delegate = MacUserNotificationDelegate { activations += 1 }
        delegate.configure { opened.append($0) }
        delegate.handleResponse(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: ["chatId": "message"])
        delegate.handleResponse(actionIdentifier: UNNotificationDefaultActionIdentifier,
                                userInfo: ["chatId": "caller", "callId": "call"])
        delegate.handleResponse(actionIdentifier: UNNotificationDismissActionIdentifier, userInfo: ["chatId": "dismissed"])
        XCTAssertEqual(opened, ["message", "caller"])
        XCTAssertEqual(activations, 2)
    }

    @MainActor
    func testOldNotificationWithoutTargetOnlyActivatesWindow() {
        var activations = 0
        let delegate = MacUserNotificationDelegate { activations += 1 }
        delegate.configure { _ in XCTFail("No chat target") }
        delegate.handleResponse(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: [:])
        XCTAssertEqual(activations, 1)
    }
}
#endif
