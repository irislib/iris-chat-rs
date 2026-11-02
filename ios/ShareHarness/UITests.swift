import XCTest

final class ShareHarnessUITests: XCTestCase {
    private func openShare() -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()
        app.buttons["Share link"].tap()
        XCTAssertTrue(app.cells["share-recipient-ada"].waitForExistence(timeout: 10))
        return app
    }

    func testRecipientsPhotosSearchAndSelection() {
        let app = openShare()
        XCTAssertTrue(app.cells["share-recipient-grace"].exists)
        XCTAssertTrue(app.cells["share-recipient-friends"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Share recipients with profile images"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let search = app.searchFields.firstMatch
        search.tap()
        search.typeText("Grace")
        XCTAssertTrue(app.cells["share-recipient-grace"].exists)
        XCTAssertFalse(app.cells["share-recipient-ada"].exists)
        app.cells["share-recipient-grace"].tap()
        let send = app.buttons["Send"]
        XCTAssertTrue(send.isEnabled)
        // The search keyboard must not cover the selected recipient's action.
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "hittable == true"), object: send)], timeout: 5), .completed)
        send.tap()
        let result = app.staticTexts["fixture-status"]
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label CONTAINS %@", "Queued: Grace Hopper"), object: result)], timeout: 5), .completed)
        XCTAssertTrue(result.label.contains("https://example.com/iris-share-test"))
        XCTAssertTrue(result.label.contains("Send requested: true"))
    }

    func testMultipleRecipientsAreQueuedOnce() {
        let app = openShare()
        app.cells["share-recipient-ada"].tap()
        app.cells["share-recipient-grace"].tap()
        app.buttons["Send 2"].tap()
        let result = app.staticTexts["fixture-status"]
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label CONTAINS %@", "Queued: Ada Lovelace, Grace Hopper"), object: result)], timeout: 5), .completed)
    }

    func testCancelDoesNotQueueASend() {
        let app = openShare()
        app.cells["share-recipient-ada"].tap()
        app.buttons["Close"].tap()
        XCTAssertTrue(app.staticTexts["Share cancelled"].waitForExistence(timeout: 5))
    }
}
