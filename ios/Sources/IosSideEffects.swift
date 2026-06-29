import Foundation

#if os(iOS)
import Intents

final class ShareSuggestionDonor {
    private let groupIdentifier = "iris-chat-share-suggestions"
    private var donatedIdentifiers = Set<String>()

    func syncRecentChats(_ chats: [ChatThreadSnapshot]) {
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
        guard force || !donatedIdentifiers.contains(identifier) else {
            return
        }
        donatedIdentifiers.insert(identifier)

        let displayName = chat.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = displayName.isEmpty ? "Chat" : displayName
        let recipient = INPerson(
            personHandle: INPersonHandle(value: chatId, type: .unknown),
            nameComponents: nil,
            displayName: title,
            image: nil,
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
        interaction.donate(completion: nil)
    }
}

private struct ShareSuggestionEntry: Codable {
    let chatId: String
    let displayName: String
    let subtitle: String?
    let pictureUrl: String?
    let isGroup: Bool
    let lastMessageAtSecs: UInt64?
}

final class ShareSuggestionsExporter {
    private let appGroupIdentifier: String
    private let fileManager: FileManager
    private let queue = DispatchQueue(label: "fi.siriusbusiness.irischat.share-suggestions", qos: .utility)
    private var lastWritten: Data?

    init(appGroupIdentifier: String, fileManager: FileManager = .default) {
        self.appGroupIdentifier = appGroupIdentifier
        self.fileManager = fileManager
    }

    func export(chats: [ChatThreadSnapshot]) {
        let entries = chats
            .sorted { ($0.lastMessageAtSecs ?? 0) > ($1.lastMessageAtSecs ?? 0) }
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
        guard let data = try? JSONEncoder().encode(Array(entries)) else {
            return
        }
        if data == lastWritten {
            return
        }
        lastWritten = data
        let groupId = appGroupIdentifier
        let fm = fileManager
        queue.async {
            guard let dir = fm.containerURL(forSecurityApplicationGroupIdentifier: groupId) else {
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
    let inviteResponsePubkeys: [String]
    let mobilePushServerUrl: String

    init(state: AppState, ownerNsec: String?) {
        self.enabled = state.preferences.desktopNotificationsEnabled
        self.ownerPubkeyHex = nonEmptyTrimmedString(state.mobilePush.ownerPubkeyHex)
        self.ownerSecretAvailable = nonEmptyTrimmedString(ownerNsec) != nil
        self.messageAuthorPubkeys = state.mobilePush.messageAuthorPubkeys
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
