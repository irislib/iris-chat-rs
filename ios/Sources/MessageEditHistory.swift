import Foundation

func irisCanViewMessageEditHistory(_ message: ChatMessageSnapshot, now: TimeInterval = Date().timeIntervalSince1970) -> Bool {
    !message.deletedForEveryone && message.editHistory.count > 1
        && message.expiresAtSecs.map { Double($0) > now } != false
}

/// Resolve against live state only: retained sheet snapshots must never restore
/// content after deletion, expiration, changing chats or leaving an account.
struct MessageEditHistoryTarget {
    let accountID: String?
    let chatID: String
    let messageID: String

    init(message: ChatMessageSnapshot, accountID: String?) {
        self.accountID = accountID
        self.chatID = message.chatId
        self.messageID = message.id
    }

    func resolve(in state: AppState, now: TimeInterval = Date().timeIntervalSince1970) -> ChatMessageSnapshot? {
        guard let accountID, state.account?.publicKeyHex == accountID,
              let chat = state.currentChat, chat.chatId == chatID,
              let message = chat.messages.first(where: { $0.id == messageID }),
              irisCanViewMessageEditHistory(message, now: now) else { return nil }
        switch state.router.screenStack.last ?? state.router.defaultScreen {
        case .chat(let id), .directChatInfo(let id):
            return id == chatID ? message : nil
        default:
            return nil
        }
    }
}

struct MessageEditHistoryRow: Identifiable {
    let id: String
    let label: String
    let body: String
    let createdAtSecs: UInt64
}

func irisMessageEditHistoryRows(_ message: ChatMessageSnapshot) -> [MessageEditHistoryRow] {
    guard !message.deletedForEveryone else { return [] }
    return message.editHistory.enumerated().reversed().map { index, version in
        MessageEditHistoryRow(
            id: version.id,
            label: index == 0 ? "Original" : (index == message.editHistory.count - 1 ? "Current" : "Edit \(index)"),
            body: version.body,
            createdAtSecs: version.createdAtSecs
        )
    }
}
