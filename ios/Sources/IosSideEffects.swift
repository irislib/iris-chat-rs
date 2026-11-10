import Foundation

#if os(iOS)
import Intents

func resolvedShareSuggestionChats(
    _ chats: [ChatThreadSnapshot], ownChatId: String? = nil, profile: (String) -> IrisUserProfile?
) -> [ChatThreadSnapshot] {
    chats.filter { !$0.isRequest }.map { chat in
        var resolved = chat
        if chat.chatId == ownChatId {
            resolved.chatId = PendingShare.noteToSelfTarget
        } else if chat.kind == .direct, let profile = profile(chat.chatId) {
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
    private let appGroupIdentifier: String
    private let fileManager: FileManager
    private let queue = DispatchQueue(label: "fi.siriusbusiness.irischat.share-suggestions", qos: .utility)
    private var lastWritten: Data?
    private var exportTask: Task<Void, Never>?
    private var thumbnails: [String: Data] = [:]
    private var savedEntries: [ShareSuggestionEntry] = []
    private var pendingSources: [IrisAvatarImageSource] = []
    private var onAvatars: (([String: Data]) -> Void)?
    private let containerOverride: URL?
    private let loadThumbnail: (IrisAvatarImageSource, AppManager?) async -> Data?

    init(
        appGroupIdentifier: String, fileManager: FileManager = .default,
        containerOverride: URL? = nil,
        loadThumbnail: @escaping (IrisAvatarImageSource, AppManager?) async -> Data? = { source, manager in
            await loadIrisAvatarImage(source: source, manager: manager, maxPixelSize: 120)?
                .jpegData(compressionQuality: 0.85)
        }
    ) {
        self.appGroupIdentifier = appGroupIdentifier
        self.fileManager = fileManager
        self.containerOverride = containerOverride
        self.loadThumbnail = loadThumbnail
        if let container = containerOverride ?? fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier),
           let data = try? Data(contentsOf: container.appendingPathComponent("share-suggestions.json")),
           let entries = try? JSONDecoder().decode([ShareSuggestionEntry].self, from: data) {
            savedEntries = entries
        }
    }

    func export(
        chats: [ChatThreadSnapshot], manager: AppManager?,
        onAvatars: @escaping ([String: Data]) -> Void
    ) {
        self.onAvatars = onAvatars
        let preferences = manager?.state.preferences
        var entries = chats
            .sorted { lhs, rhs in
                if lhs.chatId == PendingShare.noteToSelfTarget || rhs.chatId == PendingShare.noteToSelfTarget {
                    return lhs.chatId == PendingShare.noteToSelfTarget && rhs.chatId != PendingShare.noteToSelfTarget
                }
                return (lhs.lastMessageAtSecs ?? 0) > (rhs.lastMessageAtSecs ?? 0)
            }
            .prefix(20)
            .map { chat in
                ShareSuggestionEntry(
                    chatId: chat.chatId,
                    displayName: chat.displayName,
                    subtitle: chat.subtitle,
                    pictureUrl: chat.pictureUrl,
                    isGroup: chat.kind == .group,
                    lastMessageAtSecs: chat.lastMessageAtSecs
                )
            }
        let sources = entries.map {
            irisAvatarImageSource(pictureUrl: $0.pictureUrl, preferences: preferences, pixelSize: 120)
        }
        let activeKeys = Set(sources.compactMap { $0?.cacheKey })
        thumbnails = thumbnails.filter { activeKeys.contains($0.key) }
        for index in entries.indices {
            if let source = sources[index] {
                let saved = savedEntries.first {
                    $0.chatId == entries[index].chatId && $0.pictureUrl == entries[index].pictureUrl
                }?.avatarData
                let cached = IrisAvatarImageCache.image(for: source.cacheKey)?.jpegData(compressionQuality: 0.85)
                entries[index].avatarData = thumbnails[source.cacheKey] ?? saved ?? cached
                if let data = entries[index].avatarData { thumbnails[source.cacheKey] = data }
            }
        }
        pendingSources = sources.compactMap { $0 }
        publish(entries)
        // Message/typing updates must not keep cancelling the download that
        // supplies the extension's photo. One worker follows the latest list.
        guard exportTask == nil else { return }
        exportTask = Task { [weak self, weak manager] in
            var attempted = Set<String>()
            while let self, !Task.isCancelled,
                  let source = self.pendingSources.first(where: {
                      self.thumbnails[$0.cacheKey] == nil && !attempted.contains($0.cacheKey)
                  }) {
                attempted.insert(source.cacheKey)
                guard let data = await self.loadThumbnail(source, manager),
                      !Task.isCancelled, self.pendingSources.contains(where: { $0.cacheKey == source.cacheKey }),
                      !data.isEmpty else { continue }
                self.thumbnails[source.cacheKey] = data
                let preferences = manager?.state.preferences
                let updated = self.savedEntries.map { entry in
                    var entry = entry
                    if irisAvatarImageSource(pictureUrl: entry.pictureUrl, preferences: preferences, pixelSize: 120)?.cacheKey == source.cacheKey {
                        entry.avatarData = data
                    }
                    return entry
                }
                self.publish(updated)
            }
            self?.exportTask = nil
        }
    }

    private func publish(_ entries: [ShareSuggestionEntry]) {
        savedEntries = entries
        onAvatars?(Dictionary(entries.compactMap { entry in
            entry.avatarData.map { (entry.chatId, $0) }
        }, uniquingKeysWith: { _, latest in latest }))
        guard let data = try? JSONEncoder().encode(Array(entries)) else {
            return
        }
        if data == lastWritten {
            return
        }
        lastWritten = data
        let groupId = appGroupIdentifier
        let fm = fileManager
        let sharedContainer = containerOverride
        queue.async {
            guard let dir = sharedContainer ?? fm.containerURL(forSecurityApplicationGroupIdentifier: groupId) else {
                return
            }
            let url = dir.appendingPathComponent("share-suggestions.json")
            try? data.write(to: url, options: .atomic)
        }
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
