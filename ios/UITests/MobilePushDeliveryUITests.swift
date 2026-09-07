import XCTest

#if os(iOS)
final class MobilePushDeliveryUITests: XCTestCase {
    /// Run on an explicitly selected physical test device with an established
    /// chat. Send the expected message after PUSH_E2E_READY appears in the log.
    /// This preserves the installed account and requires the actual APNs preview.
    func testBackgroundPushPreviewOpensReceivedMessage() throws {
        guard let message = ProcessInfo.processInfo.environment["IRIS_PUSH_E2E_EXPECTED_MESSAGE"],
              !message.isEmpty else {
            throw XCTSkip("Requires an explicit physical-device push scenario")
        }
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))

        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 10))
        print("PUSH_E2E_READY")

        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let preview = springboard.staticTexts[message].firstMatch
        XCTAssertTrue(preview.waitForExistence(timeout: 90), "APNs must show the decrypted message")
        preview.tap()

        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))
        XCTAssertTrue(app.staticTexts[message].firstMatch.waitForExistence(timeout: 30),
                      "Opening the push must display the received message in the chat")
    }
}
#endif
