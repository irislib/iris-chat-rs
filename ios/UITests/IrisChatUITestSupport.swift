import XCTest
#if os(macOS)
import AppKit
#endif

extension IrisChatUITests {
    func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }

    func typeText(_ text: String, into target: XCUIElement, app: XCUIApplication) {
#if os(macOS)
        app.activate()
        focusTextTarget(target, app: app)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        app.typeKey("v", modifierFlags: .command)
#else
        target.tap()
        if target.elementType == .textView {
            for character in text {
                target.typeText(String(character))
            }
        } else {
            target.typeText(text)
        }
#endif
    }

    func editableElement(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
#if os(macOS)
        concreteEditableElement(app, identifier: identifier) ?? element(app, identifier)
#else
        element(app, identifier)
#endif
    }

    func concreteEditableElement(_ app: XCUIApplication, identifier: String) -> XCUIElement? {
        for query in [app.textViews, app.textFields, app.secureTextFields] {
            let candidate = query.matching(identifier: identifier).firstMatch
            if candidate.exists {
                return candidate
            }
        }
        return nil
    }

    func focusTextTarget(_ target: XCUIElement, app: XCUIApplication) {
#if os(macOS)
        if target.identifier == "chatMessageInput" {
            let composer = element(app, "chatComposerBar")
            if composer.exists {
                composer.coordinate(withNormalizedOffset: CGVector(dx: 0.55, dy: 0.5)).tap()
                return
            }
        }
#endif
        target.tap()
    }

    func ensureMacWindowVisible(_ app: XCUIApplication) {
#if os(macOS)
        app.activate()
        if !app.windows.firstMatch.waitForExistence(timeout: 2) {
            app.typeKey("n", modifierFlags: .command)
            _ = app.windows.firstMatch.waitForExistence(timeout: 5)
        }
#endif
    }

    func assertKeyboardFocused(
        _ target: XCUIElement,
        timeout: TimeInterval = 5,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
#if os(macOS)
        let predicate = NSPredicate(format: "hasKeyboardFocus == true")
        let expectation = expectation(for: predicate, evaluatedWith: target)
        let result = XCTWaiter.wait(for: [expectation], timeout: timeout)
        XCTAssertEqual(result, .completed, "field did not autofocus", file: file, line: line)
#endif
    }

    func launchCleanApp(
        runId: String = UUID().uuidString,
        qrValue: String? = nil,
        profilePicturePath: String? = nil,
        seedPeer: String? = nil,
        seedCount: Int? = nil,
        seedDaySplitIndex: Int? = nil
    ) -> XCUIApplication {
        launchApp(
            runId: runId,
            reset: true,
            qrValue: qrValue,
            profilePicturePath: profilePicturePath,
            seedPeer: seedPeer,
            seedCount: seedCount,
            seedDaySplitIndex: seedDaySplitIndex
        )
    }

    func launchApp(
        runId: String,
        reset: Bool = false,
        qrValue: String? = nil,
        profilePicturePath: String? = nil,
        seedPeer: String? = nil,
        seedCount: Int? = nil,
        seedDaySplitIndex: Int? = nil
    ) -> XCUIApplication {
        let app = XCUIApplication()
        if reset {
            app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        }
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = runId
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        if let qrValue {
            app.launchEnvironment["IRIS_QR_TEST_VALUE"] = qrValue
        }
        if let profilePicturePath {
            app.launchEnvironment["IRIS_UI_TEST_PROFILE_PICTURE_PATH"] = profilePicturePath
        }
        if let seedPeer, let seedCount {
            app.launchEnvironment["IRIS_UI_TEST_SEED_PEER"] = seedPeer
            app.launchEnvironment["IRIS_UI_TEST_SEED_COUNT"] = String(seedCount)
        }
        if let seedDaySplitIndex {
            app.launchEnvironment["IRIS_UI_TEST_SEED_DAY_SPLIT_INDEX"] = String(seedDaySplitIndex)
        }
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        ensureMacWindowVisible(app)
        return app
    }

    func dismissNotificationPromptIfPresent(app: XCUIApplication) {
#if os(iOS)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let denyButtons = [
            springboard.buttons["Don’t Allow"],
            springboard.buttons["Don't Allow"],
            springboard.descendants(matching: .button)
                .matching(NSPredicate(format: "label == %@ OR label == %@", "Don’t Allow", "Don't Allow"))
                .firstMatch
        ]
        for denyButton in denyButtons {
            if denyButton.waitForExistence(timeout: 1) {
                denyButton.tap()
                XCTAssertTrue(app.wait(for: .runningForeground, timeout: 5))
                return
            }
        }
        if springboard.wait(for: .runningForeground, timeout: 1) {
            springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.31, dy: 0.67)).tap()
            XCTAssertTrue(app.wait(for: .runningForeground, timeout: 5))
        }
