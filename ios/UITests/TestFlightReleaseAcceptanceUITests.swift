import XCTest

/// Opt-in checks of the installed TestFlight app on the reserved release phone.
/// Run with the standalone UI runner: it must not install an Iris app build.
/// Attachments are private evidence and must not be published as release assets.
final class TestFlightReleaseAcceptanceUITests: XCTestCase {
    func testObserveNormalLifecycleState() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("TestFlight acceptance requires the reserved physical iPhone")
        #else
        let mode = ProcessInfo.processInfo.environment["IRIS_TESTFLIGHT_ACCEPTANCE"]
        guard mode == "background" || mode == "verify-suspended" || mode == "resume" else {
            throw XCTSkip("Enable the local lifecycle step explicitly")
        }
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "fi.siriusbusiness.irischat")
        if mode == "background" {
            // Iris must already have been launched externally without test
            // overrides. Activating another app allows ordinary suspension.
            XCTAssertEqual(app.state, .runningForeground)
            XCUIApplication(bundleIdentifier: "com.apple.Preferences").activate()
        }
        let suspended = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in app.state == .runningBackgroundSuspended }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [suspended], timeout: 45), .completed)
        print("IRIS_ACCEPTANCE_SUSPENDED=\(Date().timeIntervalSince1970)")
        if mode == "resume" {
            app.activate()
            XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
            print("IRIS_ACCEPTANCE_RESUMED=\(Date().timeIntervalSince1970)")
        }
        // This short step does not measure survival. The runner must exit
        // before the separate normal-process observation interval starts.
        #endif
    }

    func testRefreshTestFlightAvailability() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("TestFlight acceptance requires the reserved physical iPhone")
        #else
        guard ProcessInfo.processInfo.environment["IRIS_TESTFLIGHT_ACCEPTANCE"] == "refresh" else {
            throw XCTSkip("Enable the local TestFlight refresh explicitly")
        }
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "com.apple.TestFlight")
        app.activate()
        defer { capture(app, name: "testflight-refreshed-inventory") }
        if app.navigationBars["Settings"].exists {
            let close = app.buttons["Close"].firstMatch
            XCTAssertTrue(close.isHittable)
            close.tap()
        }
        let list = app.scrollViews["AppList.ScrollView"]
        XCTAssertTrue(list.waitForExistence(timeout: 15))
        let listReady = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "hittable == true"), object: list)
        XCTAssertEqual(XCTWaiter.wait(for: [listReady], timeout: 15), .completed)
        list.swipeDown()
        let ready = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in !app.activityIndicators.firstMatch.exists }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 30), .completed)
        #endif
    }

    func testPrepareOrVerifyPreservedDraft() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("TestFlight acceptance requires the reserved physical iPhone")
        #else
        let environment = ProcessInfo.processInfo.environment
        let mode = environment["IRIS_TESTFLIGHT_ACCEPTANCE"]
        guard mode == "prepare-draft" || mode == "verify-draft" else {
            throw XCTSkip("Enable the local draft check explicitly")
        }
        continueAfterFailure = false
        let expectedMessage = try XCTUnwrap(environment["IRIS_ACCEPTANCE_MESSAGE_IDENTIFIER"])
        let expectedBody = try XCTUnwrap(environment["IRIS_ACCEPTANCE_MESSAGE_BODY"])
        let draft = try XCTUnwrap(environment["IRIS_ACCEPTANCE_DRAFT"])
        XCTAssertTrue(expectedMessage.hasPrefix("chatMessage-"))
        XCTAssertFalse(expectedBody.isEmpty)
        XCTAssertFalse(draft.isEmpty)
        let app = XCUIApplication(bundleIdentifier: "fi.siriusbusiness.irischat")
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))
        let oldMessage = app.staticTexts.matching(identifier: expectedMessage)
            .matching(NSPredicate(format: "label == %@", expectedBody)).firstMatch
        XCTAssertTrue(oldMessage.waitForExistence(timeout: 15), "The exact existing message must remain visible")
        let input = app.textViews["chatMessageInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 15))
        if mode == "prepare-draft" {
            // Never replace an existing draft or send a message.
            XCTAssertTrue((input.value as? String ?? "").isEmpty, "Preserve any existing draft")
            input.tap()
            input.typeText(draft)
        }
        XCTAssertEqual(input.value as? String, draft)
        capture(app, name: mode == "prepare-draft" ? "iris-prepared-draft" : "iris-preserved-draft")
        #endif
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let hierarchy = XCTAttachment(string: app.debugDescription)
        hierarchy.name = name + "-hierarchy"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
        let screen = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screen.name = name + "-screen"
        screen.lifetime = .keepAlways
        add(screen)
    }

    func testInspectExistingIrisState() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("TestFlight acceptance requires the reserved physical iPhone")
        #else
        guard ProcessInfo.processInfo.environment["IRIS_TESTFLIGHT_ACCEPTANCE"] == "iris-inspect" else {
            throw XCTSkip("Enable the local installed-state check explicitly")
        }
        continueAfterFailure = false
        // Inspection only. This does not qualify normal launch or lifecycle
        // survival, and must never inject profile or secret-store overrides.
        let app = XCUIApplication(bundleIdentifier: "fi.siriusbusiness.irischat")
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))
        let contentReady = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                app.buttons.firstMatch.exists || app.staticTexts.firstMatch.exists
            }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [contentReady], timeout: 20), .completed)
        let hierarchy = XCTAttachment(string: app.debugDescription)
        hierarchy.name = "iris-existing-state-hierarchy"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
        let screen = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screen.name = "iris-existing-state-screen"
        screen.lifetime = .keepAlways
        add(screen)
        print("IRIS_EXISTING_STATE_CAPTURED=1")
        #endif
    }

    func testInspectExistingTestFlightAccount() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("TestFlight acceptance requires the reserved physical iPhone")
        #else
        guard ProcessInfo.processInfo.environment["IRIS_TESTFLIGHT_ACCEPTANCE"] == "account-inspect" else {
            throw XCTSkip("Enable the local existing-account check explicitly")
        }
        continueAfterFailure = false
        let testFlight = XCUIApplication(bundleIdentifier: "com.apple.TestFlight")
        testFlight.activate()
        let settings = testFlight.buttons["Settings"].firstMatch
        let grabber = testFlight.buttons["Sheet Grabber"].firstMatch
        if !settings.isHittable && grabber.exists && grabber.isHittable {
            grabber.swipeDown()
        }
        let settingsReady = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "hittable == true"), object: settings)
        XCTAssertEqual(XCTWaiter.wait(for: [settingsReady], timeout: 15), .completed)
        settings.tap()
        _ = testFlight.navigationBars["Settings"].waitForExistence(timeout: 10)
        let hierarchy = XCTAttachment(string: testFlight.debugDescription)
        hierarchy.name = "testflight-existing-account-hierarchy"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
        let screen = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screen.name = "testflight-existing-account-screen"
        screen.lifetime = .keepAlways
        add(screen)
        print("IRIS_TESTFLIGHT_EXISTING_ACCOUNT_CAPTURED=1")
        #endif
    }

    func testInspectTestFlightAvailability() throws {
        #if os(macOS) || targetEnvironment(simulator)
        throw XCTSkip("TestFlight acceptance requires the reserved physical iPhone")
        #else
        guard ProcessInfo.processInfo.environment["IRIS_TESTFLIGHT_ACCEPTANCE"] == "inspect" else {
            throw XCTSkip("Enable the local TestFlight inventory check explicitly")
        }
        continueAfterFailure = false

        let testFlight = XCUIApplication(bundleIdentifier: "com.apple.TestFlight")
        if let invitation = ProcessInfo.processInfo.environment["IRIS_TESTFLIGHT_INVITE_URL"] {
            let url = try XCTUnwrap(URL(string: invitation))
            XCTAssertEqual(url.scheme, "https")
            XCTAssertEqual(url.host, "testflight.apple.com")
            XCTAssertTrue(url.path.hasPrefix("/join/"))
            XCTAssertNil(url.user)
            XCTAssertNil(url.password)
            if #available(iOS 16.4, *) {
                // Universal links are routed by the OS. Forcing an HTTPS link
                // through an app launch does not exercise that routing path.
                XCUIDevice.shared.system.open(url)
            } else {
                throw XCTSkip("Opening the verified invitation requires iOS 16.4 or later")
            }
        } else {
            testFlight.activate()
        }
        XCTAssertTrue(testFlight.wait(for: .runningForeground, timeout: 30),
                      "The installed TestFlight app must become available")

        let contentReady = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                !testFlight.activityIndicators.firstMatch.exists &&
                    (testFlight.buttons.firstMatch.exists || testFlight.staticTexts.firstMatch.exists)
            }, object: nil)
        let readiness = XCTWaiter.wait(for: [contentReady], timeout: 45)

        // Capture the actual offered UI before selecting a build or changing
        // an installation. Never dismiss sign-in, consent, or deletion alerts.
        let hierarchy = XCTAttachment(string: testFlight.debugDescription)
        hierarchy.name = "testflight-inventory-hierarchy"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
        let screen = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screen.name = "testflight-inventory-screen"
        screen.lifetime = .keepAlways
        add(screen)

        XCTAssertEqual(readiness, .completed, "TestFlight must finish its initial loading screen")

        // This records inventory only, not successful upgrade/runtime acceptance.
        print("IRIS_TESTFLIGHT_INVENTORY_CAPTURED=1")
        #endif
    }
}
