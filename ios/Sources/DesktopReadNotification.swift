import Foundation
import UserNotifications

struct DesktopReadNotification {
    let identifier: String
    let accountID: String?
    let chatID: String?
    let messageID: String?

    init(identifier: String, accountID: String?, chatID: String?, messageID: String?) {
        self.identifier = identifier
        self.accountID = accountID
        self.chatID = chatID
        self.messageID = messageID
    }

    init(_ request: UNNotificationRequest) {
        self.init(identifier: request.identifier,
            accountID: request.content.userInfo["iris_account_id"] as? String,
            chatID: request.content.userInfo["chatId"] as? String,
            messageID: request.content.userInfo["iris_message_id"] as? String)
    }

    static func readIdentifiers(_ records: [Self], accountID: String, dataDir: String,
                               fullyReadChatIDs: Set<String>,
                               lookup: (String, String, [DesktopNotificationMessage]) -> [UInt64] = {
                                   readDesktopNotificationIndexes(dataDir: $0, ownerPubkeyHex: $1, messages: $2)
                               }) -> [String] {
        let matching = records.filter {
            $0.identifier.hasPrefix("iris-chat-") && $0.accountID == accountID && $0.chatID != nil
        }
        let exact = matching.filter { $0.messageID?.isEmpty == false }
        let messages = exact.map { DesktopNotificationMessage(chatId: $0.chatID!, messageId: $0.messageID!) }
        let indexes = Set(dataDir.isEmpty ? [] : lookup(dataDir, accountID, messages))
        let read = exact.enumerated().compactMap { indexes.contains(UInt64($0.offset)) ? $0.element.identifier : nil }
        let legacy = matching.filter { $0.messageID == nil && fullyReadChatIDs.contains($0.chatID!) }.map(\.identifier)
        return read + legacy
    }
}
