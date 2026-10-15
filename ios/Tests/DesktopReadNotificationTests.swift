import XCTest
import UserNotifications
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class DesktopReadNotificationTests: XCTestCase {
    func testExactReadKeepsUnreadSiblingOtherAccountAndCall() {
        let records = [
            DesktopReadNotification(identifier: "iris-chat-read", accountID: "owner", chatID: "chat", messageID: "seen"),
            DesktopReadNotification(identifier: "iris-chat-unread", accountID: "owner", chatID: "chat", messageID: "older-unread"),
            DesktopReadNotification(identifier: "iris-chat-other", accountID: "other", chatID: "chat", messageID: "seen"),
            DesktopReadNotification(identifier: "iris-call", accountID: "owner", chatID: "chat", messageID: "seen"),
        ]
        let result = DesktopReadNotification.readIdentifiers(records, accountID: "owner", dataDir: "/fixture", fullyReadChatIDs: []) { _, owner, messages in
            XCTAssertEqual(owner, "owner")
            XCTAssertEqual(messages.map(\.messageId), ["seen", "older-unread"])
            return [0]
        }
        XCTAssertEqual(result, ["iris-chat-read"])
    }

    func testLegacyNeedsFullyReadChatAndExactIDsNeverUseUnreadCount() {
        let records = [
            DesktopReadNotification(identifier: "iris-chat-legacy", accountID: "owner", chatID: "chat", messageID: nil),
            DesktopReadNotification(identifier: "iris-chat-bound", accountID: "owner", chatID: "chat", messageID: "not-proven-read"),
        ]
        XCTAssertTrue(DesktopReadNotification.readIdentifiers(records, accountID: "owner", dataDir: "/fixture", fullyReadChatIDs: [], lookup: { _, _, _ in [] }).isEmpty)
        XCTAssertEqual(DesktopReadNotification.readIdentifiers(records, accountID: "owner", dataDir: "/fixture", fullyReadChatIDs: ["chat"], lookup: { _, _, _ in [] }), ["iris-chat-legacy"])
    }

    func testPersistedNotificationContentCarriesMessageIdentity() {
        let content = SystemDesktopNotificationPoster.content(accountID: "owner", chatID: "chat", messageID: "message", title: "Name", body: "Text")
        let request = UNNotificationRequest(identifier: "iris-chat-restored", content: content, trigger: nil)
        let restored = DesktopReadNotification(request)
        XCTAssertEqual(restored.accountID, "owner")
        XCTAssertEqual(restored.chatID, "chat")
        XCTAssertEqual(restored.messageID, "message")
    }
}
