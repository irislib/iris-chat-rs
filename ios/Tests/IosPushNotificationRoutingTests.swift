import XCTest

#if os(iOS)
@testable import IrisChat

final class IosPushNotificationRoutingTests: XCTestCase {
    @MainActor
    func testPushTapBeforeRestoreWaitsForAuthorizationAndOpensOnlyOnce() async throws {
        let dataDir = makeDataDir()
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(bundle: makeStoredAccountBundle()),
            dataDir: dataDir,
            environment: [:]
        )

        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "chat-restored"))

        XCTAssertFalse(rust.dispatchedActions.contains { action in
            if case .restoreAccountBundle = action { return true }
            return false
        }, "tap must be captured before the asynchronous restore starts")
        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 1)
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)

        let restoreStarted = await waitUntil { rust.dispatchedActions.contains { action in
            if case .restoreAccountBundle = action { return true }
            return false
        } }
        XCTAssertTrue(restoreStarted)
        var restoringState = makeAppState(rev: 1)
        restoringState.busy.restoringSession = true
        rust.emit(.fullState(restoringState))
        let appliedRestoringState = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(appliedRestoringState)
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)

        rust.emit(.fullState(makeAppState(rev: 2, account: makeAuthorizedAccount())))
        let appliedAuthorizedState = await waitUntil { manager.state.rev == 2 }
        XCTAssertTrue(appliedAuthorizedState)
        let openedAfterRestore = await waitUntil {
            self.openedChatIDs(in: rust.dispatchedActions) == ["chat-restored"]
        }
        XCTAssertTrue(openedAfterRestore)

        rust.emit(.fullState(makeAppState(rev: 3, account: makeAuthorizedAccount())))
        let appliedFollowUpState = await waitUntil { manager.state.rev == 3 }
        XCTAssertTrue(appliedFollowUpState)
        XCTAssertEqual(openedChatIDs(in: rust.dispatchedActions), ["chat-restored"])
    }

    @MainActor
    func testLatestPushTapWinsWhileEveryPayloadIsIngested() async throws {
        let dataDir = makeDataDir()
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(bundle: makeStoredAccountBundle()),
            dataDir: dataDir,
            environment: [:]
        )

        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "chat-first"))
        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "chat-latest"))

        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 2)
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)

        rust.emit(.fullState(makeAppState(rev: 1, account: makeAuthorizedAccount())))
        let appliedAuthorizedState = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(appliedAuthorizedState)
        let openedAfterRestore = await waitUntil {
            self.openedChatIDs(in: rust.dispatchedActions) == ["chat-latest"]
        }
        XCTAssertTrue(openedAfterRestore)
    }

    @MainActor
    func testAuthorizedPushTapIngestsThenOpensImmediately() async throws {
        let dataDir = makeDataDir()
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAuthorizedAccount()))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        rust.clearDispatchedActions()

        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "chat-warm"))

        let openedImmediately = await waitUntil {
            self.openedChatIDs(in: rust.dispatchedActions) == ["chat-warm"]
        }
        XCTAssertTrue(openedImmediately)
        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 1)
        let relevantActions = rust.dispatchedActions.filter { action in
            switch action {
            case .ingestMobilePushPayload, .openChat:
                return true
            default:
                return false
            }
        }
        XCTAssertEqual(relevantActions.count, 2)
        if case .ingestMobilePushPayload = relevantActions[0] {} else {
            XCTFail("push payload must be ingested before navigation")
        }
    }

    @MainActor
    func testLogoutClearsPendingPushNavigation() async throws {
        let dataDir = makeDataDir()
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState())
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(bundle: makeStoredAccountBundle()),
            dataDir: dataDir,
            environment: [:]
        )

        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "chat-before-logout"))
        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 1)
        manager.logout()
        rust.clearDispatchedActions()

        rust.emit(.fullState(makeAppState(rev: 1, account: makeAuthorizedAccount())))
        let appliedPostLogoutState = await waitUntil { manager.state.rev == 1 }
        XCTAssertTrue(appliedPostLogoutState)
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)
    }

    private func makeDataDir() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
    }

    private func makeStoredAccountBundle() -> StoredAccountBundle {
        StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner",
            deviceNsec: "nsec1device"
        )
    }

    private func makeAuthorizedAccount() -> AccountSnapshot {
        AccountSnapshot(
            publicKeyHex: "owner",
            npub: "npub-owner",
            displayName: "Alice",
            pictureUrl: nil,
            about: nil,
            devicePublicKeyHex: "device",
            deviceNpub: "npub-device",
            hasOwnerSigningAuthority: true,
            authorizationState: .authorized
        )
    }

    private func pushPayload(chatID: String) -> [AnyHashable: Any] {
        ["chat_id": chatID, "title": "Bob", "body": "hello"]
    }

    private func pushIngestCount(in actions: [AppAction]) -> Int {
        actions.filter { action in
            if case .ingestMobilePushPayload = action { return true }
            return false
        }.count
    }

    private func openedChatIDs(in actions: [AppAction]) -> [String] {
        actions.compactMap { action in
            if case let .openChat(chatId) = action { return chatId }
            return nil
        }
    }
}
#endif
