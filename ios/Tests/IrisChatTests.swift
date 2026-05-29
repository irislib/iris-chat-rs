import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
@testable import IrisChat
#endif
final class InMemorySecretStore: AccountSecretStore {
    var bundle: StoredAccountBundle?
    var clearSucceeds = true

    init(bundle: StoredAccountBundle? = nil) {
        self.bundle = bundle
    }

    func load() -> StoredAccountBundle? {
        bundle
    }

    func save(_ bundle: StoredAccountBundle) {
        self.bundle = bundle
    }

    @discardableResult
    func clear() -> Bool {
        guard clearSucceeds else {
            return false
        }
        bundle = nil
        return true
    }
}

private final class MockDesktopNotificationPoster: DesktopNotificationPosting {
    var posts: [(title: String, body: String)] = []

    func post(title: String, body: String) {
        posts.append((title: title, body: body))
    }
}

final class MockRustApp: RustAppClient {
    var currentState: AppState
    var supportBundleJson = "{\"ok\":true}"
    var peerDebug: PeerProfileDebugSnapshot?
    var mutualGroupsByOwner: [String: [ChatThreadSnapshot]] = [:]
    var dispatchError: Error?
    var onDispatch: ((AppAction) -> Void)?
    var pagesBefore: [String: CurrentChatSnapshot] = [:]
    var pagesAround: [String: CurrentChatSnapshot] = [:]
    var chatSnapshotGate: DispatchSemaphore?
    private var dispatchedActionsStorage: [AppAction] = []
    private let dispatchedActionsLock = NSLock()
    private var chatSnapshotCallCountStorage = 0
    private let chatSnapshotCallCountLock = NSLock()
    private var prepareForSuspendCalls = 0
    private let prepareForSuspendLock = NSLock()
    private var shutdownCalls = 0
    private let shutdownLock = NSLock()
    private var reconciler: AppReconciler?

    var dispatchedActions: [AppAction] {
        dispatchedActionsLock.lock()
        defer { dispatchedActionsLock.unlock() }
        return dispatchedActionsStorage
    }

    func clearDispatchedActions() { dispatchedActionsLock.lock(); dispatchedActionsStorage.removeAll(); dispatchedActionsLock.unlock() }

    var chatSnapshotCallCount: Int {
        chatSnapshotCallCountLock.lock()
        defer { chatSnapshotCallCountLock.unlock() }
        return chatSnapshotCallCountStorage
    }

    var prepareForSuspendCallCount: Int {
        prepareForSuspendLock.lock()
        defer { prepareForSuspendLock.unlock() }
        return prepareForSuspendCalls
    }

    var shutdownCallCount: Int {
        shutdownLock.lock()
        defer { shutdownLock.unlock() }
        return shutdownCalls
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
            uploadingAttachment: false,
            uploadProgress: nil
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
            nearbyEnabled: true,
            nearbyBluetoothEnabled: false,
            nearbyLanEnabled: false,
            nearbyShowInChatList: true,
            nearbyMailbagEnabled: true,
            nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
            imageProxyEnabled: true,
            imageProxyUrl: "https://imgproxy.iris.to",
            imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
            imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
            mutedChatIds: [],
            pinnedChatIds: [],
            blockedOwnerPubkeys: [],
            acceptedOwnerPubkeys: [],
            debugLoggingEnabled: false,
            acceptUnknownDirectMessages: true,
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
        dispatchedActionsLock.lock()
        dispatchedActionsStorage.append(action)
        dispatchedActionsLock.unlock()
        onDispatch?(action)
    }

    func search(query: String, scopeChatId: String?, limit: UInt32) -> SearchResultSnapshot {
        var result = buildLargeTestSearchResult(
            query: query,
            contactCount: 25,
            groupCount: 9,
            messageCount: max(UInt32(120), limit)
        )
        result.scopeChatId = scopeChatId
        if scopeChatId != nil {
            result.contacts = []
            result.groups = []
        }
        return result
    }

    func chatSnapshot(chatId: String, limit: UInt32) -> CurrentChatSnapshot? {
        chatSnapshotCallCountLock.lock()
        chatSnapshotCallCountStorage += 1
        chatSnapshotCallCountLock.unlock()
        if let gate = chatSnapshotGate {
            chatSnapshotGate = nil
            gate.wait()
        }
        let trimmed = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, currentState.account != nil else { return nil }
        if currentState.currentChat?.chatId == trimmed {
            return currentState.currentChat
        }
        let thread = currentState.chatList.first { $0.chatId == trimmed }
        let groupId = trimmed.hasPrefix("group:") ? String(trimmed.dropFirst("group:".count)) : nil
        return CurrentChatSnapshot(
            chatId: trimmed,
            kind: thread?.kind ?? (groupId == nil ? .direct : .group),
            displayName: thread?.displayName ?? trimmed,
            nickname: thread?.nickname,
            profileName: thread?.profileName,
            subtitle: thread?.subtitle,
            pictureUrl: thread?.pictureUrl,
            about: thread?.about,
            groupId: groupId,
            memberCount: thread?.memberCount ?? 0,
            messageTtlSeconds: nil,
            isMuted: thread?.isMuted ?? false,
            participants: [],
            messages: [],
            typingIndicators: [],
            draft: thread?.draft ?? "",
            isRequest: thread?.isRequest ?? false
        )
    }

