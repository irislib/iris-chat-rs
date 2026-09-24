import Foundation
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class MockRustApp: RustAppClient {
    var currentState: AppState
    var supportBundleJson = "{\"ok\":true}"
    var peerDebug: PeerProfileDebugSnapshot?
    var onProfileRead: (() -> Void)?
    var mutualGroupsByOwner: [String: [ChatThreadSnapshot]] = [:]
    var dispatchError: Error?
    var onDispatch: ((AppAction) -> Void)?
    var pagesBefore: [String: CurrentChatSnapshot] = [:]
    var pagesAround: [String: CurrentChatSnapshot] = [:]
    var chatSnapshotGate: DispatchSemaphore?
    var chatSnapshotOverride: CurrentChatSnapshot?
    var onSearch: (() -> Void)?
    private var dispatchedActionsStorage: [AppAction] = []
    private let dispatchedActionsLock = NSLock()
    private var chatSnapshotCallCountStorage = 0
    private let chatSnapshotCallCountLock = NSLock()
    private var prepareForSuspendCalls = 0
    private var prepareForSuspendCallback: (() -> Void)?
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

    func onNextPrepareForSuspend(_ callback: @escaping () -> Void) {
        prepareForSuspendLock.lock()
        prepareForSuspendCallback = callback
        prepareForSuspendLock.unlock()
    }

    var shutdownCallCount: Int {
        shutdownLock.lock()
        defer { shutdownLock.unlock() }
        return shutdownCalls
    }

    init(state: AppState = AppState(
        rev: 0,
        call: nil,
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
        linkDevice: nil, remoteSignerLogin: nil,
        networkStatus: nil,
        mobilePush: MobilePushSyncSnapshot(
            callDevicePubkeyHex: nil, callAuthorPubkeys: [],
            ownerPubkeyHex: nil,
            messageAuthorPubkeys: [],
            backgroundMessageAuthorPubkeys: [],
            inviteResponsePubkeys: [],
            sessions: []
        ),
        preferences: PreferencesSnapshot(
            voiceCallsEnabled: true,
            videoCallsEnabled: true, callQuality: "auto", callMaxBitrateBps: 2_000_000,
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
            imageProxyFallbackEnabled: false,
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
        userDiscoveryRevision: 0,
        userDiscoverySyncing: false,
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
        onSearch?()
        var result = buildLargeTestSearchResult(
            query: query,
            personCount: 11,
            contactCount: 25,
            groupCount: 9,
            messageCount: max(UInt32(120), limit)
        )
        result.scopeChatId = scopeChatId
        if scopeChatId != nil {
            result.people = []
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
        if let snapshot = chatSnapshotOverride { return snapshot }
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
            isRequest: thread?.isRequest ?? false,
            directChatCapability: groupId == nil ? .available : nil
        )
    }

    func chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt32) -> CurrentChatSnapshot? {
        pagesBefore["\(chatId.trimmingCharacters(in: .whitespacesAndNewlines))|\(beforeMessageId.trimmingCharacters(in: .whitespacesAndNewlines))"]
    }

    func chatSnapshotAroundMessage(chatId: String, messageId: String, beforeLimit: UInt32, afterLimit: UInt32) -> CurrentChatSnapshot? {
        pagesAround["\(chatId.trimmingCharacters(in: .whitespacesAndNewlines))|\(messageId.trimmingCharacters(in: .whitespacesAndNewlines))"]
    }

    func exportSupportBundleJson() -> String {
        supportBundleJson
    }

    func peerProfileDebug(ownerInput: String) -> PeerProfileDebugSnapshot? {
        onProfileRead?()
        return peerDebug
    }

    func mutualGroups(ownerInput: String) -> [ChatThreadSnapshot] {
        onProfileRead?()
        return mutualGroupsByOwner[ownerInput] ?? []
    }

    func prepareForSuspend() {
        prepareForSuspendLock.lock()
        prepareForSuspendCalls += 1
        let callback = prepareForSuspendCallback
        prepareForSuspendCallback = nil
        prepareForSuspendLock.unlock()
        callback?()
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

