import Foundation

#if os(iOS)
import Intents

func resolvedShareSuggestionChats(
    _ chats: [ChatThreadSnapshot], profile: (String) -> IrisUserProfile?
) -> [ChatThreadSnapshot] {
    chats.filter { !$0.isRequest }.map { chat in
        var resolved = chat
        if chat.kind == .direct, let profile = profile(chat.chatId) {
            resolved.displayName = profile.name
            resolved.pictureUrl = profile.pictureUrl
        }
        return resolved
    }
}

@MainActor
final class ShareSuggestionDonor {
    private let groupIdentifier = "iris-chat-share-suggestions"
    private var donatedIdentifiers = Set<String>()
    private var donatingIdentifiers = Set<String>()
    private var avatars: [String: Data] = [:]

    func syncRecentChats(_ chats: [ChatThreadSnapshot], avatars: [String: Data]) {
        self.avatars = avatars
        chats
            .filter { $0.lastMessageAtSecs != nil }
            .sorted { ($0.lastMessageAtSecs ?? 0) > ($1.lastMessageAtSecs ?? 0) }
            .prefix(8)
            .forEach { chat in
                donate(chat: chat, timestampSecs: chat.lastMessageAtSecs)
            }
    }

    func donateSelectedChats(_ chats: [ChatThreadSnapshot]) {
        chats.forEach { chat in
            donate(chat: chat, timestampSecs: nil, force: true)
        }
    }

    private func donate(chat: ChatThreadSnapshot, timestampSecs: UInt64?, force: Bool = false) {
        let chatId = chat.chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !chatId.isEmpty else {
            return
        }
        let timestampKey = timestampSecs.map(String.init) ?? String(Int(Date().timeIntervalSince1970))
        let identifier = "share-\(chatId)-\(timestampKey)"
        let donationKey = "\(identifier)-\(avatars[chatId]?.hashValue ?? 0)"
        guard !donatingIdentifiers.contains(donationKey),
              force || !donatedIdentifiers.contains(donationKey) else {
            return
        }
        donatingIdentifiers.insert(donationKey)

        let displayName = chat.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = displayName.isEmpty ? "Chat" : displayName
        let recipient = INPerson(
            personHandle: INPersonHandle(value: chatId, type: .unknown),
            nameComponents: nil,
            displayName: title,
            image: avatars[chatId].map { INImage(imageData: $0) },
            contactIdentifier: nil,
            customIdentifier: chatId,
            isContactSuggestion: false,
            suggestionType: .instantMessageAddress
        )
        let groupName = chat.kind == .group ? INSpeakableString(spokenPhrase: title) : nil
        let intent = INSendMessageIntent(
            recipients: chat.kind == .direct ? [recipient] : nil,
            outgoingMessageType: .outgoingMessageText,
            content: nil,
            speakableGroupName: groupName,
            conversationIdentifier: chatId,
            serviceName: "Iris Chat",
            sender: nil,
            attachments: nil
        )
        if chat.kind == .group, let data = avatars[chatId] {
            intent.setImage(INImage(imageData: data), forParameterNamed: \.speakableGroupName)
        }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .outgoing
        interaction.identifier = identifier
        interaction.groupIdentifier = groupIdentifier
        if let timestampSecs {
            interaction.dateInterval = DateInterval(
                start: Date(timeIntervalSince1970: TimeInterval(timestampSecs)),
                duration: 1
            )
        }
        interaction.donate { [weak self] error in
            Task { @MainActor in
                self?.donatingIdentifiers.remove(donationKey)
                if error == nil { self?.donatedIdentifiers.insert(donationKey) }
            }
        }
    }
}

@MainActor
final class ShareSuggestionsExporter {
    private let store: ShareSuggestionsStore
    private let loadThumbnail: (IrisAvatarImageSource, AppManager?) async -> Data?