    func chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt32) -> CurrentChatSnapshot? {
        pagesBefore["\(chatId.trimmingCharacters(in: .whitespacesAndNewlines))|\(beforeMessageId.trimmingCharacters(in: .whitespacesAndNewlines))"]
    }

    func chatSnapshotAroundMessage(chatId: String, messageId: String, beforeLimit: UInt32, afterLimit: UInt32) -> CurrentChatSnapshot? {
        pagesAround["\(chatId.trimmingCharacters(in: .whitespacesAndNewlines))|\(messageId.trimmingCharacters(in: .whitespacesAndNewlines))"]
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

    func mutualGroups(ownerInput: String) -> [ChatThreadSnapshot] {
        mutualGroupsByOwner[ownerInput] ?? []
    }

    func prepareForSuspend() {
        prepareForSuspendLock.lock()
        prepareForSuspendCalls += 1
        prepareForSuspendLock.unlock()
    }

    func shutdown() {
        shutdownLock.lock()
        shutdownCalls += 1
        shutdownLock.unlock()
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

    func testJumbomojiCountOnlyAcceptsUpToFiveEmojiIgnoringWhitespace() {
        XCTAssertEqual(irisJumbomojiCount("🔥"), 1)
        XCTAssertEqual(irisJumbomojiCount("🔥 😂"), 2)
        XCTAssertEqual(irisJumbomojiCount("👨‍👩‍👧‍👦"), 1)
        XCTAssertEqual(irisJumbomojiCount("😀😃😄😁😆"), 5)
        XCTAssertEqual(irisJumbomojiCount("😀😃😄😁😆😅"), 0)
        XCTAssertEqual(irisJumbomojiCount("nice 🔥"), 0)
    }
}

private let largeFixtureMessageCount: UInt32 = 1_200

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
        uploadingAttachment: false,
        uploadProgress: nil
    )
}

