import XCTest
import UserNotifications

final class NotificationServiceTests: XCTestCase {
    func testUnresolvedEncryptedPushesNeverRestoreServerAlert() {
        for key in ["event", "outer_event", "outer_event_json", "nostr_event", "nostr_event_json"] {
            for event: Any in [["kind": 1060], "{\"kind\":\"1060\"}"] {
                let result = deliver(payload: [key: event])
                assertEmpty(result)
            }
        }
    }

    func testGenericPlaceholderAndInvalidPayloadAreSuppressed() {
        assertEmpty(deliver(payload: [:]))
        assertEmpty(deliver(payload: ["invalid": Date()]))
    }

    func testNonMessagePayloadsAreSuppressed() {
        for kind in [0, 1, 3, 7, 15, 24133] {
            assertEmpty(deliver(payload: ["inner_kind": kind, "title": "Alice", "body": "Control update"]))
        }
    }

    func testChatMessagesAreShown() {
        for body in ["Hello", "Alice: Group message"] {
            let result = deliver(payload: ["inner_kind": 14, "title": "Alice", "body": body])
            XCTAssertEqual(result.title, "Alice")
            XCTAssertEqual(result.body, body)
            XCTAssertNotNil(result.sound)
            XCTAssertEqual(result.userInfo["body"] as? String, body)
        }
    }

    func testExpirationDoesNotCompleteAnAlreadyDeliveredRequestAgain() {
        let service = NotificationService()
        var completions = 0
        service.didReceive(request(payload: [:])) { content in
            completions += 1
            self.assertEmpty(content)
            service.serviceExtensionTimeWillExpire()
        }
        service.serviceExtensionTimeWillExpire()
        XCTAssertEqual(completions, 1)
    }

    private func deliver(payload: [AnyHashable: Any]) -> UNNotificationContent {
        let service = NotificationService()
        var result: UNNotificationContent?
        service.didReceive(request(payload: payload)) { result = $0 }
        XCTAssertNotNil(result)
        return result ?? UNNotificationContent()
    }

    private func request(payload: [AnyHashable: Any]) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = "Iris Chat"
        content.subtitle = "Server subtitle"
        content.body = "New message"
        content.sound = .default
        content.badge = 9
        content.categoryIdentifier = "message"
        content.threadIdentifier = "chat"
        content.userInfo = payload
        return UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
    }

    private func assertEmpty(_ content: UNNotificationContent, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(content.title, "", file: file, line: line)
        XCTAssertEqual(content.subtitle, "", file: file, line: line)
        XCTAssertEqual(content.body, "", file: file, line: line)
        XCTAssertNil(content.sound, file: file, line: line)
        XCTAssertNil(content.badge, file: file, line: line)
        XCTAssertTrue(content.userInfo.isEmpty, file: file, line: line)
        XCTAssertEqual(content.categoryIdentifier, "", file: file, line: line)
        XCTAssertEqual(content.threadIdentifier, "", file: file, line: line)
    }
}