    init(
        appGroupIdentifier: String, fileManager: FileManager = .default,
        containerOverride: URL? = nil,
        loadThumbnail: @escaping (IrisAvatarImageSource, AppManager?) async -> Data? = { source, manager in
            await loadIrisAvatarImage(source: source, manager: manager, maxPixelSize: 120)?
                .jpegData(compressionQuality: 0.85)
        }
    ) {
        store = ShareSuggestionsStore(container: containerOverride
            ?? fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier))
        self.loadThumbnail = loadThumbnail
    }

    func export(
        chats: [ChatThreadSnapshot], manager: AppManager?,
        onAvatars: @escaping ([String: Data]) -> Void
    ) {
        let entries = chats.map { chat in
            ShareSuggestionEntry(chatId: chat.chatId, displayName: chat.displayName,
                subtitle: chat.subtitle, pictureUrl: chat.pictureUrl, isGroup: chat.kind == .group,
                lastMessageAtSecs: chat.lastMessageAtSecs)
        }
        var keys: [String: String] = [:]
        var sources: [String: IrisAvatarImageSource] = [:]
        var cached: [String: Data] = [:]
        for entry in entries {
            guard let source = irisAvatarImageSource(pictureUrl: entry.pictureUrl,
                preferences: manager?.state.preferences, pixelSize: 120) else { continue }
            keys[entry.chatId] = source.cacheKey
            sources[source.cacheKey] = source
            cached[source.cacheKey] = IrisAvatarImageCache.image(for: source.cacheKey)?.jpegData(compressionQuality: 0.85)
        }
        let loader = loadThumbnail
        store.export(entries: entries, thumbnailKeys: keys, cachedThumbnails: cached,
            loadThumbnail: { [weak manager] key in
                guard let source = sources[key] else { return nil }
                return await loader(source, manager)
            }, onAvatars: onAvatars)
    }
}

struct IosMobilePushSyncInput: Equatable {
    let enabled: Bool
    let ownerPubkeyHex: String?
    let ownerSecretAvailable: Bool
    let messageAuthorPubkeys: [String]
    let delayedMessageAuthors: [MobilePushDelayedAuthor]
    let backgroundMessageAuthorPubkeys: [String]
    let inviteResponsePubkeys: [String]
    let mobilePushServerUrl: String

    init(state: AppState, ownerNsec: String?) {
        self.enabled = state.preferences.desktopNotificationsEnabled
        self.ownerPubkeyHex = nonEmptyTrimmedString(state.mobilePush.ownerPubkeyHex)
        self.ownerSecretAvailable = nonEmptyTrimmedString(ownerNsec) != nil
        self.messageAuthorPubkeys = state.mobilePush.messageAuthorPubkeys
        self.delayedMessageAuthors = state.mobilePush.delayedMessageAuthors
        self.backgroundMessageAuthorPubkeys = state.mobilePush.backgroundMessageAuthorPubkeys
        self.inviteResponsePubkeys = state.mobilePush.inviteResponsePubkeys
        self.mobilePushServerUrl = state.preferences.mobilePushServerUrl
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

func nonEmptyTrimmedString(_ value: String?) -> String? {
    let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return trimmed.isEmpty ? nil : trimmed
}

struct IosStateSideEffectGate {
    private var lastShareChatList: [ChatThreadSnapshot]?
    private var lastMobilePushInput: IosMobilePushSyncInput?

    mutating func shouldSyncShareSuggestions(chatList: [ChatThreadSnapshot]) -> Bool {
        guard lastShareChatList != chatList else {
            return false
        }
        lastShareChatList = chatList
        return true
    }

    mutating func shouldSyncMobilePush(state: AppState, ownerNsec: String?) -> Bool {
        let input = IosMobilePushSyncInput(state: state, ownerNsec: ownerNsec)
        guard lastMobilePushInput != input else {
            return false
        }
        lastMobilePushInput = input
        return true
    }

    mutating func resetMobilePush() {
        lastMobilePushInput = nil
    }
}
#endif
