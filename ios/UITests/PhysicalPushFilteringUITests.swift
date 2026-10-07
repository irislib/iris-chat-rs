#if os(iOS)
import XCTest

final class PhysicalPushFilteringUITests: XCTestCase {
    func testBackgroundPhysicalPush() throws {
        guard let body = ProcessInfo.processInfo.environment["IRIS_PHYSICAL_PUSH_VISIBLE_BODY"] else {
            throw XCTSkip("Requires an explicitly selected physical APNs test")
        }
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))
        // Activating Settings reliably leaves the app on physical phones;
        // the synthetic Home button is not effective on every iOS version.
        let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        settings.activate()
        XCTAssertTrue(settings.wait(for: .runningForeground, timeout: 10))
        let background = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                [.runningBackground, .runningBackgroundSuspended, .notRunning].contains(app.state)
            }, object: app
        )
        guard XCTWaiter.wait(for: [background], timeout: 10) == .completed else {
            return XCTFail("The app must leave the foreground before sending APNs (state \(app.state.rawValue))")
        }
        print("PHYSICAL_PUSH_BACKGROUND_READY state=\(app.state.rawValue)")
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        XCTAssertTrue(springboard.staticTexts[body].firstMatch.waitForExistence(timeout: 120),
                      "The positive-control message must appear through real APNs")
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "physical-push-message-visible"
        shot.lifetime = .keepAlways
        add(shot)
    }
}
#endif
