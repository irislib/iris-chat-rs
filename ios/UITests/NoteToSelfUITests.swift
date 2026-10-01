import XCTest

final class NoteToSelfUITests: IrisChatUITestCase {
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
