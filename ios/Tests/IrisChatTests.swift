import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

private final class InMemorySecretStore: AccountSecretStore {
    var bundle: StoredAccountBundle?

    init(bundle: StoredAccountBundle? = nil) {
        self.bundle = bundle
    }

    func load() -> StoredAccountBundle? {
        bundle
    }

    func save(_ bundle: StoredAccountBundle) {
        self.bundle = bundle
    }

    func clear() {
        bundle = nil
    }
}

private final class MockDesktopNotificationPoster: DesktopNotificationPosting {
    var posts: [(title: String, body: String)] = []

    func post(title: String, body: String) {
        posts.append((title: title, body: body))
    }
}

private final class MockRustApp: RustAppClient {
    var currentState: AppState
    var dispatchedActions: [AppAction] = []
    var supportBundleJson = "{\"ok\":true}"
    var peerDebug: PeerProfileDebugSnapshot?
    var dispatchError: Error?
    var onDispatch: ((AppAction) -> Void)?
    private var prepareForSuspendCalls = 0
    private let prepareForSuspendLock = NSLock()
    private var reconciler: AppReconciler?

    var prepareForSuspendCallCount: Int {
        prepareForSuspendLock.lock()
        defer { prepareForSuspendLock.unlock() }
        return prepareForSuspendCalls
    }

    init(state: AppState = AppState(
        rev: 0,
        router: Router(defaultScreen: .welcome, screenStack: []),
        account: nil,
        deviceRoster: nil,
        busy: BusyState(
            creatingAccount: false,
            restoringSession: false,
            linkingDevice: false,
            creatingChat: false,
            creatingGroup: false,
            sendingMessage: false,
            updatingRoster: false,
            updatingGroup: false,
            creatingInvite: false,
            acceptingInvite: false,
            syncingNetwork: false,
            uploadingAttachment: false
        ),
        chatList: [],
        currentChat: nil,
        groupDetails: nil,
        publicInvite: nil,
        linkDevice: nil,
        networkStatus: nil,
        mobilePush: MobilePushSyncSnapshot(
            ownerPubkeyHex: nil,
            messageAuthorPubkeys: [],
            inviteResponsePubkeys: [],
            sessions: []
        ),
        preferences: PreferencesSnapshot(
            sendTypingIndicators: true,
            sendReadReceipts: true,
            desktopNotificationsEnabled: true,
            inviteAcceptanceNotificationsEnabled: true,
            startupAtLoginEnabled: false,
            nearbyBluetoothEnabled: false,
            nearbyLanEnabled: false,
            nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
            imageProxyEnabled: true,
            imageProxyUrl: "https://imgproxy.iris.to",
            imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
            imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
            mutedChatIds: [],
            pinnedChatIds: [],
            mobilePushServerUrl: ""
        ),
        toast: nil
    )) {
        self.currentState = state
    }

    func state() -> AppState {
        currentState
    }

    func dispatch(action: AppAction) throws {
        if let dispatchError {
            throw dispatchError
        }
        dispatchedActions.append(action)
        onDispatch?(action)
    }

    func search(query: String, scopeChatId: String?, limit: UInt32) -> SearchResultSnapshot {
        SearchResultSnapshot(
            query: query,
            scopeChatId: scopeChatId,
            contacts: [],
            groups: [],
            messages: [],
            shortcut: nil
        )
    }

    func ingestNearbyEventJson(eventJson: String) -> Bool {
        true
    }

    func ingestNearbyEventJsonWithTransport(eventJson: String, transport: String) -> Bool {
        true
    }

    func buildNearbyPresenceEventJson(peerID: String, myNonce: String, theirNonce: String, profileEventID: String) -> String {
        ""
    }

    func verifyNearbyPresenceEventJson(eventJson: String, peerID: String, myNonce: String, theirNonce: String) -> String {
        ""
    }

    func nearbyEncodeFrame(envelopeJson: String) -> Data {
        Data()
    }

    func nearbyDecodeFrame(frame: Data) -> String {
        ""
    }

    func nearbyFrameBodyLenFromHeader(header: Data) -> Int {
        -1
    }

    func exportSupportBundleJson() -> String {
        supportBundleJson
    }

    func peerProfileDebug(ownerInput: String) -> PeerProfileDebugSnapshot? {
        peerDebug
    }

    func prepareForSuspend() {
        prepareForSuspendLock.lock()
        prepareForSuspendCalls += 1
        prepareForSuspendLock.unlock()
    }

