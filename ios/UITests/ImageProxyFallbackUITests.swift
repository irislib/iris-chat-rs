import XCTest

final class ImageProxyFallbackUITests: IrisChatUITestCase {
    func testImageProxyFallbackRequiresOptInAndPersists() {
        let runID = "image-proxy-fallback-\(UUID().uuidString)"
        var app = launchCleanApp(runId: runID)
        createAccount(app)
        openMediaSettings(app)

        let fallback = element(app, "myProfileImageProxyFallbackToggle")
        XCTAssertTrue(fallback.waitForExistence(timeout: 10))
        XCTAssertEqual(fallback.value as? String, "0")
        XCTAssertTrue(fallback.label.contains("Image hosts may see your IP address."))
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "image-proxy-fallback-default-off"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        fallback.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { fallback.value as? String == "1" })
        let proxy = element(app, "myProfileImageProxyToggle")
        proxy.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !fallback.isEnabled })
        proxy.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { fallback.isEnabled })
        app.terminate()

        app = launchApp(runId: runID)
        XCTAssertTrue(waitForChatList(app, timeout: 20))
        openMediaSettings(app)
        let restored = element(app, "myProfileImageProxyFallbackToggle")
        XCTAssertTrue(restored.waitForExistence(timeout: 10))
        XCTAssertEqual(restored.value as? String, "1")
        element(app, "myProfileResetImageProxyButton").tap()
        XCTAssertTrue(waitUntil(timeout: 5) { restored.value as? String == "0" })
    }

    private func openMediaSettings(_ app: XCUIApplication) {
        let profile = element(app, "chatListProfileButton")
        XCTAssertTrue(profile.waitForExistence(timeout: 10))
        profile.tap()
        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        openSettingsPage(app, "settingsMediaRow")
    }
}
