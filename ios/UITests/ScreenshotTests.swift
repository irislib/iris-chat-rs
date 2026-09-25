import XCTest

#if os(iOS) || os(macOS)
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

    #if os(iOS)
    func testVoiceCallButtonEdgeResponds() {
        assertCallButtonEdgeResponds("startVoiceCallButton")
    }

    func testVideoCallButtonEdgeResponds() {
        assertCallButtonEdgeResponds("startVideoCallButton")
    }

    private func assertCallButtonEdgeResponds(_ id: String) {
        let app = launchFixtureApp(createAccount: true)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        openFixtureChat(app, index: 0)
        capture(app, named: "call-button-hit-areas")
        // Fixture chats do not exist in the core: its validation toast proves
        // the real button/permission/dispatch path ran without placing a call.
        let button = app.buttons[id]
        XCTAssertTrue(button.waitForExistence(timeout: 5))
        XCTAssertGreaterThanOrEqual(button.frame.width, 44)
        XCTAssertGreaterThanOrEqual(button.frame.height, 44)
        button.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.08)).tap()
        let toast = app.staticTexts["Open an accepted chat to call"]
        let system = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        var responded = toast.waitForExistence(timeout: 2)
        for _ in 0..<2 {
            if responded { break }
            let alert = system.alerts.firstMatch
            guard alert.exists else { break }
            let allow = alert.buttons["Allow"]
            if allow.exists { allow.tap() }
            else { alert.buttons["OK"].tap() }
            responded = toast.waitForExistence(timeout: 2)
        }
        XCTAssertTrue(responded, "One edge tap must reach call validation promptly")
    }

    func testCallQualitySettings() {
        let app = launchFixtureApp(createAccount: true)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        let profile = app.descendants(matching: .any)["chatListProfileButton"]
        XCTAssertTrue(profile.waitForExistence(timeout: 10))
        profile.tap()
        let messaging = app.descendants(matching: .any)["settingsMessagingRow"]
        XCTAssertTrue(messaging.waitForExistence(timeout: 10))
        messaging.tap()
        let quality = app.descendants(matching: .any)["myProfileCallQualityButton"]
        XCTAssertTrue(quality.waitForExistence(timeout: 10))
        if !quality.isHittable { app.swipeUp() }
        quality.tap()
        capture(app, named: "call-quality-open")
        let picker = app.descendants(matching: .any)["callQualityPicker"]
        XCTAssertTrue(picker.waitForExistence(timeout: 10))
        picker.tap()
        app.buttons["Custom"].tap()
        let slider = app.sliders["Maximum bitrate"]
        XCTAssertTrue(slider.waitForExistence(timeout: 5))
        slider.adjust(toNormalizedSliderPosition: 0.35)
        capture(app, named: "call-quality-custom")
        app.buttons["Done"].tap()
        XCTAssertTrue(quality.waitForExistence(timeout: 5))
        quality.tap()
        XCTAssertTrue(slider.waitForExistence(timeout: 5), "Custom quality should remain selected")
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

    #endif

    func testCaptureMarketingScreenshots() {
        let app = launchFixtureApp(createAccount: true, marketing: true)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        XCTAssertTrue(app.descendants(matching: .any)["chatRow-fx-chat-1"].waitForExistence(timeout: 10))
        sleep(2)
        #if os(iOS)
        capture(app, named: "marketing-chat-list")
        #else
        openFixtureChat(app, index: 0)
        XCTAssertTrue(app.descendants(matching: .any)["chatMessage-fx-chat-1-msg-6"].waitForExistence(timeout: 10))
        sleep(2)
        let attachment = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        attachment.name = "screenshot-marketing-desktop"
        attachment.lifetime = .keepAlways
        add(attachment)
        #endif
    }

    // MARK: - Helpers

    private func launchFixtureApp(createAccount: Bool, marketing: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "screenshot-\(UUID().uuidString)"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        if marketing {
            app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_STYLE"] = "marketing"
            let bundle = Bundle(for: Self.self)
            let url = bundle.url(forResource: "ScreenshotAvatars", withExtension: "jpg", subdirectory: "Fixtures")
                ?? bundle.url(forResource: "ScreenshotAvatars", withExtension: "jpg")
            let data = url.flatMap { try? Data(contentsOf: $0) }
            XCTAssertNotNil(data, "The marketing portraits must be bundled with the UI tests")
            app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_AVATARS"] = data?.base64EncodedString()
        }
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
