#if os(iOS)
import XCTest

final class IosReviewDemoUITests: IrisChatUITestCase {
    func testDemoOnboardingAudioAndPersistenceOnIPad() {
        let runID = "review-demo-\(UUID().uuidString)"
        let app = launchCleanApp(runId: runID)
        tapWelcomeAction(app, "welcomeCreateAction")
        let name = editableElement(app, "signupNameField")
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        typeText("AppStoreDemoUserMode", into: name, app: app)
        XCTAssertTrue(element(app, "reviewDemoExplanation").exists)
        acceptOnboardingTermsIfNeeded(app)
        element(app, "generateKeyButton").tap()
        XCTAssertTrue(waitForChatList(app, timeout: 45))
        let sample = app.staticTexts["Sample conversation"].firstMatch
        XCTAssertTrue(sample.waitForExistence(timeout: 45), app.debugDescription)
        XCTAssertTrue(waitUntil(timeout: 45) { !app.staticTexts["Preparing sample messages…"].exists })
        XCTAssertFalse(app.staticTexts["Demo setup failed. Tap Retry."].exists)
        sample.tap()
        let play = app.buttons["chatAudioPlayButton"].firstMatch
        XCTAssertTrue(play.waitForExistence(timeout: 15), app.debugDescription)
        XCTAssertTrue(element(app, "reviewDemoBanner").exists)
        captureDemo(app, name: "ios-review-demo-ipad")
        play.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { play.label == "Pause audio" })
        play.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { play.label == "Play audio" })
        let microphone = element(app, "chatVoiceRecordButton")
        XCTAssertTrue(microphone.exists)
        XCTAssertTrue(element(app, "startVoiceCallButton").exists)
        app.terminate()
        let restored = launchApp(runId: runID)
        XCTAssertTrue(restored.buttons["chatAudioPlayButton"].firstMatch.waitForExistence(timeout: 30))
        XCTAssertTrue(element(restored, "reviewDemoBanner").waitForExistence(timeout: 10))
        XCTAssertTrue(restored.staticTexts["Sample conversation"].firstMatch.waitForExistence(timeout: 10))
        XCTAssertFalse(restored.staticTexts["Preparing sample messages…"].exists)
        captureDemo(restored, name: "ios-review-demo-restored")
        element(restored, "navigationBackButton").tap()
        XCTAssertTrue(waitForChatList(restored, timeout: 10))
        XCTAssertTrue(restored.staticTexts["Sample group"].exists)
    }

    private func captureDemo(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