    func listenForUpdates(reconciler: AppReconciler) {
        self.reconciler = reconciler
    }

    func emit(_ update: AppUpdate) {
        reconciler?.reconcile(update: update)
    }
}

private enum MockRustAppError: Error {
    case dispatchFailed
}

final class IrisEmojiPickerSearchTests: XCTestCase {
    func testSearchMatchesUnicodeEmojiNames() {
        XCTAssertTrue(irisEmojiMatchesSearch("👍", category: "Hands", query: "thumbs up"))
        XCTAssertTrue(irisEmojiMatchesSearch("🍕", category: "Food", query: "pizza"))
        XCTAssertFalse(irisEmojiMatchesSearch("🍕", category: "Food", query: "thumbs up"))
    }

    func testSearchMatchesCommonAliases() {
        XCTAssertTrue(irisEmojiMatchesSearch("😂", category: "Smileys", query: "laugh"))
        XCTAssertTrue(irisEmojiMatchesSearch("❤️", category: "Hearts", query: "love"))
    }

    func testQuickReactionsUseBasicSet() {
        XCTAssertEqual(irisReactionQuickChoices(), ["❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"])
    }

    func testMessageReactionSuggestionsIncludeExistingMessageEmoji() {
        XCTAssertEqual(
            irisPostReactionSuggestionEmojis([
                MessageReactionSnapshot(emoji: "🔥", count: 1, reactedByMe: true),
                MessageReactionSnapshot(emoji: "🔥", count: 2, reactedByMe: true),
                MessageReactionSnapshot(emoji: "😂", count: 1, reactedByMe: false),
            ]),
            ["🔥", "😂"]
        )
    }
}

private func makeBusyState() -> BusyState {
    BusyState(
        creatingAccount: false,
        restoringSession: false,
        linkingDevice: false,
        creatingChat: false,
        creatingGroup: false,
        sendingMessage: false,
        updatingRoster: false,
        updatingGroup: false,
        creatingInvite: false,
        acceptingInvite: false,
        syncingNetwork: false,
        uploadingAttachment: false
    )
}

private func makeAppState(
    rev: UInt64 = 0,
    router: Router = Router(defaultScreen: .welcome, screenStack: []),
    account: AccountSnapshot? = nil,
    chatList: [ChatThreadSnapshot] = [],
    currentChat: CurrentChatSnapshot? = nil,
    mobilePush: MobilePushSyncSnapshot = MobilePushSyncSnapshot(
        ownerPubkeyHex: nil,
        messageAuthorPubkeys: [],
        inviteResponsePubkeys: [],
        sessions: []
    ),
    preferences: PreferencesSnapshot = PreferencesSnapshot(
        sendTypingIndicators: true,
        sendReadReceipts: true,
        desktopNotificationsEnabled: true,
        inviteAcceptanceNotificationsEnabled: true,
        startupAtLoginEnabled: false,
        nearbyBluetoothEnabled: false,
        nearbyLanEnabled: false,
        nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
        imageProxyEnabled: true,
        imageProxyUrl: "https://imgproxy.iris.to",
        imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
        imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
        mutedChatIds: [],
        pinnedChatIds: [],
        mobilePushServerUrl: ""
    ),
    toast: String? = nil
) -> AppState {
    AppState(
        rev: rev,
        router: router,
        account: account,
        deviceRoster: nil,
        busy: makeBusyState(),
        chatList: chatList,
        currentChat: currentChat,
        groupDetails: nil,
        publicInvite: nil,
        linkDevice: nil,
        networkStatus: nil,
        mobilePush: mobilePush,
        preferences: preferences,
        toast: toast
    )
}

private func makeAccount() -> AccountSnapshot {
    AccountSnapshot(
        publicKeyHex: "owner",
        npub: "npub-owner",
        displayName: "Alice",
        pictureUrl: nil,
        devicePublicKeyHex: "device",
        deviceNpub: "npub-device",
        hasOwnerSigningAuthority: true,
        authorizationState: .authorized
    )
}

private func makeChatThread(
    unreadCount: UInt64,
    lastMessageIsOutgoing: Bool? = false,
    preview: String? = "hello"
) -> ChatThreadSnapshot {
    ChatThreadSnapshot(
        chatId: "chat-1",
        kind: .direct,
        displayName: "Bob",
        subtitle: nil,
        pictureUrl: nil,
        memberCount: 2,
        lastMessagePreview: preview,
        lastMessageAtSecs: 100,
        lastMessageIsOutgoing: lastMessageIsOutgoing,
        lastMessageDelivery: .received,
        unreadCount: unreadCount,
        isTyping: false,
        isMuted: false,
        isPinned: false,
        draft: ""
    )
}

