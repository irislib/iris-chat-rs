import XCTest

final class NoteToSelfUITests: IrisChatUITestCase {
#if os(iOS)
    func testForwardToSelfBeforeWritingTheFirstNote() {
        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)
        let text = "Save the picnic date"
        typeText(text, into: editableElement(app, "chatMessageInput"), app: app)
        element(app, "chatSendButton").tap()
        dismissNotificationPromptIfPresent(app: app)
        let message = app.staticTexts[text].firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 10))
        message.press(forDuration: 0.6)
        let forward = app.buttons["Forward"].firstMatch
        XCTAssertTrue(forward.waitForExistence(timeout: 5))
        forward.tap()

        let target = app.buttons["Note to self, not selected"].firstMatch
        XCTAssertTrue(target.waitForExistence(timeout: 5))
        target.tap()
        capture(app, "note-to-self-forward-target")
        element(app, "shareTargetSendButton").tap()
        let header = element(app, "chatHeaderTitleButton")
        XCTAssertTrue(waitUntil(timeout: 10) { header.label.contains("Note to self") })
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text))
            .firstMatch.waitForExistence(timeout: 10))
    }
#endif

    func testSearchAndProfileOpenTheSameSelfChat() {
        let app = launchCleanApp(runId: "note-to-self-\(UUID().uuidString)")
        createAccount(app)

        let search = editableElement(app, "chatListSearchField")
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        typeText("note to self", into: search, app: app)
        let result = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Note to self")).firstMatch
        XCTAssertTrue(result.waitForExistence(timeout: 10))
        capture(app, "note-to-self-search")
        result.tap()

        let input = editableElement(app, "chatMessageInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        typeText("Remember the tea", into: input, app: app)
        #if os(macOS)
        app.typeKey(.return, modifierFlags: [])
        #else
        element(app, "chatSendButton").tap()
        #endif
        let message = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "chatMessage-")
        ).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 10))
        let messageId = message.identifier
        capture(app, "note-to-self-conversation")

        #if os(iOS)
        returnToChatList(app)
        #endif
        element(app, "chatListProfileButton").tap()
        openSettingsPage(app, "settingsProfileRow")
        let profileAction = element(app, "myProfileNoteToSelfButton")
        XCTAssertTrue(profileAction.waitForExistence(timeout: 10))
        capture(app, "note-to-self-profile")
        profileAction.tap()
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        XCTAssertTrue(app.descendants(matching: .any).matching(identifier: messageId).firstMatch.waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "settingsScreen").exists)
    }

    private func capture(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
