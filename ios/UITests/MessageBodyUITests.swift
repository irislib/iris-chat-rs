#if os(iOS)
import XCTest

final class MessageBodyUITests: IrisChatUITestCase {
    private let paragraph = String(repeating: "We packed warm blankets, fresh bread, and a small map for the woodland walk. ", count: 7)
        + "We packed warm blankets, fresh bread, and small map for the woodland walk. "
        + "The final sentence is fully readable."

    override func setUpWithError() throws {
        try super.setUpWithError()
        continueAfterFailure = false
    }

    func testWrappedParagraphIsFullyLaidOut() {
        XCTAssertEqual(paragraph.count, 651)
        let app = openMessage(paragraph)
        let text = app.staticTexts.matching(NSPredicate(format: "label == %@", paragraph)).firstMatch
        XCTAssertTrue(text.waitForExistence(timeout: 10))
        capture(app, named: "wrapped-paragraph")
        // A full accessibility label alone is insufficient: SwiftUI exposes
        // it even when the visible Text ends in an ellipsis at line 14.
        XCTAssertGreaterThan(text.frame.height, 350, "The paragraph must extend beyond the 14-line cap")
        XCTAssertFalse(bodyToggle(app).exists)
    }

    func testWrappedParagraphAtAccessibilityTextSize() {
        let app = openMessage(paragraph, largeText: true)
        let text = app.staticTexts.matching(NSPredicate(format: "label == %@", paragraph)).firstMatch
        XCTAssertTrue(text.waitForExistence(timeout: 10))
        // Accessibility text makes this paragraph much taller than the
        // collapsed body; the layout test also checks exact full-text height.
        XCTAssertGreaterThan(text.frame.height, 1_000)
        XCTAssertFalse(bodyToggle(app).exists)
        app.swipeUp()
        capture(app, named: "wrapped-paragraph-large-text")
    }

    func testLongMessageExpandsAndCollapses() {
        let body = paragraph + " " + paragraph
        let app = openMessage(body)
        let text = app.staticTexts.matching(NSPredicate(format: "label == %@", body)).firstMatch
        XCTAssertTrue(text.waitForExistence(timeout: 10))
        let toggle = bodyToggle(app)
        XCTAssertTrue(toggle.waitForExistence(timeout: 5))
        XCTAssertEqual(toggle.label, "Show more")
        let collapsedHeight = text.frame.height
        capture(app, named: "long-message-collapsed")
        toggle.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { toggle.label == "Show less" && text.frame.height > collapsedHeight + 100 })
        app.swipeUp()
        capture(app, named: "long-message-expanded")
        if !toggle.isHittable { app.swipeUp() }
        toggle.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { toggle.label == "Show more" && abs(text.frame.height - collapsedHeight) < 1 })
    }

    func testShortMessageStaysCompact() {
        let app = openMessage("See you soon!")
        let text = app.staticTexts["See you soon!"].firstMatch
        XCTAssertTrue(text.waitForExistence(timeout: 10))
        XCTAssertLessThan(text.frame.height, 60)
        XCTAssertLessThan(text.frame.width, 180)
        XCTAssertFalse(bodyToggle(app).exists)
        capture(app, named: "short-message")
    }

    private func openMessage(_ body: String, largeText: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "message-body-\(UUID().uuidString)"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_DEMO_RELAYS"] = "ws://127.0.0.1:9"
        app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_MESSAGE_BODY"] = body
        if largeText {
            app.launchArguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        }
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))
        submitWelcomeName(app, name: "Alex Rivera", assertFocus: false)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        let row = element(app, "chatRow-fx-chat-1")
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15))
        return app
    }

    private func bodyToggle(_ app: XCUIApplication) -> XCUIElement {
        // The bubble's accessibility identifier propagates to its children.
        app.buttons.matching(NSPredicate(format: "label IN %@", ["Show more", "Show less"])).firstMatch
    }

    private func capture(_ app: XCUIApplication, named name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