@MainActor
private func waitUntil(
    timeoutNanoseconds: UInt64 = 1_000_000_000,
    condition: @escaping () -> Bool
) async -> Bool {
    let deadline = DispatchTime.now().uptimeNanoseconds + timeoutNanoseconds
    while DispatchTime.now().uptimeNanoseconds < deadline {
        if condition() {
            return true
        }
        await Task.yield()
    }
    return condition()
}

private func restoreUserDefault(_ previousValue: Any?, forKey key: String) {
    if let previousValue {
        UserDefaults.standard.set(previousValue, forKey: key)
    } else {
        UserDefaults.standard.removeObject(forKey: key)
    }
}

private func makeIsolatedUserDefaults() -> (defaults: UserDefaults, suiteName: String) {
    let suiteName = "IrisChatTests.\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: suiteName)!
    defaults.removePersistentDomain(forName: suiteName)
    return (defaults, suiteName)
}

final class IrisChatTests: XCTestCase {
    func testLaunchRecoveryDefaultsAreClearedWithoutAffectingAuthStartup() {
        let (defaults, suiteName) = makeIsolatedUserDefaults()
        defer { defaults.removePersistentDomain(forName: suiteName) }

        defaults.set(true, forKey: LaunchRecoveryDefaults.pendingKey)
        defaults.set("launch-id", forKey: LaunchRecoveryDefaults.launchIDKey)
        defaults.set("3.0.18", forKey: LaunchRecoveryDefaults.versionKey)
        defaults.set(1_000.0, forKey: LaunchRecoveryDefaults.startedAtKey)
        defaults.set("3.0.18", forKey: LaunchRecoveryDefaults.disabledVersionKey)

        LaunchRecoveryDefaults.clear(userDefaults: defaults)

        XCTAssertNil(defaults.object(forKey: LaunchRecoveryDefaults.pendingKey))
        XCTAssertNil(defaults.string(forKey: LaunchRecoveryDefaults.launchIDKey))
        XCTAssertNil(defaults.string(forKey: LaunchRecoveryDefaults.versionKey))
        XCTAssertNil(defaults.object(forKey: LaunchRecoveryDefaults.startedAtKey))
        XCTAssertNil(defaults.string(forKey: LaunchRecoveryDefaults.disabledVersionKey))
    }

#if os(iOS)
    func testIosStateSideEffectGateIgnoresUnrelatedFullStateChanges() {
        var gate = IosStateSideEffectGate()
        let chatList = [makeChatThread(unreadCount: 0)]
        let push = MobilePushSyncSnapshot(
            ownerPubkeyHex: "owner",
            messageAuthorPubkeys: ["author-1"],
            inviteResponsePubkeys: ["invite-1"],
            sessions: []
        )
        let first = makeAppState(rev: 1, chatList: chatList, mobilePush: push)
        let unrelated = makeAppState(rev: 2, chatList: chatList, mobilePush: push, toast: "Synced")

        XCTAssertTrue(gate.shouldSyncShareSuggestions(chatList: first.chatList))
        XCTAssertFalse(gate.shouldSyncShareSuggestions(chatList: unrelated.chatList))
        XCTAssertTrue(gate.shouldSyncMobilePush(state: first, ownerNsec: "secret"))
        XCTAssertFalse(gate.shouldSyncMobilePush(state: unrelated, ownerNsec: "secret"))
    }

