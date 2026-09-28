#if os(macOS)
import AVFoundation
import XCTest

final class IrisAudioMessageUITests: IrisChatUITestCase {
    func testIncomingAndOutgoingVoiceMessagesPlayInsideChat() throws {
        continueAfterFailure = false
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("audio-ui-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let cache = directory.appendingPathComponent("attachments/downloaded")
        try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
        try writeSilentVoiceMessage(to: cache.appendingPathComponent("fixture-voice-Voice message.m4a"))

        let app = XCUIApplication()
        defer { app.terminate() }
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = directory.lastPathComponent
        app.launchEnvironment["IRIS_UI_TEST_DATA_DIR"] = directory.path
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_DEMO_RELAYS"] = "ws://127.0.0.1:9"
        app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_AUDIO_MESSAGES"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        submitWelcomeName(app, name: "Alex Rivera", assertFocus: false)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        element(app, "chatRow-fx-chat-1").tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15))

        // Message-row accessibility identifiers can propagate to children,
        // so locate controls by their user-facing accessibility labels.
        let playButtons = app.buttons.matching(NSPredicate(format: "label == 'Play audio'"))
        XCTAssertTrue(waitUntil(timeout: 10) { playButtons.count == 2 })
        let incomingPlay = playButtons.element(boundBy: 0)
        let outgoingPlay = playButtons.element(boundBy: 1)
        incomingPlay.tap()
        let pauseButtons = app.buttons.matching(NSPredicate(format: "label == 'Pause audio'"))
        XCTAssertTrue(waitUntil(timeout: 10) { pauseButtons.count == 1 })
        XCTAssertEqual(app.state, .runningForeground, "Audio must stay in Iris Chat")
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "value CONTAINS '/ 1:00'")).firstMatch.waitForExistence(timeout: 5))

        let progress = app.sliders.matching(NSPredicate(format: "label == 'Audio position'")).element(boundBy: 0)
        XCTAssertTrue(waitUntil(timeout: 5) { (Double("\(progress.value ?? "")") ?? 0) > 0 },
                      "The AVPlayer timeline must actually advance")
        let speed = app.buttons.matching(NSPredicate(format: "label == 'Playback speed'")).element(boundBy: 0)
        speed.tap()
        XCTAssertTrue(waitUntil(timeout: 3) { speed.value as? String == "1.5×" })
        pauseButtons.firstMatch.tap()
        XCTAssertTrue(waitUntil(timeout: 3) { playButtons.count == 2 })
        progress.adjust(toNormalizedSliderPosition: 0.5)
        XCTAssertTrue(waitUntil(timeout: 3) {
            abs((Double("\(progress.value ?? "")") ?? -1) - 30) < 1
        })
        incomingPlay.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { pauseButtons.count == 1 })
        // The remaining Play control now belongs to the outgoing message.
        playButtons.firstMatch.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { pauseButtons.count == 1 && playButtons.count == 1 })
        XCTAssertGreaterThan(pauseButtons.firstMatch.frame.minX, incomingPlay.frame.minX)
        pauseButtons.firstMatch.tap()
        XCTAssertTrue(waitUntil(timeout: 3) { playButtons.count == 2 })
        XCTAssertTrue(outgoingPlay.exists)

        let screenshot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        screenshot.name = "macos-inline-voice-messages"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    private func writeSilentVoiceMessage(to url: URL) throws {
        let file = try AVAudioFile(forWriting: url, settings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100,
            AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 64_000,
        ])
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 44_100))
        let channel = try XCTUnwrap(buffer.floatChannelData?[0])
        channel.initialize(repeating: 0, count: 44_100)
        buffer.frameLength = 44_100
        for second in 0..<60 {
            for frame in 0..<44_100 {
                let t = Double(second) + Double(frame) / 44_100
                let amplitude = second % 9 < 2 ? 0 : 0.1 + 0.6 * pow(sin(t * 0.6), 2)
                channel[frame] = Float(sin(t * 440 * 2 * .pi) * amplitude)
            }
            try file.write(from: buffer)
        }
    }
}
#endif
