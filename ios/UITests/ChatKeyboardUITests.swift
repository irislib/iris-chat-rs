import XCTest

final class ChatKeyboardUITests: IrisChatUITestCase {
    func testKeyboardKeepsLatestVisibleAndPreservesOlderReadingPosition() throws {
#if os(macOS)
        throw XCTSkip("On-screen keyboard layout is iOS-specific")
#else
        continueAfterFailure = false
        let app = launchCleanApp(seedPeer: "self", seedCount: 40)
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60))
        openSeededChat(app)
        let timeline = app.scrollViews["chatTimeline"].firstMatch
        let input = editableElement(app, "chatMessageInput")
        var latest = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "LAST_SCROLL_SENTINEL")).firstMatch
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        XCTAssertTrue(waitUntil(timeout: 5) { latest.frame.maxY <= input.frame.minY + 4 })
        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(waitUntil(timeout: 5) {
            latest.frame.maxY <= input.frame.minY + 4 && latest.frame.minY > timeline.frame.minY
        }, "Opening the keyboard hid the latest message: latest=\(latest.frame), input=\(input.frame), timeline=\(timeline.frame)")
        capture(app, "ios-latest-keyboard-open")
        let reply = "Writing with the keyboard open"
        input.typeText(reply)
        element(app, "chatSendButton").tap()
        latest = app.staticTexts[reply].firstMatch
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        XCTAssertTrue(waitUntil(timeout: 5) { latest.frame.maxY <= input.frame.minY + 4 })
        XCTAssertTrue(app.keyboards.firstMatch.exists)
        capture(app, "ios-new-message-keyboard-open")
        hideKeyboard(app, timeline: timeline)
        XCTAssertTrue(waitUntil(timeout: 5) { latest.frame.maxY <= input.frame.minY + 4 })
        capture(app, "ios-latest-keyboard-hidden")

        dragVertically(timeline, x: 0.75, fromY: 0.35, toY: 0.85)
        dragVertically(timeline, x: 0.75, fromY: 0.35, toY: 0.85)
        XCTAssertTrue(element(app, "chatJumpToBottom").waitForExistence(timeout: 5))
        let candidate = try XCTUnwrap(app.staticTexts.allElementsBoundByIndex.last {
            $0.label.hasPrefix("seed-msg-") && $0.frame.minY > timeline.frame.minY + 150 && $0.frame.maxY < input.frame.minY
        })
        let reading = app.staticTexts[candidate.identifier]
        let readingY = reading.frame.minY
        let distanceFromComposer = input.frame.minY - readingY
        capture(app, "ios-older-keyboard-hidden-before")
        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(waitUntil(timeout: 5) { abs(input.frame.minY - reading.frame.minY - distanceFromComposer) < 8 }, "Older messages did not move with the keyboard: previous=\(distanceFromComposer), input=\(input.frame), message=\(reading.frame)")
        XCTAssertTrue(element(app, "chatJumpToBottom").exists, "Opening the keyboard jumped to the latest message")
        capture(app, "ios-older-keyboard-open")
        hideKeyboard(app, timeline: timeline)
        XCTAssertTrue(waitUntil(timeout: 5) { abs(reading.frame.minY - readingY) < 6 }, "Hiding the keyboard jumped the older reading position")
        capture(app, "ios-older-keyboard-hidden-after")
        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let start = app.windows.firstMatch.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 180, dy: input.frame.minY - 35))
        let end = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.96))
        start.press(forDuration: 0.1, thenDragTo: end)
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists })
        XCTAssertTrue(element(app, "chatJumpToBottom").exists)
        capture(app, "ios-older-interactive-keyboard-dismissal")
#endif
    }

    private func hideKeyboard(_ app: XCUIApplication, timeline: XCUIElement) {
        app.windows.firstMatch.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 20, dy: 200)).tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists })
    }

    private func capture(_ app: XCUIApplication, _ name: String) {
        let screenshot = app.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
