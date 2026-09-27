import XCTest

final class ContactDetailsUITests: IrisChatUITestCase {
    override func setUpWithError() throws {
        try super.setUpWithError()
        continueAfterFailure = false
    }

    func testNicknameAndNoteSaveCancelRelaunchAndRemove() {
        let runId = "contact-details-\(UUID().uuidString)"
        var app = launchApp(runId: runId, reset: true)
        createAccount(app)
        openChatWithPeer(app)
        openDetails(app)
        element(app, "directChatNicknameRow").tap()
        let nickname = editableElement(app, "directChatNicknameField")
        XCTAssertTrue(nickname.waitForExistence(timeout: 5))
        typeText("Work Alice", into: nickname, app: app)
        typeText("Met at lunch. Likes tea.", into: editableElement(app, "directChatNoteField"), app: app)
        capture(app, "contact-details-edit")
        element(app, "directChatSaveNicknameButton").tap()
        XCTAssertTrue(element(app, "directChatNoteText").waitForExistence(timeout: 5))
        XCTAssertEqual(element(app, "directChatNoteText").label, "Met at lunch. Likes tea.")
        capture(app, "contact-details-saved")

        element(app, "directChatNicknameRow").tap()
        typeText(" Unsaved", into: editableElement(app, "directChatNoteField"), app: app)
        element(app, "directChatCancelNicknameButton").tap()
        XCTAssertEqual(element(app, "directChatNoteText").label, "Met at lunch. Likes tea.")

        app.terminate()
        app = launchApp(runId: runId)
        // The app restores the selected conversation on relaunch.
        openDetails(app)
        XCTAssertEqual(element(app, "directChatNoteText").label, "Met at lunch. Likes tea.")
        element(app, "directChatNicknameRow").tap()
        XCTAssertEqual(editableElement(app, "directChatNicknameField").value as? String, "Work Alice")
        element(app, "directChatRemoveNicknameButton").tap()
        XCTAssertFalse(element(app, "directChatNoteText").exists)
        element(app, "directChatNicknameRow").tap()
        XCTAssertTrue(["", "Nickname"].contains(editableElement(app, "directChatNicknameField").value as? String ?? ""))
    }

    private func openDetails(_ app: XCUIApplication) {
        let header = element(app, "chatHeaderTitleButton")
        XCTAssertTrue(header.waitForExistence(timeout: 10))
        header.tap()
        XCTAssertTrue(element(app, "directChatNicknameRow").waitForExistence(timeout: 10))
    }

    private func capture(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
