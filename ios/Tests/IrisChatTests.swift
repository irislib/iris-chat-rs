import XCTest
@testable import IrisChat

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
    var onDispatch: ((AppAction) -> Void)?
    private var reconciler: AppReconciler?

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
        networkStatus: nil,
        mobilePush: MobilePushSyncSnapshot(ownerPubkeyHex: nil, messageAuthorPubkeys: [], sessions: []),
        preferences: PreferencesSnapshot(
            sendTypingIndicators: true,
            sendReadReceipts: true,
            desktopNotificationsEnabled: true,
            startupAtLoginEnabled: false,
            nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
            imageProxyEnabled: true,
            imageProxyUrl: "https://imgproxy.iris.to",
            imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
            imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c"
        ),
        toast: nil
    )) {
        self.currentState = state
    }

    func state() -> AppState {
        currentState
    }

    func dispatch(action: AppAction) {
        dispatchedActions.append(action)
        onDispatch?(action)
    }

    func exportSupportBundleJson() -> String {
        supportBundleJson
    }

    func listenForUpdates(reconciler: AppReconciler) {
        self.reconciler = reconciler
    }

    func emit(_ update: AppUpdate) {
        reconciler?.reconcile(update: update)
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
    preferences: PreferencesSnapshot = PreferencesSnapshot(
        sendTypingIndicators: true,
        sendReadReceipts: true,
        desktopNotificationsEnabled: true,
        startupAtLoginEnabled: false,
        nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
        imageProxyEnabled: true,
        imageProxyUrl: "https://imgproxy.iris.to",
        imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
        imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c"
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
        networkStatus: nil,
        mobilePush: MobilePushSyncSnapshot(ownerPubkeyHex: nil, messageAuthorPubkeys: [], sessions: []),
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
        memberCount: 2,
        lastMessagePreview: preview,
        lastMessageAtSecs: 100,
        lastMessageIsOutgoing: lastMessageIsOutgoing,
        lastMessageDelivery: .received,
        unreadCount: unreadCount,
        isTyping: false
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

final class IrisChatTests: XCTestCase {
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
                    startupAtLoginEnabled: false,
                    nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
                    imageProxyEnabled: true,
                    imageProxyUrl: "https://imgproxy.iris.to",
                    imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                    imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c"
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
                startupAtLoginEnabled: false,
                nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
                imageProxyEnabled: true,
                imageProxyUrl: "https://imgproxy.iris.to",
                imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c"
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
        XCTAssertEqual(resolved.errorMessage, "This approval QR belongs to a different owner.")
    }

    func testKeychainSecretStoreRoundTrip() {
        let service = "social.innode.irischat.tests.\(UUID().uuidString)"
        let account = "stored-account-bundle"
        let store = KeychainSecretStore(service: service, account: account)
        let expected = StoredAccountBundle(
            ownerNsec: "nsec1owner",
            ownerPubkeyHex: "owner-hex",
            deviceNsec: "nsec1device"
        )

        store.clear()
        store.save(expected)
        XCTAssertEqual(store.load(), expected)
        store.clear()
        XCTAssertNil(store.load())
    }

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
        XCTAssertEqual(manager.toastMessage, "synced")

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
    func testNavigateBackDispatchesUpdateScreenStack() async {
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
}
