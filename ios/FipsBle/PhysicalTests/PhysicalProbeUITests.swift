import XCTest

final class PhysicalProbeUITests: XCTestCase {
    func testDiscoversAndroidAndExchangesBytes() throws {
        guard let token = ProcessInfo.processInfo.environment["IRIS_BLE_PROBE_TOKEN"] else {
            throw XCTSkip("Opt-in physical Android/iPhone adapter probe")
        }
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_BLE_PROBE_TOKEN"] = token
        app.launch()
        defer { app.terminate() }
        let allow = XCUIApplication(bundleIdentifier: "com.apple.springboard").alerts.buttons["Allow"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        let status = app.staticTexts["bluetoothProbeStatus"]
        let complete = expectation(for: NSPredicate(format: "label == %@", "Bluetooth exchange complete"), evaluatedWith: status)
        wait(for: [complete], timeout: 90)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.lifetime = .keepAlways
        add(screenshot)
        // Keep the radio alive until Android has consumed the reply too.
        Thread.sleep(forTimeInterval: 5)
    }
}
