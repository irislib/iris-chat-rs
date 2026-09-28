#if os(iOS)
import XCTest

final class IosReviewDemoUITests: IrisChatUITestCase {
    func testAudioShowsWaveformAndDurationBeforePlaying() {
        continueAfterFailure = false
        let app = launchCleanApp()
        submitWelcomeName(app, name: "AppStoreDemoUserMode")
        XCTAssertTrue(waitForChatList(app, timeout: 45))
        let sample = app.staticTexts["Sample conversation"].firstMatch
        XCTAssertTrue(sample.waitForExistence(timeout: 45))
        XCTAssertTrue(waitUntil(timeout: 45) { !app.staticTexts["Preparing sample messages…"].exists })
        sample.tap()
        let play = app.buttons["chatAudioPlayButton"].firstMatch
        XCTAssertTrue(play.waitForExistence(timeout: 15))
        captureDemo(app, name: "audio-before-first-play")
        let duration = app.staticTexts["chatAudioDuration"].firstMatch
        XCTAssertTrue(waitUntil(timeout: 10) { duration.label.contains("0:06") },
                      "The downloaded clip must show its duration before Play")
        XCTAssertEqual(play.label, "Play audio")
        let position = app.sliders["chatAudioProgress"].firstMatch
        XCTAssertTrue(position.isEnabled, "The waveform must be seekable before playback")
        position.adjust(toNormalizedSliderPosition: 0.5)
        XCTAssertTrue(waitUntil(timeout: 3) { (position.value as? String ?? "").contains("3 of 6") })
        XCTAssertEqual(play.label, "Play audio", "Seeking must not start playback")
        XCTAssertFalse(element(app, "chatReplyComposer").exists, "Scrubbing must not swipe the message into reply")
        position.adjust(toNormalizedSliderPosition: 0.1)
        XCTAssertFalse(element(app, "messageInfoSheet").exists, "Scrubbing backwards must not open message details")
        captureDemo(app, name: "audio-ready-before-first-play")

        let caption = app.staticTexts["Play this sample audio. You can pause, seek, and change playback speed."].firstMatch
        dragHorizontally(caption, from: 0.15, to: 0.98)
        XCTAssertTrue(element(app, "chatReplyComposer").waitForExistence(timeout: 5),
                      "Swiping the message outside the player must still open reply")
        app.buttons["Close"].firstMatch.tap()
        XCTAssertTrue(waitUntil(timeout: 3) { !element(app, "chatReplyComposer").exists })
        dragHorizontally(caption, from: 0.85, to: 0.02)
        XCTAssertTrue(element(app, "messageInfoSheet").waitForExistence(timeout: 5),
                      "Swiping the message outside the player must still open details")
    }

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