func makeAppState(
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
        nearbyEnabled: true,
        nearbyBluetoothEnabled: false,
        nearbyLanEnabled: false,
        nearbyShowInChatList: true,
        nearbyMailbagEnabled: true,
        nostrRelayUrls: ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://temp.iris.to"],
        imageProxyEnabled: true,
        imageProxyUrl: "https://imgproxy.iris.to",
        imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
        imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
        mutedChatIds: [],
        pinnedChatIds: [],
        blockedOwnerPubkeys: [],
        acceptedOwnerPubkeys: [],
        debugLoggingEnabled: false,
        acceptUnknownDirectMessages: true,
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

private func makeLargeFixtureState(
    rev: UInt64 = 1,
    router: Router? = nil,
    account: AccountSnapshot? = nil,
    chatList: [ChatThreadSnapshot]? = nil,
    currentChat: CurrentChatSnapshot? = nil,
    mobilePush: MobilePushSyncSnapshot? = nil,
    preferences: PreferencesSnapshot? = nil,
    toast: String? = nil
) -> AppState {
    var state = buildLargeTestAppState(
        directChatCount: 80,
        groupChatCount: 20,
        messagesInCurrentChat: largeFixtureMessageCount
    )
    state.rev = rev
    state.preferences.nearbyBluetoothEnabled = false
    state.preferences.nearbyLanEnabled = false
    if let router {
        state.router = router
    }
    if let account {
        state.account = account
    }
    if let chatList {
        state.chatList = chatList
    }
    if let currentChat {
        state.currentChat = currentChat
    }
    if let mobilePush {
        state.mobilePush = mobilePush
    }
    if let preferences {
        state.preferences = preferences
    }
    state.toast = toast
    return state
}

private func makeLargeChatList(replacingFirstWith chat: ChatThreadSnapshot) -> [ChatThreadSnapshot] {
    var rows = makeLargeFixtureState().chatList
    rows.removeAll { $0.chatId == chat.chatId }
    rows.insert(chat, at: 0)
    return rows
}

private func makeAccount() -> AccountSnapshot {
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

private func makeChatThread(
    unreadCount: UInt64,
    lastMessageIsOutgoing: Bool? = false,
    preview: String? = "hello"
) -> ChatThreadSnapshot {
    ChatThreadSnapshot(
        chatId: "chat-1",
        kind: .direct,
        displayName: "Bob",
        nickname: nil,
        profileName: nil,
        subtitle: nil,
        pictureUrl: nil,
        about: nil,
        memberCount: 2,
        lastMessagePreview: preview,
        lastMessageAtSecs: 100,
        lastMessageIsOutgoing: lastMessageIsOutgoing,
        lastMessageDelivery: .received,
        unreadCount: unreadCount,
        isTyping: false,
        isMuted: false,
        isPinned: false,
        draft: "",
        isRequest: false
    )
}

private func makeCurrentChat(
    chatId: String,
    kind: ChatKind = .direct,
    messages: [ChatMessageSnapshot] = []
) -> CurrentChatSnapshot {
    CurrentChatSnapshot(
        chatId: chatId,
        kind: kind,
        displayName: "Chat",
        nickname: nil,
        profileName: nil,
        subtitle: nil,
        pictureUrl: nil,
        about: nil,
        groupId: kind == .group ? String(chatId.dropFirst("group:".count)) : nil,
        memberCount: kind == .group ? 2 : 0,
        messageTtlSeconds: nil,
        isMuted: false,
        participants: [],
        messages: messages,
        typingIndicators: [],
        draft: "",
        isRequest: false
    )
}

private func makeMessage(
    chatId: String,
    id: String,
    body: String? = nil,
    author: String = "owner",
    isOutgoing: Bool = true,
    createdAtSecs: UInt64? = nil
) -> ChatMessageSnapshot {
    ChatMessageSnapshot(
        id: id,
        chatId: chatId,
        kind: .user,
        author: author,
        authorOwnerPubkeyHex: author,
        authorPictureUrl: nil,
        body: body ?? "message \(id)",
        attachments: [],
        reactions: [],
        reactors: [],
        isOutgoing: isOutgoing,
        createdAtSecs: createdAtSecs ?? UInt64(id) ?? 0,
        expiresAtSecs: nil,
        delivery: .sent,
        recipientDeliveries: [],
        deliveryTrace: MessageDeliveryTraceSnapshot(
            outerEventIds: [],
            pendingRelayEventIds: [],
            queuedProtocolTargets: [],
            targetDeviceIds: [],
            transportChannels: [],
            lastTransportError: nil
        ),
        sourceEventId: nil
    )
}

@MainActor
func waitUntil(
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

private func makeShareContainer() throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("IrisChatShare-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(
        at: url.appendingPathComponent("pending-shares", isDirectory: true),
        withIntermediateDirectories: true
    )
    return url
}

private func writePendingShare(
    id: String,
    text: String,
    chatIds: [String],
    to shareContainer: URL,
    autoSend: Bool,
    attachments: [PendingShareAttachment] = []
) throws -> URL {
    let payload = PendingShare(
        id: id,
        text: text,
        attachments: attachments,
        suggestedChatId: chatIds.first,
        suggestedChatIds: chatIds,
        autoSend: autoSend,
        isForward: nil
    )
    let url = shareContainer
        .appendingPathComponent("pending-shares", isDirectory: true)
        .appendingPathComponent(id)
        .appendingPathExtension("json")
    let data = try JSONEncoder().encode(payload)
    try data.write(to: url, options: .atomic)
    return url
}
final class IrisChatTests: XCTestCase {
    func testGroupSenderNameColorsAvoidBrandPurpleAndSpreadDeterministically() {
        let names = [
            "Alice", "Bob", "Charlie", "Dina", "Eve", "Frank",
            "Grace", "Heidi", "Ivan", "Judy", "Mallory", "Niaj",
        ]

        let lightColors = names.map {
            irisGroupSenderNameColorHex(for: $0, isDarkMode: false)
        }
        let darkColors = names.map {
            irisGroupSenderNameColorHex(for: $0, isDarkMode: true)
        }

        XCTAssertFalse(lightColors.contains(0x702ACE))
        XCTAssertFalse(darkColors.contains(0x702ACE))
        XCTAssertEqual(
            irisGroupSenderNameColorHex(for: "Alice", isDarkMode: false),
            irisGroupSenderNameColorHex(for: " alice ", isDarkMode: false)
        )
        XCTAssertGreaterThan(Set(lightColors).count, 4)
        XCTAssertGreaterThan(Set(darkColors).count, 4)
    }

    func testGroupSenderAvatarAndNameFollowAdjacentAuthorLikeSignal() {
        let firstAlice = makeMessage(
            chatId: "group:trip",
            id: "100",
            author: "Alice",
            isOutgoing: false,
            createdAtSecs: 100
        )
        let secondAlice = makeMessage(
            chatId: "group:trip",
            id: "500",
            author: "Alice",
            isOutgoing: false,
            createdAtSecs: 500
        )
        let bob = makeMessage(
            chatId: "group:trip",
            id: "560",
            author: "Bob",
            isOutgoing: false,
            createdAtSecs: 560
        )

        XCTAssertFalse(irisShowsGroupSenderAvatar(message: firstAlice, next: secondAlice, chatKind: .group))
        XCTAssertTrue(irisShowsGroupSenderAvatar(message: secondAlice, next: bob, chatKind: .group))
        XCTAssertTrue(irisShowsGroupSenderAvatar(message: bob, next: nil, chatKind: .group))

        XCTAssertTrue(irisShowsGroupSenderName(previous: nil, message: firstAlice, chatKind: .group))
        XCTAssertFalse(irisShowsGroupSenderName(previous: firstAlice, message: secondAlice, chatKind: .group))
        XCTAssertTrue(irisShowsGroupSenderName(previous: secondAlice, message: bob, chatKind: .group))
    }

    func testGroupSenderAvatarAndNameResetAcrossDateBreak() {
        let firstAlice = makeMessage(
            chatId: "group:trip",
            id: "100",
            author: "Alice",
            isOutgoing: false,
            createdAtSecs: 100
        )
        let nextDayAlice = makeMessage(
            chatId: "group:trip",
            id: "90000",
            author: "Alice",
            isOutgoing: false,
            createdAtSecs: 90_000
        )

        XCTAssertTrue(irisShowsGroupSenderAvatar(message: firstAlice, next: nextDayAlice, chatKind: .group))
        XCTAssertTrue(irisShowsGroupSenderName(previous: firstAlice, message: nextDayAlice, chatKind: .group))
    }

    func testGroupSenderAvatarAndNameOnlyApplyToIncomingGroupMessages() {
        let incoming = makeMessage(
            chatId: "group:trip",
            id: "100",
            author: "Alice",
            isOutgoing: false
        )
        let outgoing = makeMessage(
            chatId: "group:trip",
            id: "101",
            author: "You",
            isOutgoing: true
        )

        XCTAssertFalse(irisShowsGroupSenderAvatar(message: incoming, next: nil, chatKind: .direct))
        XCTAssertFalse(irisShowsGroupSenderName(previous: nil, message: incoming, chatKind: .direct))
        XCTAssertFalse(irisShowsGroupSenderAvatar(message: outgoing, next: nil, chatKind: .group))
        XCTAssertFalse(irisShowsGroupSenderName(previous: nil, message: outgoing, chatKind: .group))
    }

    func testFloatingDaySeparatorKeepsPreviousUntilNextVisibleHeaderReachesStickyTop() throws {
        let previous = daySeparatorFrame(messageId: "yesterday", text: "Yesterday", y: -34)
        let next = daySeparatorFrame(messageId: "today", text: "Today", y: 23)

        let separator = try XCTUnwrap(irisFloatingDaySeparator(
            frames: [previous, next],
            viewportMinY: 0,
            stickyTopY: 12
        ))

        XCTAssertEqual(separator.messageId, "yesterday")
        XCTAssertEqual(separator.text, "Yesterday")
        XCTAssertEqual(separator.offsetY, CGFloat(-4), accuracy: 0.001)
    }

    func testFloatingDaySeparatorSwitchesWhenNextVisibleHeaderPassesStickyTop() throws {
        let previous = daySeparatorFrame(messageId: "yesterday", text: "Yesterday", y: -34)
        let next = daySeparatorFrame(messageId: "today", text: "Today", y: 11)

        let separator = try XCTUnwrap(irisFloatingDaySeparator(
            frames: [previous, next],
            viewportMinY: 0,
            stickyTopY: 12
        ))

        XCTAssertEqual(separator.messageId, "today")
        XCTAssertEqual(separator.text, "Today")
        XCTAssertEqual(separator.offsetY, CGFloat(12), accuracy: 0.001)
    }

    func testFloatingDaySeparatorPushesPreviousBeforeHandoff() throws {
        let previous = daySeparatorFrame(messageId: "yesterday", text: "Yesterday", y: -34)
        let next = daySeparatorFrame(messageId: "today", text: "Today", y: 30)

        let separator = try XCTUnwrap(irisFloatingDaySeparator(
            frames: [previous, next],
            viewportMinY: 0,
            stickyTopY: 12
        ))

        XCTAssertEqual(separator.messageId, "yesterday")
        XCTAssertEqual(separator.offsetY, CGFloat(3), accuracy: 0.001)
    }

    func testFloatingDaySeparatorDoesNotDuplicateFirstVisibleHeader() {
        let today = daySeparatorFrame(messageId: "today", text: "Today", y: 20)

        XCTAssertNil(irisFloatingDaySeparator(frames: [today], viewportMinY: 0, stickyTopY: 12))
    }

    @MainActor
    func testSetUserBlockedDispatchesActionToCore() {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let ownerHex = "aa" + String(repeating: "11", count: 31)
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1, account: makeAccount()))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir
        )

        // Block list lives in the Rust core (Signal-style): the iOS
        // manager has nothing of its own to persist — it just
        // dispatches and trusts the core to surface the new state.
        // Block-list-driven outgoing-message refusal lives in the
        // Rust core too (`send_message` → "User is blocked."), so
        // there's no shell-side fast path left to assert here.
        manager.setUserBlocked(ownerHex, blocked: true)
        XCTAssertTrue(rust.dispatchedActions.contains(.setUserBlocked(ownerPubkeyHex: ownerHex, blocked: true)))

        manager.setUserBlocked(ownerHex, blocked: false)
        XCTAssertTrue(rust.dispatchedActions.contains(.setUserBlocked(ownerPubkeyHex: ownerHex, blocked: false)))
    }

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

    private func daySeparatorFrame(messageId: String, text: String, y: CGFloat) -> ChatTimelineDaySeparatorFrame {
        ChatTimelineDaySeparatorFrame(
            messageId: messageId,
            text: text,
            frame: CGRect(x: 0, y: y, width: 140, height: 22)
        )
    }

