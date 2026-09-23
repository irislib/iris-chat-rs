import XCTest

#if os(iOS)
/// Explicit opt-in only: operates the established chat on the selected test
/// phone, preserving its account. The paired phone places the incoming call.
final class PhoneCallUITests: XCTestCase {
    func testOutgoingCallToPairedPhone() throws {
        guard ProcessInfo.processInfo.environment["IRIS_PHONE_CALL_E2E"] == "1" else {
            throw XCTSkip("Requires the explicitly selected paired test phones")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        if app.buttons["Done"].exists { app.buttons["Done"].tap() }
        let start = app.buttons["startVideoCallButton"]
        XCTAssertTrue(start.waitForExistence(timeout: 10))
        print("PHONE_CALL_OUTGOING_READY")
        start.tap()
        let connected = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "[0-9]+:[0-9]{2}")).firstMatch
        XCTAssertTrue(connected.waitForExistence(timeout: 40))
        print("PHONE_CALL_CONNECTED")
        let connectedAt = Date()
        let stable = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            Date().timeIntervalSince(connectedAt) >= 20 && connected.exists && !app.buttons["Done"].exists
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [stable], timeout: 24), .completed)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "outgoing-video-connected"; screenshot.lifetime = .keepAlways; add(screenshot)
        app.buttons.matching(NSPredicate(format: "label == 'End'")).firstMatch.tap()
        XCTAssertTrue(app.buttons["Done"].waitForExistence(timeout: 10))
        print("PHONE_CALL_ENDED")
    }

    func testIncomingSystemCallConnectsAndDismisses() throws {
        guard ProcessInfo.processInfo.environment["IRIS_PHONE_CALL_E2E"] == "1" else {
            throw XCTSkip("Requires the explicitly selected paired test phones")
        }
        let app = XCUIApplication()
        continueAfterFailure = false
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        print("PHONE_CALL_READY")
        let answer = app.buttons.matching(NSPredicate(format: "label == 'Answer'")).firstMatch
        XCTAssertTrue(answer.waitForExistence(timeout: 60), "The paired phone must place a call")
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "incoming-system-call"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let system = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let callUI = XCUIApplication(bundleIdentifier: "com.apple.InCallService")
        print("PHONE_SYSTEM_UI: \(system.debugDescription)\n\(callUI.debugDescription)")
        let nativeAnswer = callUI.buttons["Answer call"].firstMatch
        XCTAssertTrue(nativeAnswer.waitForExistence(timeout: 5))
        nativeAnswer.tap()
        let end = app.buttons.matching(NSPredicate(format: "label == 'End'")).firstMatch
        XCTAssertTrue(end.waitForExistence(timeout: 15))
        let connected = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "[0-9]+:[0-9]{2}")).firstMatch
        XCTAssertTrue(connected.waitForExistence(timeout: 15), "Call media must become ready")
        print("PHONE_CALL_CONNECTED")
        let connectedAt = Date()
        let stable = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            Date().timeIntervalSince(connectedAt) >= 8 && connected.exists && !app.buttons["Done"].exists
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [stable], timeout: 12), .completed)
        let active = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        active.name = "connected-phone-call"; active.lifetime = .keepAlways; add(active)
        end.tap()
        XCTAssertTrue(app.buttons["Done"].waitForExistence(timeout: 10))
        print("PHONE_CALL_ENDED")
    }

    func testCallerCancellationDismissesIncomingSystemCall() throws {
        guard ProcessInfo.processInfo.environment["IRIS_PHONE_CALL_E2E"] == "1" else {
            throw XCTSkip("Requires the explicitly selected paired test phones")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.activate()
        print("PHONE_CALL_READY")
        let system = XCUIApplication(bundleIdentifier: "com.apple.InCallService")
        let answer = system.buttons["Answer call"].firstMatch
        XCTAssertTrue(answer.waitForExistence(timeout: 60))
        print("PHONE_CALL_INCOMING_VISIBLE")
        XCTAssertTrue(app.buttons["Done"].waitForExistence(timeout: 15))
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !answer.exists }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "canceled-system-call-dismissed"; screenshot.lifetime = .keepAlways; add(screenshot)
        print("PHONE_CALL_ENDED")
    }
}
#endif