    func testIosStateSideEffectGateTracksPushSecretAvailability() {
        var gate = IosStateSideEffectGate()
        let push = MobilePushSyncSnapshot(
            ownerPubkeyHex: "owner",
            messageAuthorPubkeys: ["author-1"],
            inviteResponsePubkeys: [],
            sessions: []
        )
        let state = makeAppState(rev: 1, mobilePush: push)

        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: nil))
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "secret"))
        XCTAssertFalse(gate.shouldSyncMobilePush(state: state, ownerNsec: "secret"))
    }

    @MainActor
    func testNearbyLanDoesNotAutoStartBeforeLocalNetworkGrant() async throws {
        let attemptedKey = "nearbyLanPermissionPromptAttempted"
        let grantedKey = "nearbyLanPermissionGranted"
        let previousAttempted = UserDefaults.standard.object(forKey: attemptedKey)
        let previousGranted = UserDefaults.standard.object(forKey: grantedKey)
        defer {
            restoreUserDefault(previousAttempted, forKey: attemptedKey)
            restoreUserDefault(previousGranted, forKey: grantedKey)
        }
        UserDefaults.standard.removeObject(forKey: attemptedKey)
        UserDefaults.standard.removeObject(forKey: grantedKey)

        var preferences = makeAppState().preferences
        preferences.nearbyLanEnabled = true
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }

        let manager = AppManager(
            rust: MockRustApp(state: makeAppState(preferences: preferences)),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )

        XCTAssertFalse(manager.nearbyIris.isLanVisible)
    }

    @MainActor
    func testNearbyLanPreferenceSyncWaitsForLocalNetworkGrant() async throws {
        let attemptedKey = "nearbyLanPermissionPromptAttempted"
        let grantedKey = "nearbyLanPermissionGranted"
        let previousAttempted = UserDefaults.standard.object(forKey: attemptedKey)
        let previousGranted = UserDefaults.standard.object(forKey: grantedKey)
        defer {
            restoreUserDefault(previousAttempted, forKey: attemptedKey)
            restoreUserDefault(previousGranted, forKey: grantedKey)
        }
        UserDefaults.standard.removeObject(forKey: attemptedKey)
        UserDefaults.standard.removeObject(forKey: grantedKey)

        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeAppState(rev: 1))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        var preferences = makeAppState().preferences
        preferences.nearbyLanEnabled = true

        rust.emit(.fullState(makeAppState(rev: 2, preferences: preferences)))
        await Task.yield()

        XCTAssertTrue(manager.state.preferences.nearbyLanEnabled)
        XCTAssertFalse(manager.nearbyIris.isLanVisible)
    }

    @MainActor
    func testBackgroundPreparationIsIdempotentUntilForeground() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )

        manager.appBackgrounded()
        manager.appBackgrounded()
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 1)

        manager.appForegrounded()
        manager.appBackgrounded()
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(rust.prepareForSuspendCallCount, 2)
        XCTAssertEqual(rust.dispatchedActions.last, .appForegrounded)
    }

    @MainActor
    func testForegroundEncryptedPushWithUnserializablePayloadIsSuppressed() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(
            rust: MockRustApp(),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let content = UNMutableNotificationContent()
        content.title = "Iris Chat"
        content.body = "New activity"
        content.userInfo = [
            "event": ["kind": 1060],
            "non_json_value": Date(),
        ]

        let options = await manager.foregroundPushPresentationOptions(content: content)

        XCTAssertTrue(options.isEmpty, "opaque encrypted pushes must not show the APNS fallback")
    }

    @MainActor
    func testForegroundEncryptedAliasPushWithStringKindIsSuppressed() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(
            rust: MockRustApp(),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let content = UNMutableNotificationContent()
        content.title = "DM by Someone"
        content.body = "New message"
        content.userInfo = [
            "outer_event_json": #"{"kind":"1060"}"#,
            "non_json_value": Date(),
        ]

        let options = await manager.foregroundPushPresentationOptions(content: content)

        XCTAssertTrue(options.isEmpty, "aliased encrypted pushes must not show the APNS fallback")
    }

    @MainActor
    func testForegroundNonPushWithUnserializablePayloadUsesSystemPresentation() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(
            rust: MockRustApp(),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let content = UNMutableNotificationContent()
        content.title = "Calendar"
        content.body = "Meeting soon"
        content.userInfo = ["non_json_value": Date()]

        let options = await manager.foregroundPushPresentationOptions(content: content)

        XCTAssertTrue(options.contains(.banner))
        XCTAssertTrue(options.contains(.sound))
        XCTAssertTrue(options.contains(.list))
    }

    @MainActor
    func testForegroundGenericIrisFallbackWithoutEventIsSuppressed() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(
            rust: MockRustApp(),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let content = UNMutableNotificationContent()
        content.title = "Iris Chat"
        content.body = "New activity"
        content.userInfo = [
            "aps": ["alert": ["title": "Iris Chat", "body": "New activity"]],
        ]

        let options = await manager.foregroundPushPresentationOptions(content: content)

        XCTAssertTrue(options.isEmpty, "generic Iris APNS fallback should not be presented in foreground")
    }

    @MainActor
    func testForegroundPushSuppressionMatchesCanonicalChatIDPayload() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(
            rust: MockRustApp(
                state: makeAppState(
                    router: Router(defaultScreen: .chat(chatId: "chat-1"), screenStack: [])
                )
            ),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )

        let suppressed = manager.shouldSuppressPushNotification(userInfo: [
            "chat_id": "chat-1",
            "title": "Bob",
            "body": "hello",
        ])

        XCTAssertTrue(suppressed)
    }

    @MainActor
    func testForegroundDecryptedPushForActiveChatIsSuppressed() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let manager = AppManager(
            rust: MockRustApp(
                state: makeAppState(
                    router: Router(defaultScreen: .chat(chatId: "chat-1"), screenStack: [])
                )
            ),
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let content = UNMutableNotificationContent()
        content.title = "Bob"
        content.body = "hello"
        content.userInfo = [
            "iris_foreground_decrypted_push": true,
            "chat_id": "chat-1",
            "title": "Bob",
            "body": "hello",
        ]

        let options = await manager.foregroundPushPresentationOptions(content: content)

        XCTAssertTrue(options.isEmpty)
    }
