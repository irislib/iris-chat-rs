import XCTest

#if os(iOS)
@testable import IrisChat

final class IosPushNotificationRoutingTests: XCTestCase {
    @MainActor
    func testEncryptedPushCacheMissLeavesMainActorResponsive() async throws {
        let dataDir = makeDataDir()
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(rust: MockRustApp(state: makeAppState(rev: 1, account: makeAuthorizedAccount())),
            secretStore: InMemorySecretStore(bundle: makeStoredAccountBundle()), dataDir: dataDir, environment: [:])
        // A parseable envelope with unavailable local keys takes the real
        // preview-cache retry path, which previously slept on the main thread.
        let payload: [AnyHashable: Any] = ["event": [
            "id": String(repeating: "0", count: 64),
            "pubkey": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "sig": String(repeating: "0", count: 128), "created_at": 1,
            "kind": 1060, "tags": [["header", "unavailable"]], "content": "unavailable"
        ]]
        let start = Date()
        let handling = Task { await manager.foregroundPushPresentationOptions(userInfo: payload) }
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertLessThan(Date().timeIntervalSince(start), 1, "notification cache retries must not freeze the UI")
        let options = await handling.value
        XCTAssertTrue(options.isEmpty)
        XCTAssertGreaterThan(Date().timeIntervalSince(start), 4, "exercise the actual cache retry path")
    }

    @MainActor
    func testSlowNotificationDecryptionDoesNotBlockUIOrDelayIngestion() async {
        let started = expectation(description: "preview worker started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let resolver = MobilePushNotificationResolver { _, _, payload in
            XCTAssertFalse(Thread.isMainThread)
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            return resolveMobilePushNotificationPayload(rawPayloadJson: payload)
        }
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAuthorizedAccount()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), environment: [:], pushNotificationResolver: resolver)
        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "slow-chat"))
        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 1)
        await fulfillment(of: [started], timeout: 2)
        // This resumes on the UI executor while the preview is still blocked.
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)
        manager.recordUserActivity()
        gate.signal()
        let opened = await waitUntil { self.openedChatIDs(in: rust.dispatchedActions) == ["slow-chat"] }
        XCTAssertTrue(opened)
    }

    @MainActor
    func testLogoutDiscardsNotificationDecryptionAlreadyInFlight() async {
        let started = expectation(description: "preview worker started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let resolver = MobilePushNotificationResolver { _, _, payload in
            if payload != "{}" {
                started.fulfill()
                XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            }
            return resolveMobilePushNotificationPayload(rawPayloadJson: payload)
        }
        let dataDir = makeDataDir()
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAuthorizedAccount()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: dataDir, environment: [:], pushNotificationResolver: resolver)
        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "old-chat"))
        await fulfillment(of: [started], timeout: 2)
        manager.logout()
        XCTAssertTrue(manager.bootstrapInFlight)
        gate.signal()
        let reset = await waitUntil { !manager.bootstrapInFlight && rust.shutdownCallCount == 1 }
        XCTAssertTrue(reset)
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)
    }

    @MainActor
    func testNotificationFromPreviousAccountDoesNotIngestOrNavigate() {
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAuthorizedAccount()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), environment: [:])
        manager.handlePushNotificationTap(userInfo: ["chat_id": "old-chat", "iris_account_id": "old-owner", "body": "Old message"])
        XCTAssertTrue(openedChatIDs(in: rust.dispatchedActions).isEmpty)
        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 0)
    }

    @MainActor
    func testGroupPushTapPrefersGroupOverSenderButExplicitChatWins() async {
        let rust = MockRustApp(state: makeAppState(rev: 1, account: makeAuthorizedAccount()))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), environment: [:])
        manager.handlePushNotificationTap(userInfo: ["group_id": "team", "sender_pubkey": "sender", "body": "Hello"])
        let groupOpened = await waitUntil { self.openedChatIDs(in: rust.dispatchedActions) == ["group:team"] }
        XCTAssertTrue(groupOpened)
        manager.handlePushNotificationTap(userInfo: ["chat_id": "explicit", "group_id": "team", "sender_pubkey": "sender"])
        let explicitOpened = await waitUntil { self.openedChatIDs(in: rust.dispatchedActions) == ["group:team", "explicit"] }
        XCTAssertTrue(explicitOpened)
    }

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
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: dataDir,
            environment: [:]
        )

        manager.handlePushNotificationTap(userInfo: pushPayload(chatID: "chat-before-logout"))
        XCTAssertEqual(pushIngestCount(in: rust.dispatchedActions), 1)
        manager.logout()
        let resetCompleted = await waitUntil { !manager.bootstrapInFlight && rust.shutdownCallCount == 1 }
        XCTAssertTrue(resetCompleted)
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