#endif
    }

    func createAccount(_ app: XCUIApplication) {
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 20), "chat list never appeared after account creation")
    }

    func submitWelcomeName(
        _ app: XCUIApplication,
        name: String = "ios tester",
        assertFocus: Bool = true,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        tapWelcomeAction(app, "welcomeCreateAction", file: file, line: line)
        XCTAssertTrue(element(app, "createAccountScreen").waitForExistence(timeout: 15), file: file, line: line)
#if os(macOS)
        XCTAssertFalse(element(app, "onboardingTermsAgreementToggle").exists, file: file, line: line)
        XCTAssertFalse(element(app, "onboardingTermsNotice").exists, file: file, line: line)
#endif
        let nameField = editableElement(app, "signupNameField")
        XCTAssertTrue(nameField.waitForExistence(timeout: 15), file: file, line: line)
        XCTAssertTrue(nameField.isEnabled, file: file, line: line)
        nameField.tap()
        if assertFocus {
            assertKeyboardFocused(nameField)
        }
        typeText(name, into: nameField, app: app)
        let action = element(app, "generateKeyButton")
        XCTAssertTrue(action.waitForExistence(timeout: 10), file: file, line: line)
        if !action.isEnabled {
            acceptOnboardingTermsIfNeeded(app, file: file, line: line)
        }
        XCTAssertTrue(waitUntil(timeout: 5) { action.isEnabled }, file: file, line: line)
        XCTAssertTrue(action.isEnabled, file: file, line: line)
        action.tap()
    }
    func tapWelcomeAction(
        _ app: XCUIApplication,
        _ identifier: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let action = element(app, identifier)
        XCTAssertTrue(action.waitForExistence(timeout: 15), file: file, line: line)
        if !action.isEnabled {
            acceptOnboardingTermsIfNeeded(app, file: file, line: line)
        }
        XCTAssertTrue(action.isEnabled, file: file, line: line)
        action.tap()
    }
    func acceptOnboardingTermsIfNeeded(
        _ app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let toggle = element(app, "onboardingTermsAgreementToggle")
        XCTAssertTrue(toggle.waitForExistence(timeout: 10), file: file, line: line)
        toggle.tap()
    }
    func waitForChatList(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        waitForAnyElement(app, identifiers: ["chatListNewChatButton", "desktopNewChatRow"], timeout: timeout) != nil
    }
    func seededChatRowPreview(_ app: XCUIApplication) -> XCUIElement {
        let predicate = NSPredicate(format: "identifier BEGINSWITH 'chatRow-'")
        let cell = app.cells.matching(predicate).firstMatch
#if os(macOS)
        return cell.exists ? cell : app.buttons.matching(predicate).firstMatch
#else
        return cell
#endif
    }
    func openSeededChat(
        _ app: XCUIApplication,
        rowTimeout: TimeInterval = 45,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        if element(app, "chatMessageInput").exists { return }
        let deadline = Date().addingTimeInterval(rowTimeout)
        var sawRow = false
        repeat {
            let row = seededChatRowPreview(app)
            if row.waitForExistence(timeout: min(5, max(0.1, deadline.timeIntervalSinceNow))) {
                sawRow = true
                row.tap()
                if element(app, "chatMessageInput").waitForExistence(timeout: 2) { return }
            }
            _ = waitForChatList(app, timeout: 1)
        } while Date() < deadline
        XCTAssertTrue(sawRow, "seeded chat row never appeared", file: file, line: line)
        XCTAssertTrue(element(app, "chatMessageInput").exists, "seeded chat did not open", file: file, line: line)
    }
    func inlineDaySeparator(_ app: XCUIApplication, label: String) -> XCUIElement {
        app.descendants(matching: .any).matching(
            NSPredicate(
                format: "identifier BEGINSWITH 'chatInlineDaySeparator-' AND label == %@",
                label
            )
        ).firstMatch
    }
    func assertOnboardingScreenUsesHeaderBack(
        _ app: XCUIApplication,
        actionIdentifier: String,
        screenIdentifier: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        tapWelcomeAction(app, actionIdentifier, file: file, line: line)
        XCTAssertTrue(element(app, screenIdentifier).waitForExistence(timeout: 10), file: file, line: line)
        XCTAssertTrue(element(app, "navigationBackButton").waitForExistence(timeout: 5), file: file, line: line)
        XCTAssertFalse(element(app, "onboardingBackButton").exists, file: file, line: line)
        element(app, "navigationBackButton").tap()
        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 10), file: file, line: line)
    }
    func tapNewChat(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        guard let newChat = waitForAnyElement(
            app,
            identifiers: ["chatListNewChatButton", "desktopNewChatRow"],
            timeout: 10
        ) else {
            XCTFail("New chat control never appeared", file: file, line: line)
            return
        }
        if newChat.identifier == "chatListNewChatButton" {
            newChat.coordinate(withNormalizedOffset: CGVector(dx: 0.12, dy: 0.5)).tap()
        } else {
            newChat.tap()
        }
    }
    func returnToChatList(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        if let settingsCloseButton = waitForAnyElement(
            app,
            identifiers: ["settingsCloseButton", "settingsDoneButton"],
            timeout: 1
        ) {
            settingsCloseButton.tap()
            XCTAssertTrue(waitForChatList(app, timeout: 10), file: file, line: line)
            return
        }

        let backButton = element(app, "navigationBackButton")
        if backButton.exists {
            backButton.tap()
        } else if let newChat = waitForAnyElement(
            app,
            identifiers: ["desktopNewChatRow", "chatListNewChatButton"],
            timeout: 5
        ) {
            newChat.tap()
        } else {
            XCTFail("Could not return to chat list", file: file, line: line)
            return
        }
        XCTAssertTrue(waitForChatList(app, timeout: 10), file: file, line: line)
    }
    func waitForAnyElement(
        _ app: XCUIApplication,
        identifiers: [String],
        timeout: TimeInterval
    ) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for identifier in identifiers {
                let candidate = element(app, identifier)
                if candidate.exists {
                    return candidate
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return nil
    }
    func waitUntil(timeout: TimeInterval, condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if condition() {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return condition()
    }
    func dragHorizontally(_ element: XCUIElement, from startX: CGFloat, to endX: CGFloat) {
        let start = element.coordinate(withNormalizedOffset: CGVector(dx: startX, dy: 0.5))
        let end = element.coordinate(withNormalizedOffset: CGVector(dx: endX, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: end)
    }
    func dragVertically(_ element: XCUIElement, x: CGFloat, fromY: CGFloat, toY: CGFloat) {
        let start = element.coordinate(withNormalizedOffset: CGVector(dx: x, dy: fromY))
        let end = element.coordinate(withNormalizedOffset: CGVector(dx: x, dy: toY))
        start.press(forDuration: 0.05, thenDragTo: end)
    }
    func flickVertically(_ element: XCUIElement, x: CGFloat, fromY: CGFloat, toY: CGFloat) {
        let start = element.coordinate(withNormalizedOffset: CGVector(dx: x, dy: fromY))
        let end = element.coordinate(withNormalizedOffset: CGVector(dx: x, dy: toY))
        start.press(
            forDuration: 0.01,
            thenDragTo: end,
            withVelocity: XCUIGestureVelocity.fast,
            thenHoldForDuration: 0
        )
    }
    func openGroupDetails(_ app: XCUIApplication) {
        let header = element(app, "chatHeaderTitleButton")
        XCTAssertTrue(header.waitForExistence(timeout: 5))
        header.tap()
    }

    func assertNoDispatchFailureToast(
        _ app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let toast = app.staticTexts["Action failed. Copy support bundle in Settings."]
        XCTAssertFalse(toast.waitForExistence(timeout: 1), "dispatch failure toast appeared", file: file, line: line)
    }

    func openSettingsPage(
        _ app: XCUIApplication,
        _ identifier: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let row = element(app, identifier)
        XCTAssertTrue(row.waitForExistence(timeout: 10), "settings row \(identifier) did not appear", file: file, line: line)
        row.tap()
    }
}
