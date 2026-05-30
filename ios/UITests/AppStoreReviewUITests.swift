import XCTest

final class AppStoreReviewUITests: XCTestCase {
    private let phaseKey = "IRIS_UI_TEST_APP_STORE_REVIEW_PHASE"
    private let runIDKey = "IRIS_UI_TEST_APP_STORE_REVIEW_RUN_ID"
    private let messageKey = "IRIS_UI_TEST_APP_STORE_REVIEW_MESSAGE"

    func testIncomingMessageRequestCanBeBlockedAndReported() throws {
#if os(macOS)
        throw XCTSkip("App Store review flow is iOS-only")
#else
        let env = ProcessInfo.processInfo.environment
        guard let phase = env[phaseKey]?.trimmingCharacters(in: .whitespacesAndNewlines),
              !phase.isEmpty else {
            throw XCTSkip("Set \(phaseKey) to create_profile or block_report")
        }
        let runID = env[runIDKey]?.trimmingCharacters(in: .whitespacesAndNewlines)
            ?? "app-store-review-\(UUID().uuidString)"
        let message = env[messageKey]?.trimmingCharacters(in: .whitespacesAndNewlines)
            ?? "app-store-review-\(UUID().uuidString)"

        switch phase {
        case "create_profile":
            let app = launchReviewApp(runID: runID, reset: true)
            createProfile(app, name: "Review Tester")
            XCTAssertTrue(waitForChatList(app, timeout: 20))
            app.terminate()
        case "block_report":
            let app = launchReviewApp(runID: runID, reset: false)
            openIncomingMessageRequest(app, message: message)
            assertMessageRequestDeclineActionsReachable(app)
            blockIncomingRequest(app)
        default:
            XCTFail("Unknown \(phaseKey): \(phase)")
        }
#endif
    }

    private func launchReviewApp(runID: String, reset: Bool) -> XCUIApplication {
        let app = XCUIApplication()
        if reset {
            app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        }
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = runID
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        return app
    }

    private func createProfile(_ app: XCUIApplication, name: String) {
        tapWelcomeAction(app, "welcomeCreateAction")
        XCTAssertTrue(element(app, "createAccountScreen").waitForExistence(timeout: 15))

        let nameField = element(app, "signupNameField")
        XCTAssertTrue(nameField.waitForExistence(timeout: 15))
        nameField.tap()
        nameField.typeText(name)

        let createButton = element(app, "generateKeyButton")
        XCTAssertTrue(createButton.waitForExistence(timeout: 10))
        if !createButton.isEnabled {
            let terms = element(app, "onboardingTermsAgreementToggle")
            XCTAssertTrue(terms.waitForExistence(timeout: 10))
            terms.tap()
        }
        XCTAssertTrue(waitUntil(timeout: 5) { createButton.isEnabled })
        createButton.tap()
    }

    private func tapWelcomeAction(_ app: XCUIApplication, _ identifier: String) {
        let action = element(app, identifier)
        XCTAssertTrue(action.waitForExistence(timeout: 15))
        if !action.isEnabled {
            let terms = element(app, "onboardingTermsAgreementToggle")
            XCTAssertTrue(terms.waitForExistence(timeout: 10))
            terms.tap()
        }
        XCTAssertTrue(action.isEnabled)
        action.tap()
    }

    private func openIncomingMessageRequest(_ app: XCUIApplication, message: String) {
        if element(app, "messageRequestBar").waitForExistence(timeout: 5) {
            XCTAssertTrue(app.staticTexts[message].firstMatch.waitForExistence(timeout: 20))
            return
        }

        XCTAssertTrue(waitForChatList(app, timeout: 30))
        let row = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'chatRow-'"))
            .firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 120), "incoming DM row did not appear")
        row.tap()

        XCTAssertTrue(element(app, "messageRequestBar").waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts[message].firstMatch.waitForExistence(timeout: 20))
        XCTAssertTrue(messageRequestDeclineAction(app).waitForExistence(timeout: 5))
    }

    private func blockIncomingRequest(_ app: XCUIApplication) {
        var blockAction = reportDialogButton(app, "messageRequestDeclineBlockButton", fallbackLabel: "Block")
        if !blockAction.waitForExistence(timeout: 1) {
            openMessageRequestDeclineActions(app)
            blockAction = reportDialogButton(app, "messageRequestDeclineBlockButton", fallbackLabel: "Block")
        }
        XCTAssertTrue(blockAction.waitForExistence(timeout: 5))
        blockAction.tap()

        XCTAssertTrue(reportDialogButton(app, "messageRequestBlockAndReportButton", fallbackLabel: "Report and block").waitForExistence(timeout: 5))
        let identifiedConfirm = app.buttons["messageRequestBlockConfirmKeep"].firstMatch
        if identifiedConfirm.waitForExistence(timeout: 5) {
            identifiedConfirm.tap()
        } else {
            let fallbackConfirm = app.buttons["Block"].firstMatch
            XCTAssertTrue(fallbackConfirm.waitForExistence(timeout: 5))
            fallbackConfirm.tap()
        }

        XCTAssertTrue(element(app, "blockedComposerBar").waitForExistence(timeout: 10))
        let backButton = element(app, "navigationBackButton")
        XCTAssertTrue(backButton.waitForExistence(timeout: 5))
        backButton.tap()
        XCTAssertTrue(waitForChatList(app, timeout: 10))
        XCTAssertTrue(waitForNoChatRows(app, timeout: 15), "blocked request stayed in the chat list")
    }

    private func messageRequestDeclineAction(_ app: XCUIApplication) -> XCUIElement {
        let identified = element(app, "messageRequestDeclineButton")
        if identified.exists {
            return identified
        }
        return app.buttons["Decline"].firstMatch
    }

    private func openMessageRequestDeclineActions(_ app: XCUIApplication) {
        let declineButton = messageRequestDeclineAction(app)
        XCTAssertTrue(declineButton.waitForExistence(timeout: 5))
        declineButton.tap()
    }

    private func assertMessageRequestDeclineActionsReachable(_ app: XCUIApplication) {
        openMessageRequestDeclineActions(app)

        XCTAssertTrue(reportDialogButton(app, "messageRequestDeclineBlockButton", fallbackLabel: "Block").waitForExistence(timeout: 5))
        let reportAction = reportDialogButton(app, "messageRequestDeclineReportButton", fallbackLabel: "Report")
        XCTAssertTrue(reportAction.waitForExistence(timeout: 5))
        XCTAssertTrue(reportDialogButton(app, "messageRequestDeclineDeleteButton", fallbackLabel: "Delete chat").waitForExistence(timeout: 5))
    }

    private func waitForChatList(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        waitForAnyElement(
            app,
            identifiers: ["chatListNewChatButton", "desktopNewChatRow"],
            timeout: timeout
        ) != nil
    }

    private func waitForAnyElement(
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

    private func reportDialogButton(
        _ app: XCUIApplication,
        _ identifier: String,
        fallbackLabel: String
    ) -> XCUIElement {
        let identified = app.buttons[identifier].firstMatch
        if identified.exists {
            return identified
        }
        return app.buttons[fallbackLabel].firstMatch
    }

    private func waitUntil(timeout: TimeInterval, condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if condition() {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return condition()
    }

    private func waitForNoChatRows(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            let row = app.descendants(matching: .any)
                .matching(NSPredicate(format: "identifier BEGINSWITH 'chatRow-'"))
                .firstMatch
            if !row.exists {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return false
    }
    private func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }
}
