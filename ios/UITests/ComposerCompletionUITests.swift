import XCTest

final class ComposerCompletionUITests: IrisChatUITestCase {
    func testGroupMentionOverlayKeepsComposerStableAndMobileColonsLiteral() {
        continueAfterFailure = false
        let app = launchCleanApp()
        defer {
            capture(app, named: "group-completion-final-state")
            app.terminate()
        }
        createAccount(app)
        openSelfOnlyGroup(app)
        let input = editableElement(app, "chatMessageInput")
        typeText("Hi ", into: input, app: app)
        #if os(iOS)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(waitUntil(timeout: 5) {
            input.frame.maxY <= app.keyboards.firstMatch.frame.minY
        }, "The editor must be above the visible keyboard before measuring completion layout")
        #endif
        var editorFrame = input.frame
        var lastChange = Date()
        XCTAssertTrue(waitUntil(timeout: 5) {
            let current = input.frame
            if current != editorFrame { editorFrame = current; lastChange = Date() }
            return Date().timeIntervalSince(lastChange) >= 0.5
        }, "The keyboard transition must finish before measuring the editor")
        capture(app, named: "group-editor-before-mention")
        #if os(iOS)
        input.typeText("@")
        #else
        typeText("@", into: input, app: app)
        #endif
        XCTAssertTrue(waitUntil(timeout: 5) { (input.value as? String) == "Hi @" }, "Native typing must survive the hosted composer update")
        let person = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'chatMentionCompletion-'")).firstMatch
        XCTAssertTrue(person.waitForExistence(timeout: 10), "Bare @ must immediately offer people in the group")
        #if os(iOS)
        // A keyboard update can still be presenting an intermediate frame after
        // the suggestion becomes accessible. Compare settled geometry on both
        // sides; keep the same one-point bound so a lasting shift still fails.
        var mentionFrame = input.frame
        var mentionLastChange = Date()
        XCTAssertTrue(waitUntil(timeout: 5) {
            let current = input.frame
            if current != mentionFrame { mentionFrame = current; mentionLastChange = Date() }
            return Date().timeIntervalSince(mentionLastChange) >= 0.5
        }, "The mention editor frame must settle before comparing layout")
        print("IRIS_COMPOSER_FRAMES before=\(editorFrame) after=\(mentionFrame) keyboard=\(app.keyboards.firstMatch.frame)")
        capture(app, named: "group-mention-before-geometry-check")
        #endif
        XCTAssertEqual(input.frame.minY, editorFrame.minY, accuracy: 1)
        XCTAssertEqual(input.frame.height, editorFrame.height, accuracy: 1)
        XCTAssertLessThanOrEqual(person.frame.maxY, input.frame.minY,
                                 "Suggestions must remain above the editable text")
        XCTAssertTrue(input.isHittable, "The mention panel must not cover the editor")
        capture(app, named: "group-mention-menu")
        person.tap()
        XCTAssertTrue((input.value as? String)?.contains("@") == true)
        XCTAssertFalse((input.value as? String)?.contains("nostr:") == true)
        #if os(iOS)
        // Keep typing at the insertion point established by the mention.
        // Tapping the editor again moves the caret into the existing draft.
        input.typeText(":sm")
        #else
        typeText(":sm", into: input, app: app)
        #endif
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
        input.typeText("ile:")
        XCTAssertTrue((input.value as? String)?.hasSuffix(":smile:") == true,
                      "The full colon shortcode must remain literal: \(input.value ?? "nil")")
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
