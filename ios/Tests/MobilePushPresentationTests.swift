import XCTest
import UserNotifications
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class MobilePushPresentationTests: XCTestCase {
    func testUndecodablePushHasVisibleQuietFallbackInsteadOfRestoringOriginalAlert() {
        let original = push()
        for content in [
            MobilePushPresentation.fallbackContent(from: original),
            MobilePushPresentation.resolvedContent(from: original, resolution: resolution(show: false)),
            MobilePushPresentation.resolvedContent(from: original, resolution: resolution(
                show: true, title: "Iris Chat", body: "New message"
            )),
        ] {
            XCTAssertEqual(content.title, "Iris Chat")
            XCTAssertEqual(content.body, "Background update")
            XCTAssertNil(content.sound)
            XCTAssertNil(content.badge)
            XCTAssertEqual(content.interruptionLevel, .passive)
            XCTAssertEqual(content.userInfo["event"] as? String, original.userInfo["event"] as? String)
        }
        XCTAssertEqual(original.body, "New message", "Do not mutate the request")
    }

    func testRealMessageKeepsItsPreviewAndAlert() {
        let content = MobilePushPresentation.resolvedContent(from: push(), resolution: resolution(
            show: true, title: "Friend", body: "Hello"
        ))
        XCTAssertEqual(content.title, "Friend")
        XCTAssertEqual(content.body, "Hello")
        XCTAssertNotNil(content.sound)
        XCTAssertEqual(content.interruptionLevel, .active)
    }

    func testDecodedControlUpdateRemainsQuiet() {
        let content = MobilePushPresentation.resolvedContent(from: push(), resolution: resolution(
            show: false, title: "Friend", body: "Seen"
        ))
        XCTAssertEqual(content.body, "Seen")
        XCTAssertNil(content.sound)
        XCTAssertNil(content.badge)
        XCTAssertEqual(content.interruptionLevel, .passive)
    }

    private func push() -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = "Iris Chat"
        content.body = "New message"
        content.sound = .default
        content.badge = 1
        content.userInfo = ["event": #"{"kind":"1060"}"#]
        return content
    }

    private func resolution(show: Bool, title: String = "", body: String = "") -> MobilePushNotificationResolution {
        MobilePushNotificationResolution(shouldShow: show, title: title, body: body, payloadJson: "{}")
    }
}