#if os(iOS)
    func testIosStateSideEffectGateIgnoresUnrelatedFullStateChanges() {
        var gate = IosStateSideEffectGate()
        let chatList = makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 0))
        let push = MobilePushSyncSnapshot(
            ownerPubkeyHex: "owner",
            messageAuthorPubkeys: ["author-1"],
            inviteResponsePubkeys: ["invite-1"],
            sessions: []
        )
        let first = makeLargeFixtureState(rev: 1, chatList: chatList, mobilePush: push)
        let unrelated = makeLargeFixtureState(rev: 2, chatList: chatList, mobilePush: push, toast: "Synced")

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
        let state = makeLargeFixtureState(rev: 1, mobilePush: push)

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

        var preferences = makeLargeFixtureState().preferences
        preferences.nearbyLanEnabled = true
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }

        let manager = AppManager(
            rust: MockRustApp(state: makeLargeFixtureState(preferences: preferences)),
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
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        var preferences = makeLargeFixtureState().preferences
        preferences.nearbyLanEnabled = true

        rust.emit(.fullState(makeLargeFixtureState(rev: 2, preferences: preferences)))
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
    func testPendingShareUrlWaitsForAuthorizedRestoreBeforeAutoSend() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let shareContainer = try makeShareContainer()
        defer {
            try? FileManager.default.removeItem(at: dataDir)
            try? FileManager.default.removeItem(at: shareContainer)
        }
        let shareID = UUID().uuidString
        let rust = MockRustApp(state: makeAppState(rev: 1))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner",
                deviceNsec: "nsec1device"
            )),
            dataDir: dataDir,
            environment: ["IRIS_SHARE_CONTAINER_DIR": shareContainer.path]
        )
        let payloadURL = try writePendingShare(
            id: shareID,
            text: "hello from share",
            chatIds: ["owner"],
            to: shareContainer,
            autoSend: true
        )

        XCTAssertTrue(manager.handleShareURL(URL(string: "irischat://share/\(shareID)?send=1")!))
        XCTAssertNotNil(manager.pendingShare)
        XCTAssertFalse(rust.dispatchedActions.contains(.sendMessage(chatId: "owner", text: "hello from share")))

        rust.emit(.fullState(makeLargeFixtureState(rev: 2, account: makeAccount())))

        let sentAfterRestore = await waitUntil {
            rust.dispatchedActions.contains(.sendMessage(chatId: "owner", text: "hello from share"))
        }
        XCTAssertTrue(sentAfterRestore)
        XCTAssertTrue(rust.dispatchedActions.contains(.openChat(chatId: "owner")))
        XCTAssertNil(manager.pendingShare)
        XCTAssertFalse(FileManager.default.fileExists(atPath: payloadURL.path))
    }

    @MainActor
    func testPendingAutoShareOnDiskIsSentWhenAppStartsAuthorized() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let shareContainer = try makeShareContainer()
        defer {
            try? FileManager.default.removeItem(at: dataDir)
            try? FileManager.default.removeItem(at: shareContainer)
        }
        let shareID = UUID().uuidString
        let payloadURL = try writePendingShare(
            id: shareID,
            text: "queued while closed",
            chatIds: ["owner"],
            to: shareContainer,
            autoSend: true
        )
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1, account: makeAccount()))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: ["IRIS_SHARE_CONTAINER_DIR": shareContainer.path]
        )

        let sentOnLaunch = await waitUntil {
            rust.dispatchedActions.contains(.sendMessage(chatId: "owner", text: "queued while closed"))
        }
        XCTAssertTrue(sentOnLaunch)
        XCTAssertTrue(rust.dispatchedActions.contains(.openChat(chatId: "owner")))
        XCTAssertNil(manager.pendingShare)
        XCTAssertFalse(FileManager.default.fileExists(atPath: payloadURL.path))
    }

    @MainActor
    func testPendingAutoShareCopiesAttachmentsBeforeClearingStagingFiles() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let shareContainer = try makeShareContainer()
        defer {
            try? FileManager.default.removeItem(at: dataDir)
            try? FileManager.default.removeItem(at: shareContainer)
        }
        let shareID = UUID().uuidString
        let filesDir = shareContainer
            .appendingPathComponent("pending-shares", isDirectory: true)
            .appendingPathComponent("\(shareID)-files", isDirectory: true)
        try FileManager.default.createDirectory(at: filesDir, withIntermediateDirectories: true)
        let sourceURL = filesDir.appendingPathComponent("photo.txt")
        try Data("shared attachment".utf8).write(to: sourceURL)
        let payloadURL = try writePendingShare(
            id: shareID,
            text: "caption",
            chatIds: ["owner"],
            to: shareContainer,
            autoSend: true,
            attachments: [
                PendingShareAttachment(path: sourceURL.path, filename: "photo.txt")
            ]
        )
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1, account: makeAccount()))
        _ = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: ["IRIS_SHARE_CONTAINER_DIR": shareContainer.path]
        )

        let sentOnLaunch = await waitUntil {
            rust.dispatchedActions.contains { action in
                if case let .sendAttachments(chatId, attachments, caption) = action {
                    return chatId == "owner"
                        && caption == "caption"
                        && attachments.count == 1
                        && attachments[0].filename == "photo.txt"
                        && FileManager.default.fileExists(atPath: attachments[0].filePath)
                        && attachments[0].filePath.contains("/attachments/outgoing/")
                }
                return false
            }
        }
        XCTAssertTrue(sentOnLaunch)
        XCTAssertFalse(FileManager.default.fileExists(atPath: payloadURL.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: filesDir.path))
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
                state: makeLargeFixtureState(
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
                state: makeLargeFixtureState(
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
    func testDebugLoggingToggleReachesCoreAndDebugDumpStaysAvailable() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1, account: makeAccount()))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            dataDir: dataDir,
            environment: [:]
        )

        manager.dispatch(.setDebugLoggingEnabled(enabled: true))
        XCTAssertEqual(rust.dispatchedActions.last, .setDebugLoggingEnabled(enabled: true))

        var preferences = makeLargeFixtureState().preferences
        preferences.debugLoggingEnabled = true
        rust.emit(.fullState(makeLargeFixtureState(rev: 2, account: makeAccount(), preferences: preferences)))

        let updated = await waitUntil {
            manager.state.preferences.debugLoggingEnabled
        }
        XCTAssertTrue(updated)
        XCTAssertTrue(manager.supportBundleJson().contains("\"ok\":true"))
    }

    @MainActor
    func testDesktopNotificationPostedForNewUnreadIncomingMessage() async {
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                account: makeAccount(),
                chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 0))
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        rust.emit(.fullState(makeLargeFixtureState(
            rev: 2,
            account: makeAccount(),
            chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 1, preview: "new text"))
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
            state: makeLargeFixtureState(
                rev: 1,
                router: activeRoute,
                account: makeAccount(),
                chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 0))
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        rust.emit(.fullState(makeLargeFixtureState(
            rev: 2,
            router: activeRoute,
            account: makeAccount(),
            chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 1, preview: "new text"))
        )))

        XCTAssertTrue(notifications.posts.isEmpty)
        _ = manager
    }

    @MainActor
    func testDesktopNotificationPostedForActiveChatWhenAppBackgrounded() async {
        let activeRoute = Router(defaultScreen: .chat(chatId: "chat-1"), screenStack: [])
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: activeRoute,
                account: makeAccount(),
                chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 0))
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        // Simulate the OS reporting that the app moved off-screen, e.g. the
        // user switched to another desktop app while the chat was open.
        manager.appBackgrounded()

        rust.emit(.fullState(makeLargeFixtureState(
            rev: 2,
            router: activeRoute,
            account: makeAccount(),
            chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 1, preview: "after backgrounded"))
        )))

        let posted = await waitUntil { notifications.posts.count == 1 }
        XCTAssertTrue(posted)
        XCTAssertEqual(notifications.posts.first?.title, "Bob")
        XCTAssertEqual(notifications.posts.first?.body, "after backgrounded")
        _ = manager
    }

    @MainActor
    func testDesktopNotificationPreferenceSuppressesNewUnreadMessages() async {
        var preferences = makeLargeFixtureState().preferences
        preferences.desktopNotificationsEnabled = false
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                account: makeAccount(),
                chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 0)),
                preferences: preferences
            )
        )
        let notifications = MockDesktopNotificationPoster()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            desktopNotifications: notifications
        )

        rust.emit(.fullState(makeLargeFixtureState(
            rev: 2,
            account: makeAccount(),
            chatList: makeLargeChatList(replacingFirstWith: makeChatThread(unreadCount: 1, preview: "new text")),
            preferences: preferences
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

    func testResolveDeviceAuthorizationInputRejectsPlainDeviceKey() {
        let ownerNpub = "npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj"
        let device = "npub1p34efzmkewwdsksmpp2r0tk7quke9jcfdz2zl7ezk8wnsj43uz2s8x5sp4"

        let resolved = resolveDeviceAuthorizationInput(
            rawInput: device,
            ownerNpub: ownerNpub,
            ownerPublicKeyHex: normalizePeerInput(input: ownerNpub)
        )

        XCTAssertEqual(resolved.deviceInput, "")
        XCTAssertEqual(resolved.errorMessage, "Not a valid link code.")
    }

    func testKeychainSecretStoreRoundTrip() throws {
#if os(macOS)
        throw XCTSkip("macOS test lane uses the file-backed test store to avoid Keychain permission UI")
#else
        let service = "to.iris.chat.tests.\(UUID().uuidString)"
        let account = "stored-account-bundle"
        let probeQuery: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: "\(account)-probe",
            kSecValueData: Data()
        ]
        let probeStatus = SecItemAdd(probeQuery as CFDictionary, nil)
        if probeStatus == errSecMissingEntitlement {
            throw XCTSkip("unsigned simulator test bundle cannot access Keychain")
        }
        XCTAssertEqual(probeStatus, errSecSuccess)
        SecItemDelete(probeQuery as CFDictionary)

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

        XCTAssertTrue(store.clear())
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
        XCTAssertTrue(store.clear())
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
        XCTAssertTrue(manager.bootstrapInFlight)
        rust.emit(.fullState(makeLargeFixtureState(
            rev: 1,
            router: Router(defaultScreen: .chatList, screenStack: []),
            account: makeAccount()
        )))
        await Task.yield()
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

        let newer = makeLargeFixtureState(
            rev: 2,
            router: Router(defaultScreen: .chatList, screenStack: []),
            toast: "synced"
        )
        let older = makeLargeFixtureState(rev: 1)

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
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 5))
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
    func testAppManagerExportsPersistedOwnerSecret() async {
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
    }

    @MainActor
    func testLogoutClearsSecretStoreAndLocalDataDirectory() async {
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1))
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
        XCTAssertEqual(rust.shutdownCallCount, 1)
        XCTAssertNil(store.load())
        XCTAssertTrue(FileManager.default.fileExists(atPath: tempDir.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: staleFile.path))
        XCTAssertEqual(manager.state.router.defaultScreen, .welcome)
        XCTAssertEqual(manager.state.rev, 2)
    }

    @MainActor
    func testLogoutRebindsFreshRustCoreBeforeFutureRestore() async {
        let firstRust = MockRustApp(state: makeLargeFixtureState(rev: 7))
        firstRust.onDispatch = { action in
            if action == .logout {
                firstRust.currentState = makeAppState(rev: 8)
            }
        }
        let freshRust = MockRustApp(state: makeAppState(rev: 0))
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: "owner-hex",
                deviceNsec: "nsec1device"
            )
        )
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: firstRust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:],
            rustFactory: { freshRust }
        )

        await Task.yield()
        manager.logout()
        manager.restoreSession(ownerNsec: "nsec1restored")

        XCTAssertTrue(firstRust.dispatchedActions.contains(.logout))
        XCTAssertEqual(firstRust.shutdownCallCount, 1)
        XCTAssertEqual(manager.state.rev, 0)
        XCTAssertTrue(freshRust.dispatchedActions.contains(.restoreSession(ownerNsec: "nsec1restored")))
        XCTAssertFalse(firstRust.dispatchedActions.contains(.restoreSession(ownerNsec: "nsec1restored")))
    }

    @MainActor
    func testLogoutDoesNotDeleteLocalDataWhenSecretClearFails() async {
        let account = makeAccount()
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1, account: account))
        let store = InMemorySecretStore(
            bundle: StoredAccountBundle(
                ownerNsec: "nsec1owner",
                ownerPubkeyHex: account.publicKeyHex,
                deviceNsec: "nsec1device"
            )
        )
        store.clearSucceeds = false
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        let sessionFile = tempDir.appendingPathComponent("session.json")
        FileManager.default.createFile(atPath: sessionFile.path, contents: Data("session".utf8))
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let manager = AppManager(
            rust: rust,
            secretStore: store,
            dataDir: tempDir,
            environment: [:]
        )

        await Task.yield()
        manager.logout()

        XCTAssertFalse(rust.dispatchedActions.contains(.logout))
        XCTAssertNotNil(store.load())
        XCTAssertTrue(FileManager.default.fileExists(atPath: sessionFile.path))
        XCTAssertEqual(manager.toasts.message, "Could not clear secret key.")
    }

    @MainActor
    func testDeleteProfileAndLocalDataBlanksProfileBeforeLocalReset() async {
        let logoutExpectation = expectation(description: "logout dispatched")
        let rust = MockRustApp(state: makeLargeFixtureState(rev: 1, account: makeAccount()))
        rust.onDispatch = { action in
            switch action {
            case .deleteProfileMetadata:
                rust.currentState = makeLargeFixtureState(rev: 2, account: makeAccount())
            case .logout:
                rust.currentState = makeAppState(rev: 3)
                logoutExpectation.fulfill()
            default:
                break
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
        manager.deleteProfileAndLocalData()
        await fulfillment(of: [logoutExpectation], timeout: 2)

        let deletionFlowActions = rust.dispatchedActions.filter { action in
            switch action {
            case .deleteProfileMetadata, .logout:
                return true
            default:
                return false
            }
        }
        XCTAssertEqual(deletionFlowActions, [.deleteProfileMetadata, .logout])
        XCTAssertNil(store.load())
        XCTAssertTrue(FileManager.default.fileExists(atPath: tempDir.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: staleFile.path))
        XCTAssertEqual(manager.state.router.defaultScreen, .welcome)
        XCTAssertEqual(manager.state.rev, 3)
    }

    @MainActor
    func testNavigateBackDispatchesExplicitStack() async {
        let rust = MockRustApp(
            state: makeLargeFixtureState(
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

        let expectedAction = AppAction.updateScreenStack(stack: [.chatList])
        let dispatched = await waitUntil {
            rust.dispatchedActions.contains(expectedAction)
        }
        XCTAssertTrue(dispatched)
        XCTAssertTrue(rust.dispatchedActions.contains(expectedAction))
        XCTAssertEqual(manager.state.router.screenStack, [.chatList])
    }

    @MainActor
    func testNavigateBackFallsBackLocallyWhenDispatchFails() async {
        let rust = MockRustApp(
            state: makeLargeFixtureState(
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
    func testNavigateBackKeepsLocalRouteWhileRustCatchesUp() async {
        let chatId = "chat-1"
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)])
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

        XCTAssertEqual(manager.state.router.screenStack, [])
        XCTAssertEqual(manager.activeScreen, .chatList)
        XCTAssertNil(manager.state.currentChat)

        rust.emit(
            .fullState(
                makeLargeFixtureState(
                    rev: 2,
                    router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)])
                )
            )
        )
        await Task.yield()

        XCTAssertEqual(manager.state.rev, 2)
        XCTAssertEqual(manager.state.router.screenStack, [])
        XCTAssertEqual(manager.activeScreen, .chatList)
        XCTAssertNil(manager.state.currentChat)

        rust.emit(
            .fullState(
                makeLargeFixtureState(
                    rev: 3,
                    router: Router(defaultScreen: .chatList, screenStack: [])
                )
            )
        )
        await Task.yield()

        XCTAssertEqual(manager.state.rev, 3)
        XCTAssertEqual(manager.state.router.screenStack, [])
        XCTAssertEqual(manager.activeScreen, .chatList)
    }

    @MainActor
    func testOpenChatAppliesLocalRouteWhileRustCatchesUp() async {
        let chatId = "chat-1"
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [])
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
        manager.dispatch(.openChat(chatId: chatId))

        let dispatched = await waitUntil {
            rust.dispatchedActions.contains(.openChat(chatId: chatId))
        }
        XCTAssertTrue(dispatched)
        XCTAssertTrue(rust.dispatchedActions.contains(.openChat(chatId: chatId)))
        XCTAssertEqual(manager.state.router.screenStack, [.chat(chatId: chatId)])
        XCTAssertEqual(manager.activeScreen, .chat(chatId: chatId))
        let initialPageLoaded = await waitUntil {
            manager.state.currentChat?.chatId == chatId
        }
        XCTAssertTrue(initialPageLoaded)
        XCTAssertEqual(manager.state.currentChat?.chatId, chatId)

        rust.emit(
            .fullState(
                makeLargeFixtureState(
                    rev: 2,
                    router: Router(defaultScreen: .chatList, screenStack: [])
                )
            )
        )
        await Task.yield()

        XCTAssertEqual(manager.state.rev, 2)
        XCTAssertEqual(manager.state.router.screenStack, [.chat(chatId: chatId)])
        XCTAssertEqual(manager.activeScreen, .chat(chatId: chatId))
    }

    @MainActor
    func testOpenChatRouteDoesNotWaitForInitialSnapshot() async {
        let chatId = "chat-1"
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [])
            )
        )
        let snapshotGate = DispatchSemaphore(value: 0)
        rust.chatSnapshotGate = snapshotGate
        defer { snapshotGate.signal() }
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
        manager.dispatch(.openChat(chatId: chatId))

        XCTAssertEqual(manager.state.router.screenStack, [.chat(chatId: chatId)])
        XCTAssertEqual(manager.activeScreen, .chat(chatId: chatId))
        let snapshotStarted = await waitUntil {
            rust.chatSnapshotCallCount == 1
        }
        XCTAssertTrue(snapshotStarted)

        snapshotGate.signal()
        let snapshotLoaded = await waitUntil {
            manager.state.currentChat?.chatId == chatId
        }
        XCTAssertTrue(snapshotLoaded)
    }

    @MainActor
    func testOpenChatAtMessageLoadsSearchHitPageOutsideInitialPage() async {
        let chatId = "chat-1"
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [])
            )
        )
        rust.pagesAround["\(chatId)|25"] = makeCurrentChat(
            chatId: chatId,
            messages: (15...35).map { makeMessage(chatId: chatId, id: String($0)) }
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
        manager.openChatAtMessage(chatId: chatId, messageId: "25")

        let openDispatched = await waitUntil {
            rust.dispatchedActions.contains(.openChat(chatId: chatId))
        }
        XCTAssertTrue(openDispatched)
        XCTAssertTrue(rust.dispatchedActions.contains(.openChat(chatId: chatId)))
        XCTAssertEqual(manager.state.router.screenStack, [.chat(chatId: chatId)])
        for _ in 0..<100 where manager.state.currentChat?.messages.contains(where: { $0.id == "25" }) != true {
            await Task.yield()
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertTrue(manager.state.currentChat?.messages.contains(where: { $0.id == "25" }) == true)
    }

    @MainActor
    func testFullStateKeepsLoadedSearchHitContextForVisibleChat() async {
        let chatId = "chat-1"
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)]),
                currentChat: makeCurrentChat(
                    chatId: chatId,
                    messages: (15...35).map { makeMessage(chatId: chatId, id: String($0)) }
                )
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
        rust.emit(
            .fullState(
                makeLargeFixtureState(
                    rev: 2,
                    router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)]),
                    currentChat: makeCurrentChat(
                        chatId: chatId,
                        messages: (121...200).map { makeMessage(chatId: chatId, id: String($0)) }
                    )
                )
            )
        )
        await Task.yield()

        let messageIds = manager.state.currentChat?.messages.map(\.id) ?? []
        XCTAssertEqual(manager.state.rev, 2)
        XCTAssertTrue(messageIds.contains("25"))
        XCTAssertTrue(messageIds.contains("200"))
        XCTAssertEqual(messageIds.first, "15")
        XCTAssertEqual(messageIds.last, "200")
    }

    @MainActor
    func testFullStatePreservesPageOrderForSameSecondVisibleMessages() async {
        let chatId = "chat-1"
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)]),
                currentChat: makeCurrentChat(
                    chatId: chatId,
                    messages: [
                        makeMessage(chatId: chatId, id: "older-context", createdAtSecs: 9)
                    ]
                )
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
        rust.emit(
            .fullState(
                makeLargeFixtureState(
                    rev: 2,
                    router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatId)]),
                    currentChat: makeCurrentChat(
                        chatId: chatId,
                        messages: [
                            makeMessage(chatId: chatId, id: "z-first", createdAtSecs: 10),
                            makeMessage(chatId: chatId, id: "a-second", createdAtSecs: 10),
                            makeMessage(chatId: chatId, id: "m-last", createdAtSecs: 10)
                        ]
                    )
                )
            )
        )
        await Task.yield()

        XCTAssertEqual(
            manager.state.currentChat?.messages.map(\.id),
            ["older-context", "z-first", "a-second", "m-last"]
        )
    }

    @MainActor
    func testPushScreenAppliesLocalRouteWhileRustCatchesUp() async {
        let rust = MockRustApp(
            state: makeLargeFixtureState(
                rev: 1,
                router: Router(defaultScreen: .chatList, screenStack: [])
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
        manager.dispatch(.pushScreen(screen: .settings))

        let dispatched = await waitUntil {
            rust.dispatchedActions.contains(.pushScreen(screen: .settings))
        }
        XCTAssertTrue(dispatched)
        XCTAssertTrue(rust.dispatchedActions.contains(.pushScreen(screen: .settings)))
        XCTAssertEqual(manager.state.router.screenStack, [.settings])
        XCTAssertEqual(manager.activeScreen, .settings)

        rust.emit(
            .fullState(
                makeLargeFixtureState(
                    rev: 2,
                    router: Router(defaultScreen: .chatList, screenStack: [])
                )
            )
        )
        await Task.yield()

        XCTAssertEqual(manager.state.rev, 2)
        XCTAssertEqual(manager.state.router.screenStack, [.settings])
        XCTAssertEqual(manager.activeScreen, .settings)
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

        let failed = await waitUntil {
            manager.toasts.message == "Action failed. Copy support bundle in Settings."
        }
        XCTAssertTrue(failed)
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
        XCTAssertEqual(rust.dispatchedActions.count, 1)
        XCTAssertTrue(manager.bootstrapInFlight)
        rust.emit(.fullState(makeLargeFixtureState(
            rev: 1,
            router: Router(defaultScreen: .chatList, screenStack: []),
            account: makeAccount()
        )))
        await Task.yield()
        XCTAssertFalse(manager.bootstrapInFlight)
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

    #if os(macOS)
    func testMacComposerEmojiInsertionUsesSelectedCursorPosition() {
        let textView = NSTextView()
        textView.string = "hello world"
        textView.setSelectedRange(NSRange(location: 6, length: 0))

        let updated = IrisAppKitComposerTextView.insertTextAtSelection("🙂", into: textView)

        XCTAssertEqual(updated, "hello 🙂world")
        XCTAssertEqual(textView.selectedRange().location, 6 + ("🙂" as NSString).length)
        XCTAssertEqual(textView.selectedRange().length, 0)
    }

    func testMacComposerEmojiInsertionReplacesSelection() {
        let textView = NSTextView()
        textView.string = "abcdef"
        textView.setSelectedRange(NSRange(location: 2, length: 3))

        let updated = IrisAppKitComposerTextView.insertTextAtSelection("🔥", into: textView)

        XCTAssertEqual(updated, "ab🔥f")
        XCTAssertEqual(textView.selectedRange().location, 2 + ("🔥" as NSString).length)
        XCTAssertEqual(textView.selectedRange().length, 0)
    }
    #endif

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
