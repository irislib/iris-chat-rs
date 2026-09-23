import XCTest

final class OptionalProfileNameUITests: IrisChatUITestCase {
    func testCreateProfileWithoutName() {
        checkOptionalProfileName("")
    }

    func testCreateProfileWithWhitespaceNameFromKeyboard() {
        checkOptionalProfileName("   ", fromKeyboard: true)
    }

    private func checkOptionalProfileName(_ name: String, fromKeyboard: Bool = false) {
        let app = launchCleanApp()
        tapWelcomeAction(app, "welcomeCreateAction")
        let nameField = editableElement(app, "signupNameField")
        XCTAssertTrue(nameField.waitForExistence(timeout: 15))
        if !name.isEmpty {
            typeText(name, into: nameField, app: app)
        }
        let action = element(app, "generateKeyButton")
#if os(iOS)
        XCTAssertFalse(action.isEnabled, "Terms acceptance is still required")
#endif
        if !action.isEnabled {
            acceptOnboardingTermsIfNeeded(app)
        }
        XCTAssertTrue(action.isEnabled)
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "optional-profile-name"
        attachment.lifetime = .keepAlways
        add(attachment)
        if fromKeyboard {
#if os(macOS)
            nameField.typeText("\n")
#else
            let done = app.keyboards.buttons["Done"]
            XCTAssertTrue(done.waitForExistence(timeout: 5), app.debugDescription)
            done.tap()
#endif
        } else {
            action.tap()
        }
        XCTAssertTrue(waitForChatList(app, timeout: 20))
        element(app, "chatListProfileButton").tap()
        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        let profileRow = element(app, "settingsProfileRow")
        XCTAssertTrue(profileRow.waitForExistence(timeout: 10))
        profileRow.tap()
        let profileName = editableElement(app, "myProfileDisplayNameInput")
        XCTAssertTrue(profileName.waitForExistence(timeout: 10))
        XCTAssertFalse((profileName.value as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }
}
