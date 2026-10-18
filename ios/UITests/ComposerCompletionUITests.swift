import XCTest

final class ComposerCompletionUITests: IrisChatUITestCase {
    func testGroupMentionOverlayKeepsComposerStableAndMobileColonsLiteral() {
        continueAfterFailure = false
        let app = launchCleanApp()
        defer { app.terminate() }
        createAccount(app)
        openSelfOnlyGroup(app)
        let input = editableElement(app, "chatMessageInput")
        typeText("Hi ", into: input, app: app)
        let editorFrame = input.frame
        typeText("@", into: input, app: app)
        XCTAssertTrue(waitUntil(timeout: 5) { (input.value as? String) == "Hi @" }, "Native typing must survive the hosted composer update")
        let person = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'chatMentionCompletion-'")).firstMatch
        XCTAssertTrue(person.waitForExistence(timeout: 10), "Bare @ must immediately offer people in the group")
        XCTAssertEqual(input.frame.minY, editorFrame.minY, accuracy: 1)
        XCTAssertEqual(input.frame.height, editorFrame.height, accuracy: 1)
        capture(app, named: "group-mention-menu")
        person.tap()
        XCTAssertTrue((input.value as? String)?.contains("@") == true)
        XCTAssertFalse((input.value as? String)?.contains("nostr:") == true)
        typeText(":sm", into: input, app: app)
        let smile = app.buttons["chatEmojiCompletion-smile"]
        #if os(macOS)
        XCTAssertTrue(smile.waitForExistence(timeout: 10))
        capture(app, named: "group-emoji-menu")
        smile.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { (input.value as? String)?.contains("😄") == true })
        XCTAssertFalse((input.value as? String)?.contains(":sm") == true)
        #else
        XCTAssertFalse(smile.exists)
        XCTAssertTrue((input.value as? String)?.hasSuffix(":sm") == true)
        typeText("ile:", into: input, app: app)
        XCTAssertTrue((input.value as? String)?.hasSuffix(":smile:") == true)
        XCTAssertFalse(smile.exists)
        #endif
        capture(app, named: "group-mention-and-emoji-draft")
    }

    private func capture(_ app: XCUIApplication, named name: String) {
        let image = XCTAttachment(screenshot: app.screenshot())
        image.name = name
        image.lifetime = .keepAlways
        add(image)
    }
}
