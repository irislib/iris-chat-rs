import XCTest
import UserNotifications

#if os(iOS)
@testable import IrisChat

final class MobilePushNotificationPresentationTests: XCTestCase {
    func testSuppressedResolutionsReturnCompletelyEmptyContent() {
        // Controls may retain preview text in the shared core. shouldShow is
        // authoritative even when that text is nonempty.
        for body in ["", "Seen", "Delivered", "Reacted 👍", "Typing…", "Stopped typing", "Typing update"] {
            let content = MobilePushNotificationPresentation.content(
                for: resolution(body: body), original: serverPlaceholder()
            )
            assertEmpty(content)
        }
    }

    func testMessagesKeepTheirPreviewAndRouting() {
        for body in ["Hello", "Alice: Group message"] {
            let original = serverPlaceholder()
            let content = MobilePushNotificationPresentation.content(
                for: resolution(body: body, shouldShow: true), original: original
            )
            XCTAssertEqual(content.title, "Alice")
            XCTAssertEqual(content.subtitle, "")
            XCTAssertEqual(content.body, body)
            XCTAssertNotNil(content.sound)
            XCTAssertEqual(content.userInfo["iris_account_id"] as? String, "account")
            XCTAssertEqual(content.userInfo["event"] as? String, "encrypted event")
            XCTAssertEqual(content.threadIdentifier, "chat")
            XCTAssertEqual(original.body, "New message")
        }
    }

    func testSuppressingOnePushDoesNotAffectTheNextMessage() {
        let original = serverPlaceholder()
        assertEmpty(MobilePushNotificationPresentation.content(
            for: resolution(body: "Seen"), original: original
        ))
        let message = MobilePushNotificationPresentation.content(
            for: resolution(body: "Hello", shouldShow: true), original: original
        )
        XCTAssertEqual(message.body, "Hello")
        XCTAssertNotNil(message.sound)
    }

    private func assertEmpty(_ content: UNNotificationContent, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(content.title, "", file: file, line: line)
        XCTAssertEqual(content.subtitle, "", file: file, line: line)
        XCTAssertEqual(content.body, "", file: file, line: line)
        XCTAssertNil(content.sound, file: file, line: line)
        XCTAssertNil(content.badge, file: file, line: line)
        XCTAssertTrue(content.userInfo.isEmpty, file: file, line: line)
        XCTAssertTrue(content.attachments.isEmpty, file: file, line: line)
        XCTAssertEqual(content.categoryIdentifier, "", file: file, line: line)
        XCTAssertEqual(content.threadIdentifier, "", file: file, line: line)
    }

    private func resolution(body: String, shouldShow: Bool = false) -> MobilePushNotificationResolution {
        MobilePushNotificationResolution(shouldShow: shouldShow, title: body.isEmpty ? "" : "Alice", body: body, payloadJson: "{}")
    }

    private func serverPlaceholder() -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = "Iris Chat"
        content.subtitle = "Server placeholder"
        content.body = "New message"
        content.sound = .default
        content.badge = 13
        content.userInfo = ["iris_account_id": "account", "event": "encrypted event"]
        content.categoryIdentifier = "message"
        content.threadIdentifier = "chat"
        return content
    }
}
#endif