#endif

    @MainActor
    func testDesktopNotificationPostedForNewUnreadIncomingMessage() async {
        let rust = MockRustApp(
            state: makeAppState(
                rev: 1,
                account: makeAccount(),
                chatList: [makeChatThread(unreadCount: 0)]
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        rust.emit(.fullState(makeAppState(
            rev: 2,
            account: makeAccount(),
            chatList: [makeChatThread(unreadCount: 1, preview: "new text")]
        )))

        let posted = await waitUntil { notifications.posts.count == 1 }
        XCTAssertTrue(posted)
        XCTAssertEqual(notifications.posts.first?.title, "Bob")
        XCTAssertEqual(notifications.posts.first?.body, "new text")
        _ = manager
    }

    @MainActor
    func testDesktopNotificationSuppressedForActiveChatRoute() async {
        let activeRoute = Router(defaultScreen: .chat(chatId: "chat-1"), screenStack: [])
        let rust = MockRustApp(
            state: makeAppState(
                rev: 1,
                router: activeRoute,
                account: makeAccount(),
                chatList: [makeChatThread(unreadCount: 0)]
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        rust.emit(.fullState(makeAppState(
            rev: 2,
            router: activeRoute,
            account: makeAccount(),
            chatList: [makeChatThread(unreadCount: 1, preview: "new text")]
        )))

        XCTAssertTrue(notifications.posts.isEmpty)
        _ = manager
    }

    @MainActor
    func testDesktopNotificationPreferenceSuppressesNewUnreadMessages() async {
        let rust = MockRustApp(
            state: makeAppState(
                rev: 1,
                account: makeAccount(),
                chatList: [makeChatThread(unreadCount: 0)],
                preferences: PreferencesSnapshot(
                    sendTypingIndicators: true,
                    sendReadReceipts: true,
                    desktopNotificationsEnabled: false,
                    inviteAcceptanceNotificationsEnabled: true,
                    startupAtLoginEnabled: false,
                    nearbyBluetoothEnabled: false,
                    nearbyLanEnabled: false,
                    nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
                    imageProxyEnabled: true,
                    imageProxyUrl: "https://imgproxy.iris.to",
                    imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                    imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
                    mutedChatIds: [],
                    pinnedChatIds: [],
                    mobilePushServerUrl: ""
                )
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        rust.emit(.fullState(makeAppState(
            rev: 2,
            account: makeAccount(),
            chatList: [makeChatThread(unreadCount: 1, preview: "new text")],
            preferences: PreferencesSnapshot(
                sendTypingIndicators: true,
                sendReadReceipts: true,
                desktopNotificationsEnabled: false,
                inviteAcceptanceNotificationsEnabled: true,
                startupAtLoginEnabled: false,
                nearbyBluetoothEnabled: false,
                nearbyLanEnabled: false,
                nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
                imageProxyEnabled: true,
                imageProxyUrl: "https://imgproxy.iris.to",
                imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
                mutedChatIds: [],
                pinnedChatIds: [],
                mobilePushServerUrl: ""
            )
        )))

        _ = await waitUntil(timeoutNanoseconds: 50_000_000) { notifications.posts.count == 1 }
        XCTAssertTrue(notifications.posts.isEmpty)
        _ = manager
    }

    func testDeviceApprovalQrRoundTrip() {
        let encoded = DeviceApprovalQr.encode(ownerInput: "npub-owner", deviceInput: "npub-device")
        let decoded = DeviceApprovalQr.decode(encoded)
        XCTAssertEqual(decoded, DeviceApprovalQrPayload(ownerInput: "npub-owner", deviceInput: "npub-device"))
    }

    func testResolveDeviceAuthorizationInputRejectsDifferentOwner() {
        let ownerNpub = "npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj"
        let otherOwner = "npub1m40q2j9vq7yrmgaf4q4f5a30gq2r6hwhzmu7t4j50c5f8ga2g8vs3hmzdt"
        let device = "npub1p34efzmkewwdsksmpp2r0tk7quke9jcfdz2zl7ezk8wnsj43uz2s8x5sp4"
        let qr = DeviceApprovalQr.encode(ownerInput: otherOwner, deviceInput: device)

        let resolved = resolveDeviceAuthorizationInput(
            rawInput: qr,
            ownerNpub: ownerNpub,
            ownerPublicKeyHex: normalizePeerInput(input: ownerNpub)
        )

        XCTAssertEqual(resolved.deviceInput, "")
        XCTAssertEqual(resolved.errorMessage, "This code is for a different profile.")
    }

    func testKeychainSecretStoreRoundTrip() throws {
#if os(macOS)
        throw XCTSkip("macOS test lane uses the file-backed test store to avoid Keychain permission UI")
#else
        let service = "to.iris.chat.tests.\(UUID().uuidString)"
        let account = "stored-account-bundle"
        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        let legacyStore = KeychainSecretStore(service: service, account: account, accessibility: nil)
        legacyStore.clear()
        legacyStore.save(expected)
        XCTAssertEqual(legacyStore.load(), expected)

        let store = KeychainSecretStore(service: service, account: account)
        XCTAssertEqual(store.load(), expected)
        store.save(expected)
        XCTAssertEqual(store.load(), expected)

        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecReturnAttributes: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        XCTAssertEqual(SecItemCopyMatching(query as CFDictionary, &item), errSecSuccess)
        let attributes = item as? [String: Any]
        XCTAssertEqual(
            attributes?[kSecAttrAccessible as String] as? String,
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String
        )

        store.clear()
        XCTAssertNil(store.load())
#endif
    }

    func testNotificationDataDirUsesBackgroundReadableProtection() throws {
#if os(macOS)
        throw XCTSkip("macOS has no iOS Notification Service Extension data protection")
#else
        let fileManager = FileManager.default
        let tempDir = fileManager.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let nestedDir = tempDir.appendingPathComponent("core", isDirectory: true)
        let nestedFile = nestedDir.appendingPathComponent("state.json")
        defer { try? fileManager.removeItem(at: tempDir) }

        try fileManager.createDirectory(at: nestedDir, withIntermediateDirectories: true)
        try Data("{}".utf8).write(to: nestedFile)

        AppPaths.prepareDataDirForBackgroundNotificationReads(tempDir, fileManager: fileManager)

        let keys: Set<URLResourceKey> = [.fileProtectionKey]
        let dirProtection = try tempDir.resourceValues(forKeys: keys).fileProtection
        guard dirProtection != nil else {
            throw XCTSkip("simulator filesystem does not report iOS file-protection attributes")
        }
        XCTAssertEqual(
            dirProtection,
            .completeUntilFirstUserAuthentication
        )
        XCTAssertEqual(
            try nestedFile.resourceValues(forKeys: keys).fileProtection,
            .completeUntilFirstUserAuthentication
        )
#endif
    }

    func testFileAccountSecretStoreRoundTrip() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let store = FileAccountSecretStore(
            url: tempDir.appendingPathComponent("account-secret.json"),
            fileManager: .default
        )
        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        store.save(expected)
        XCTAssertEqual(store.load(), expected)
        store.clear()
        XCTAssertNil(store.load())
    }

#if os(macOS)
    func testMacUiTestSecretStoreUsesDataDirectoryFile() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let secretFile = tempDir.appendingPathComponent("account-secret.json")
        let store = AppPaths.secretStore(
            dataDir: tempDir,
            fileManager: .default,
            environment: ["IRIS_UI_TEST_RUN_ID": UUID().uuidString]
        )
        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        store.save(expected)
        XCTAssertEqual(store.load(), expected)
        XCTAssertTrue(FileManager.default.fileExists(atPath: secretFile.path))
    }
#endif

    @MainActor
    func testAppManagerRestoresPersistedBundleOnLaunch() async {
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let rust = MockRustApp()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        guard let first = rust.dispatchedActions.first else {
            return XCTFail("expected restore action")
        }
        switch first {
        case .restoreAccountBundle(let ownerNsec, let ownerPubkeyHex, let deviceNsec):
            XCTAssertEqual(ownerNsec, "nsec1owner")
            XCTAssertEqual(ownerPubkeyHex, "owner-hex")
            XCTAssertEqual(deviceNsec, "nsec1device")
        default:
            XCTFail("unexpected action \(first)")
        }
        XCTAssertFalse(manager.bootstrapInFlight)
    }

    @MainActor
    func testAppManagerAppliesNewestFullStateOnly() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        let newer = makeAppState(rev: 2, router: Router(defaultScreen: .chatList, screenStack: []), toast: "synced")
        let older = makeAppState(rev: 1)

        rust.emit(.fullState(newer))
        await Task.yield()
        XCTAssertEqual(manager.state.rev, 2)
        XCTAssertEqual(manager.toasts.message, "synced")

        rust.emit(.fullState(older))
        await Task.yield()
        XCTAssertEqual(manager.state.rev, 2)
    }

    @MainActor
    func testPersistAccountBundleSideEffectAppliesEvenWhenRevIsStale() async {
        let rust = MockRustApp(state: makeAppState(rev: 5))
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        rust.emit(
            .persistAccountBundle(
                rev: 1,
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let persisted = await waitUntil {
            store.bundle != nil
        }
        XCTAssertTrue(persisted)
        XCTAssertEqual(manager.state.rev, 5)

        XCTAssertEqual(
            store.bundle,
            StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
    }

    @MainActor
    func testAppManagerExportsPersistedOwnerAndDeviceSecrets() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        XCTAssertEqual(manager.exportOwnerNsec(), "nsec1owner")
        XCTAssertEqual(manager.exportDeviceNsec(), "nsec1device")
    }

    @MainActor
    func testAppManagerExportsDeviceSecretForLinkedDeviceBundle() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: nil,
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        XCTAssertNil(manager.exportOwnerNsec())
        XCTAssertEqual(manager.exportDeviceNsec(), "nsec1device")
    }

    @MainActor
    func testLogoutClearsSecretStoreAndLocalDataDirectory() async {
        let rust = MockRustApp(state: makeAppState(rev: 1))
        rust.onDispatch = { action in
            if action == .logout {
                rust.currentState = makeAppState(rev: 2)
            }
        }
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        let staleFile = tempDir.appendingPathComponent("stale.txt")
        FileManager.default.createFile(atPath: staleFile.path, contents: Data("old".utf8))
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.logout()

        XCTAssertTrue(rust.dispatchedActions.contains(.logout))
        XCTAssertNil(store.load())
        XCTAssertTrue(FileManager.default.fileExists(atPath: tempDir.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: staleFile.path))
        XCTAssertEqual(manager.state.router.defaultScreen, .welcome)
        XCTAssertEqual(manager.state.rev, 2)
    }

    @MainActor
    func testNavigateBackDispatchesExplicitStack() async {
        let rust = MockRustApp(
            state: makeAppState(
                rev: 1,
                router: Router(defaultScreen: .welcome, screenStack: [.chatList, .newChat])
            )
        )
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.navigateBack()

        guard let first = rust.dispatchedActions.first else {
            return XCTFail("expected navigation action")
        }
        XCTAssertEqual(first, .updateScreenStack(stack: [.chatList]))
    }

    @MainActor
    func testNavigateBackFallsBackLocallyWhenDispatchFails() async {
        let rust = MockRustApp(
            state: makeAppState(
                rev: 1,
                router: Router(defaultScreen: .welcome, screenStack: [.chatList, .newChat])
            )
        )
        rust.dispatchError = MockRustAppError.dispatchFailed
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.navigateBack()

        XCTAssertEqual(manager.state.router.screenStack, [.chatList])
        XCTAssertNil(manager.toasts.message)
    }

    @MainActor
    func testDispatchFailureShowsToastInsteadOfEscaping() async {
        let rust = MockRustApp()
        rust.dispatchError = MockRustAppError.dispatchFailed
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.dispatch(.pushScreen(screen: .newChat))

        XCTAssertEqual(manager.toasts.message, "Action failed. Copy support bundle in Settings.")
        XCTAssertTrue(rust.dispatchedActions.isEmpty)
        let supportBundle = manager.supportBundleJson()
        XCTAssertTrue(supportBundle.contains("\"client_log\""))
        XCTAssertTrue(supportBundle.contains("ffi.dispatch.failed"))
        XCTAssertTrue(supportBundle.contains("pushScreen"))
    }

    @MainActor
    func testLiveSafeDispatchPassesActionBufferToRust() async throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let app = FfiApp(dataDir: tempDir.path, keychainGroup: "", appVersion: "test")

        XCTAssertNoThrow(try app.dispatchSafely(action: .pushScreen(screen: .createAccount)))
        let reachedCreateAccount = await waitUntil {
            app.state().router.screenStack == [.createAccount]
        }
        XCTAssertTrue(reachedCreateAccount)
    }

    @MainActor
    func testLiveSafeFfiHelpersReturnFallbacksForBadNearbyInput() async throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let app = FfiApp(dataDir: tempDir.path, keychainGroup: "", appVersion: "test")

        let envelopeJson = #"{"v":1,"type":"hello","peer_id":"abc"}"#
        let frame = app.nearbyEncodeFrameSafely(envelopeJson: envelopeJson)
        XCTAssertFalse(frame.isEmpty)
        let decoded = app.nearbyDecodeFrameSafely(frame: frame)
        let decodedObject = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(decoded.utf8)) as? [String: Any]
        )
        XCTAssertEqual(decodedObject["peer_id"] as? String, "abc")
        XCTAssertEqual(app.nearbyFrameBodyLenFromHeaderSafely(header: Data(frame.prefix(13))), frame.count - 13)

        XCTAssertEqual(app.nearbyDecodeFrameSafely(frame: Data([0x01, 0x02, 0x03])), "")
        XCTAssertEqual(app.nearbyFrameBodyLenFromHeaderSafely(header: Data([0x01])), -1)
        XCTAssertEqual(
            app.verifyNearbyPresenceEventJsonSafely(
                eventJson: "{",
                peerID: "peer",
                myNonce: "mine",
                theirNonce: "theirs"
            ),
            ""
        )
    }

    @MainActor
    func testBootstrapSettlesWithoutStoredCredentials() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertTrue(rust.dispatchedActions.isEmpty)
    }

    @MainActor
    func testBootstrapSettlesAfterRestoringStoredCredentials() async {
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let rust = MockRustApp()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        XCTAssertFalse(manager.bootstrapInFlight)
        XCTAssertEqual(rust.dispatchedActions.count, 1)
    }

    @MainActor
    func testAddAuthorizedDeviceTrimsInputBeforeDispatch() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.addAuthorizedDevice(deviceInput: "  device-hex  ")

        XCTAssertEqual(rust.dispatchedActions.last, .addAuthorizedDevice(deviceInput: "device-hex"))
    }

    @MainActor
    func testCreateGroupAllowsEmptyMemberList() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.createGroup(name: "  Notes  ", memberInputs: [], picture: nil)

        XCTAssertEqual(rust.dispatchedActions.last, .createGroup(name: "Notes", memberInputs: []))
    }

    @MainActor
    func testRemoveAuthorizedDeviceTrimsInputBeforeDispatch() async {
        let rust = MockRustApp()
        let store = InMemorySecretStore()
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.removeAuthorizedDevice(devicePubkeyHex: "  device-hex  ")

        XCTAssertEqual(rust.dispatchedActions.last, .removeAuthorizedDevice(devicePubkeyHex: "device-hex"))
    }

    func testNearbyPeripheralWriteQueueDropsOldestChunks() {
        var queue = IrisNearbyPeripheralWriteQueue()
        for value in 0..<5 {
            queue.append(Data(repeating: UInt8(value), count: 1))
        }

        let dropped = queue.trimToLimits(maxChunks: 3, maxBytes: 64)

        XCTAssertEqual(dropped, 2)
        XCTAssertEqual(queue.count, 3)
        XCTAssertEqual(queue.pendingBytes, 3)
        XCTAssertEqual(queue.popFirst(), Data([2]))
        XCTAssertEqual(queue.popFirst(), Data([3]))
        XCTAssertEqual(queue.popFirst(), Data([4]))
        XCTAssertTrue(queue.isEmpty)
    }

    func testNearbyPeripheralWriteQueueDropsOldestBytes() {
        var queue = IrisNearbyPeripheralWriteQueue()
        queue.append(Data(repeating: 1, count: 100))
        queue.append(Data(repeating: 2, count: 100))
        queue.append(Data(repeating: 3, count: 100))

        let dropped = queue.trimToLimits(maxChunks: 10, maxBytes: 150)

        XCTAssertEqual(dropped, 2)
        XCTAssertEqual(queue.count, 1)
        XCTAssertEqual(queue.pendingBytes, 100)
        XCTAssertEqual(queue.popFirst(), Data(repeating: 3, count: 100))
        XCTAssertTrue(queue.isEmpty)
    }
}
