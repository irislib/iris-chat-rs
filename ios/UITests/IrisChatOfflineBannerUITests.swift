import XCTest

extension IrisChatUITests {
    func testRestoreScreenDoesNotShowOfflineWarningBeforeLogin() throws {
#if os(macOS)
        throw XCTSkip("The offline banner is mobile-only")
#else
        let app = launchCleanApp()
        let restore = element(app, "welcomeRestoreAction")
        XCTAssertTrue(restore.waitForExistence(timeout: 20))
        restore.tap()
        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))

        let bannerAppears = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true"),
            object: element(app, "offlineStatusBanner")
        )
        bannerAppears.isInverted = true
        // Stay on the login form beyond the production 30-second grace period.
        XCTAssertEqual(XCTWaiter.wait(for: [bannerAppears], timeout: 35), .completed)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "restore-without-offline-warning"
        screenshot.lifetime = .keepAlways
        add(screenshot)
#endif
    }

    func testLoggedInServerOutageShowsAccurateWarning() throws {
#if os(macOS)
        throw XCTSkip("The offline banner is mobile-only")
#else
        let app = launchCleanApp()
        createAccount(app)
        XCTAssertTrue(element(app, "offlineStatusBanner").waitForExistence(timeout: 45))
        XCTAssertTrue(app.staticTexts["Can’t reach message servers"].exists)
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'Bluetooth off'")).firstMatch.exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "logged-in-server-outage"
        screenshot.lifetime = .keepAlways
        add(screenshot)
#endif
    }
}
