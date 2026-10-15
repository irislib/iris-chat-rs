import XCTest

final class ComposerCompletionUITests: IrisChatUITestCase {
    func testGroupMentionAndEmojiMenusUseTheNativeComposer() {
        continueAfterFailure = false
        let app = launchCleanApp()
        defer { app.terminate() }
        createAccount(app)
        openSelfOnlyGroup(app)
        let input = editableElement(app, "chatMessageInput")
        typeText("@", into: input, app: app)
        XCTAssertTrue(waitUntil(timeout: 5) { (input.value as? String) == "@" }, "Native typing must survive the hosted composer update")
        let person = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'chatMentionCompletion-'")).firstMatch
        XCTAssertTrue(person.waitForExistence(timeout: 10), "Bare @ must immediately offer people in the group")
        capture(app, named: "group-mention-menu")
        person.tap()
        XCTAssertTrue((input.value as? String)?.contains("@") == true)
        XCTAssertFalse((input.value as? String)?.contains("nostr:") == true)
        typeText(":sm", into: input, app: app)
        let smile = app.buttons["chatEmojiCompletion-smile"]
        XCTAssertTrue(smile.waitForExistence(timeout: 10))
        capture(app, named: "group-emoji-menu")
        smile.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { (input.value as? String)?.contains("😄") == true })
        XCTAssertFalse((input.value as? String)?.contains(":sm") == true)
        capture(app, named: "group-mention-and-emoji-draft")
    }

    private func capture(_ app: XCUIApplication, named name: String) {
        let image = XCTAttachment(screenshot: app.screenshot())
        image.name = name
        image.lifetime = .keepAlways
        add(image)
    }
}
