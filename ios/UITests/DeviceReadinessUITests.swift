import XCTest

#if os(iOS)
final class DeviceReadinessUITests: XCTestCase {
    func testDismissPostRestartSIMSheet() {
        continueAfterFailure = false
        let system = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let title = system.staticTexts["Enter your SIM PIN"]
        attachScreen("sim-sheet-before")
        let hierarchy = XCTAttachment(string: system.debugDescription)
        hierarchy.name = "sim-sheet-system-hierarchy"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
        XCTAssertTrue(title.waitForExistence(timeout: 5), "Expected the observed SIM PIN sheet; no action taken")

        // Cancel only this known post-restart sheet. Never enter a PIN, accept
        // a permission, or dismiss an unrelated alert to make a test pass.
        let cancel = system.buttons.matching(
            NSPredicate(format: "label == %@ OR label == %@", "Cancel", "Close")
        ).firstMatch
        XCTAssertTrue(cancel.exists && cancel.isHittable,
                      "The SIM sheet must expose an explicit Cancel or Close button")
        cancel.tap()
        let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: title)
        XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 5), .completed)
        attachScreen("sim-sheet-after")
        print("IRIS_SIM_SHEET_DISMISSED no_pin_entered=true")
    }

    private func attachScreen(_ name: String) {
        let image = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        image.name = name
        image.lifetime = .keepAlways
        add(image)
    }
}
#endif
