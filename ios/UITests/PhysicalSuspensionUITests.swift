import XCTest

final class PhysicalSuspensionUITests: IrisChatUITestCase {
    func testBackgroundSuspensionSurvivesAndResumesTheDraft() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("Suspension termination requires a physical iPhone")
        #else
        guard ProcessInfo.processInfo.environment["IRIS_PHYSICAL_SUSPENSION_TEST"] == "1" else {
            throw XCTSkip("Enable the reserved physical iPhone suspension test")
        }
        continueAfterFailure = false
        let app = launchCleanApp()
        defer { app.terminate() }
        createAccount(app)
        openSelfOnlyGroup(app)
        let input = editableElement(app, "chatMessageInput")
        let draft = "Keep this draft through suspension"
        typeText(draft, into: input, app: app)
        XCTAssertEqual(input.value as? String, draft)

        // A signed-in snapshot opens the real attachment store. This scheme
        // does not attach a debugger, which would prevent normal suspension.
        print("IRIS_SUSPENSION_READY=\(Date().timeIntervalSince1970)")
        XCUIApplication(bundleIdentifier: "com.apple.Preferences").activate()
        XCTAssertTrue(waitUntil(timeout: 45) { app.state == .runningBackgroundSuspended },
                      "The app must really suspend before the observation begins")
        let started = ProcessInfo.processInfo.systemUptime
        print("IRIS_SUSPENSION_ENTERED=\(Date().timeIntervalSince1970)")
        while ProcessInfo.processInfo.systemUptime - started < 135 {
            XCTAssertEqual(app.state, .runningBackgroundSuspended,
                           "The suspended process must remain alive")
            Thread.sleep(forTimeInterval: 1)
        }
        print("IRIS_SUSPENSION_SURVIVED=\(Date().timeIntervalSince1970)")
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        XCTAssertEqual(input.value as? String, draft)
        typeText(" and resume", into: input, app: app)
        XCTAssertEqual(input.value as? String, draft + " and resume")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "physical-suspension-resumed-draft"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        #endif
    }
}
