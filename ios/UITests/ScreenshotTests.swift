import XCTest

#if os(iOS)
/// Drives the app through the screens we publish to the App Store and
/// saves each one as an `XCTAttachment` named `screenshot-<slug>`. The
/// host script (`scripts/screenshot_ios.sh`) extracts the named PNGs from
/// the generated `.xcresult` bundle and writes them under `dist/screenshots/`.
///
/// The whole flow runs against the `IRIS_UI_TEST_SCREENSHOT_FIXTURE`
/// state-override path, so chat rows, message bubbles, and avatars are
/// deterministic across runs.
final class ScreenshotTests: XCTestCase {
    override func setUpWithError() throws {
        try super.setUpWithError()
        continueAfterFailure = false
    }

    func testCaptureAppStoreScreenshots() {
        // Welcome chooser — taken before any account exists so the
        // fixture override doesn't kick in yet.
        let welcomeApp = launchFixtureApp(createAccount: false)
        XCTAssertTrue(welcomeApp.descendants(matching: .any)["welcomeChooserCard"].waitForExistence(timeout: 15))
        sleep(1)
        capture(welcomeApp, named: "01-welcome")
        welcomeApp.terminate()

        // The remaining screens run against a fully-populated fixture
        // account so chat rows / timelines paint the curated demo data.
        let app = launchFixtureApp(createAccount: true)
        XCTAssertTrue(waitForChatList(app, timeout: 30), "chat list never appeared after account creation")

        // Settle one extra second so the chat list rows finish first paint
        // (avatars, last-message timestamps) before we shutter.
        sleep(1)
        capture(app, named: "02-chat-list")

        openFixtureChat(app, index: 0)
        sleep(1)
        capture(app, named: "03-direct-chat")
        returnToChatList(app)

        openFixtureChat(app, index: 1)
        sleep(1)
        capture(app, named: "04-group-chat")
        returnToChatList(app)

        // Nearby modal.
        let nearbyRow = app.descendants(matching: .any)["nearbyChatRow"]
        if nearbyRow.waitForExistence(timeout: 5) {
            nearbyRow.tap()
            if app.descendants(matching: .any)["nearbyCloseButton"].waitForExistence(timeout: 10) {
                sleep(1)
                capture(app, named: "05-nearby")
                app.descendants(matching: .any)["nearbyCloseButton"].tap()
                _ = waitForChatList(app, timeout: 5)
            }
        }

        // New chat screen (push, not a sheet on iPhone).
        if let newChat = waitForAnyElement(app, identifiers: ["chatListNewChatButton", "desktopNewChatRow"], timeout: 10) {
            if newChat.identifier == "chatListNewChatButton" {
                newChat.coordinate(withNormalizedOffset: CGVector(dx: 0.12, dy: 0.5)).tap()
            } else {
                newChat.tap()
            }
            sleep(1)
            capture(app, named: "06-new-chat")
            returnToChatList(app)
        }

        // Settings / profile.
        let profileButton = app.descendants(matching: .any)["chatListProfileButton"]
        if profileButton.waitForExistence(timeout: 10) {
            profileButton.tap()
            if app.descendants(matching: .any)["settingsScreen"].waitForExistence(timeout: 10) {
                sleep(1)
                capture(app, named: "07-settings")
            }
        }
    }

    // MARK: - Helpers

    private func launchFixtureApp(createAccount: Bool) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "screenshot-\(UUID().uuidString)"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))
        if createAccount {
            self.createAccount(in: app)
        }
        return app
    }

    private func createAccount(in app: XCUIApplication) {
        let create = app.descendants(matching: .any)["welcomeCreateAction"]
        XCTAssertTrue(create.waitForExistence(timeout: 15))
        create.tap()
        XCTAssertTrue(app.descendants(matching: .any)["createAccountScreen"].waitForExistence(timeout: 15))
        let nameField = app.descendants(matching: .any)["signupNameField"]
        XCTAssertTrue(nameField.waitForExistence(timeout: 10))
        nameField.tap()
        nameField.typeText("Alex Rivera")
        let terms = app.descendants(matching: .any)["onboardingTermsAgreementToggle"]
        if terms.waitForExistence(timeout: 3), terms.value as? String != "1" {
            terms.tap()
        }
        app.descendants(matching: .any)["generateKeyButton"].tap()
    }

    private func openFixtureChat(_ app: XCUIApplication, index: Int) {
        // Fixture chat IDs are `fx-chat-1`, `fx-chat-2`, ...; chat row
        // accessibility identifiers truncate the chat ID to its first 12
        // characters, which preserves uniqueness for these short IDs.
        let chatId = "fx-chat-\(index + 1)"
        let row = app.descendants(matching: .any)["chatRow-\(String(chatId.prefix(12)))"]
        if !row.waitForExistence(timeout: 10) {
            let debug = XCTAttachment(screenshot: app.screenshot())
            debug.name = "debug-missing-\(chatId)"
            debug.lifetime = .keepAlways
            add(debug)
            XCTFail("fixture chat row \(chatId) never appeared")
            return
        }
        row.tap()
        XCTAssertTrue(app.descendants(matching: .any)["chatMessageInput"].waitForExistence(timeout: 15))
    }

    private func returnToChatList(_ app: XCUIApplication) {
        // Drive the back gesture via the navigation back button's own
        // coordinate, which is reliable even when `isHittable` flickers
        // false during the chat header animation. The element offsets
        // differ between iPhone and iPad header layouts.
        let back = app.descendants(matching: .any)["navigationBackButton"].firstMatch
        if back.waitForExistence(timeout: 5) {
            back.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        }
        _ = waitForChatList(app, timeout: 10)
    }

    private func waitForChatList(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        waitForAnyElement(app, identifiers: ["chatListNewChatButton", "desktopNewChatRow"], timeout: timeout) != nil
    }

    private func waitForAnyElement(
        _ app: XCUIApplication,
        identifiers: [String],
        timeout: TimeInterval
    ) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for identifier in identifiers {
                let candidate = app.descendants(matching: .any)[identifier]
                if candidate.exists {
                    return candidate
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return nil
    }

    private func capture(_ app: XCUIApplication, named name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "screenshot-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
