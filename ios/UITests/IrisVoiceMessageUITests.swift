#if os(iOS)
import XCTest

final class IrisVoiceMessageUITests: IrisChatUITestCase {
    override func setUpWithError() throws {
        try super.setUpWithError()
        continueAfterFailure = false
    }

    func testTypingReplacesMicrophoneWithSendAndPreservesDraft() {
        let app = openVoiceChat()
        let microphone = element(app, "chatVoiceRecordButton")
        XCTAssertTrue(microphone.isHittable)
        XCTAssertFalse(element(app, "chatSendButton").exists)

        let draft = "Keep this message while typing"
        let input = editableElement(app, "chatMessageInput")
        typeText(draft, into: input, app: app)

        XCTAssertTrue(element(app, "chatSendButton").waitForExistence(timeout: 5))
        XCTAssertTrue(waitUntil(timeout: 5) { !microphone.exists })
        XCTAssertEqual(input.value as? String, draft)
        XCTAssertFalse(element(app, "chatVoiceRecording").exists)
        XCTAssertFalse(element(app, "chatVoiceDeleteButton").exists)
        XCTAssertFalse(element(app, "chatAudioPlayButton").exists)

        element(app, "chatTimeline")
            .coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
            .tap()
        XCTAssertEqual(input.value as? String, draft)
        XCTAssertFalse(microphone.exists)
    }

    func testHoldAndSlideUpLocksThenStopPreviewsAndDeleteDiscards() {
        let app = openVoiceChat()
        dragMicrophone(app, by: CGVector(dx: 0, dy: -140), holdDuration: 4)

        let recording = element(app, "chatVoiceRecording")
        XCTAssertTrue(recording.waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "chatVoiceDuration").exists)
        let stop = element(app, "chatVoiceStopButton")
        XCTAssertTrue(stop.waitForExistence(timeout: 5))
        XCTAssertTrue(stop.isHittable)
        XCTAssertTrue(element(app, "chatVoiceCancelButton").isHittable)
        XCTAssertTrue(element(app, "chatVoiceSendButton").isHittable)
        capture(app, named: "voice-recording-locked")

        stop.tap()

        // SwiftUI can merge the preview container into the composer in the
        // accessibility tree; its controls provide stable preview evidence.
        let delete = element(app, "chatVoiceDeleteButton")
        let playPause = element(app, "chatAudioPlayButton")
        XCTAssertTrue(delete.waitForExistence(timeout: 10))
        XCTAssertTrue(playPause.waitForExistence(timeout: 5))
        XCTAssertTrue(waitUntil(timeout: 5) { !recording.exists })
        XCTAssertTrue(element(app, "chatVoiceSendButton").isHittable)
        XCTAssertTrue(delete.isHittable)
        XCTAssertEqual(playPause.label, "Play audio", "Preview must not autoplay")
        capture(app, named: "voice-recording-preview")

        playPause.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { playPause.label == "Pause audio" })
        playPause.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { playPause.label == "Play audio" })

        delete.tap()
        assertEmptyVoiceComposer(app)
    }

    func testHoldAndSlideLeftCancelsWithoutStagingAnAttachment() {
        let app = openVoiceChat()
        dragMicrophone(app, by: CGVector(dx: -140, dy: 0))

        assertEmptyVoiceComposer(app)
        XCTAssertFalse(element(app, "chatSelectedAttachments").exists)
        XCTAssertFalse(element(app, "chatAttachmentLoading").exists)
        XCTAssertFalse(app.staticTexts["Uploading"].exists)
    }

    func testShortTapDoesNotStartRecording() {
        let app = openVoiceChat()
        element(app, "chatVoiceRecordButton").tap()

        // Wait through the hold threshold to catch a delayed start after release.
        XCTAssertFalse(element(app, "chatVoiceRecording").waitForExistence(timeout: 1))
        assertEmptyVoiceComposer(app)
    }

    private func openVoiceChat() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "voice-message-\(UUID().uuidString)"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_DEMO_RELAYS"] = "ws://127.0.0.1:9"
        // Only the microphone input is replaced; gestures and draft UI use
        // the production recording flow without recording ambient audio.
        app.launchEnvironment["IRIS_UI_TEST_VOICE_RECORDING"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        createAccount(app)
        openSelfOnlyGroup(app)
        XCTAssertTrue(element(app, "chatVoiceRecordButton").waitForExistence(timeout: 10))
        return app
    }

    private func dragMicrophone(
        _ app: XCUIApplication,
        by offset: CGVector,
        holdDuration: TimeInterval = 1.4
    ) {
        let microphone = element(app, "chatVoiceRecordButton")
        XCTAssertTrue(waitUntil(timeout: 5) { microphone.isHittable })
        let start = microphone.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        // Exceed the minimum valid duration before dragging, so Stop can
        // produce a preview rather than discard an accidental short tap.
        start.press(forDuration: holdDuration, thenDragTo: start.withOffset(offset))
    }

    private func assertEmptyVoiceComposer(
        _ app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(element(app, "chatVoiceRecordButton").waitForExistence(timeout: 5), file: file, line: line)
        XCTAssertTrue(waitUntil(timeout: 5) {
            !element(app, "chatVoiceRecording").exists && !element(app, "chatVoiceDeleteButton").exists
        }, file: file, line: line)
        XCTAssertFalse(element(app, "chatAudioPlayButton").exists, file: file, line: line)
        XCTAssertEqual(editableElement(app, "chatMessageInput").value as? String, "", file: file, line: line)
        XCTAssertFalse(element(app, "chatVoiceSendButton").exists, file: file, line: line)
        XCTAssertFalse(element(app, "chatSendButton").exists, file: file, line: line)
        XCTAssertFalse(element(app, "chatSelectedAttachments").exists, file: file, line: line)
    }

    private func capture(_ app: XCUIApplication, named name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
