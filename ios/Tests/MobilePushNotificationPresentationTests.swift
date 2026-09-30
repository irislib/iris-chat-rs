import XCTest
import UserNotifications

#if os(iOS)
@testable import IrisChat

final class MobilePushNotificationPresentationTests: XCTestCase {
    func testFailedDecryptionNeverReturnsAnEmptyOrNewMessageAlert() {
        let content = serverPlaceholder()
        MobilePushNotificationPresentation.prepareFallback(content)
        MobilePushNotificationPresentation.apply(resolution(body: ""), to: content)
        XCTAssertEqual(content.title, "Iris Chat")
        XCTAssertEqual(content.body, "Chat updated")
        XCTAssertNil(content.sound)
        XCTAssertNil(content.badge)
    }

    func testControlsKeepTheirAccurateLabelsWithoutSoundOrBadge() {
        for body in ["Seen", "Delivered", "Typing…", "Stopped typing", "Typing update"] {
            let content = serverPlaceholder()
            MobilePushNotificationPresentation.prepareFallback(content)
            MobilePushNotificationPresentation.apply(resolution(body: body), to: content)
            XCTAssertEqual(content.title, "Alice")
            XCTAssertEqual(content.body, body)
            XCTAssertNil(content.sound)
            XCTAssertNil(content.badge)
        }
    }

    func testEntitledBuildCanHideControlsButStillShowsChatMessages() {
        let content = serverPlaceholder()
        MobilePushNotificationPresentation.apply(resolution(body: "Seen"), to: content, canFilter: true)
        XCTAssertEqual(content.title, "")
        XCTAssertEqual(content.body, "")
        XCTAssertNil(content.sound)
        MobilePushNotificationPresentation.apply(resolution(body: "Hello", shouldShow: true), to: content, canFilter: true)
        XCTAssertEqual(content.title, "Alice")
        XCTAssertEqual(content.body, "Hello")
        XCTAssertNotNil(content.sound)
    }

    private func resolution(body: String, shouldShow: Bool = false) -> MobilePushNotificationResolution {
        MobilePushNotificationResolution(shouldShow: shouldShow, title: body.isEmpty ? "" : "Alice", body: body, payloadJson: "{}")
    }

    private func serverPlaceholder() -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = "Iris Chat"
        content.body = "New message"
        content.sound = .default
        content.badge = 13
        return content
    }
}
#endif
