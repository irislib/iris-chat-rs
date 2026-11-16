#if os(iOS) && targetEnvironment(simulator)
import XCTest

final class ShareFromPhotosUITests: IrisChatUITestCase {
    func testAutomaticPhotoShareAfterColdLaunch() throws {
        try sharePhoto(useCurrentFormat: false)
    }

    func testOriginalHEICShareWithIrisInBackground() throws {
        try sharePhoto(useCurrentFormat: true)
    }

    private func sharePhoto(useCurrentFormat: Bool) throws {
        continueAfterFailure = false
        let runID = UUID().uuidString
        let app = launchCleanApp(runId: runID)
        createAccount(app)
        if !useCurrentFormat { app.terminate() }

        let photos = XCUIApplication(bundleIdentifier: "com.apple.mobileslideshow")
        photos.launch()
        if photos.buttons["Continue"].firstMatch.waitForExistence(timeout: 2) {
            photos.buttons["Continue"].firstMatch.tap()
        }
        if photos.buttons["Share"].firstMatch.exists {
            let back = photos.buttons["Back"].firstMatch
            XCTAssertTrue(back.exists, photos.debugDescription)
            back.tap()
        }
        let library = photos.buttons["Library"].firstMatch
        if library.exists { library.tap() }
        // The standard iPhone simulator library includes a March 2018 HEIC
        // photo of flowers. Pick it by date so unrelated imports are ignored.
        let image = photos.images.matching(identifier: "PXGGridLayout-Info")
            .matching(NSPredicate(format: "label CONTAINS %@", "2018")).firstMatch
        try XCTSkipUnless(image.waitForExistence(timeout: 8), "The simulator’s sample HEIC photo is missing")
        image.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        let share = photos.buttons["Share"].firstMatch
        XCTAssertTrue(share.waitForExistence(timeout: 5), photos.debugDescription)
        share.tap()

        if useCurrentFormat {
            let options = photos.buttons.matching(NSPredicate(format: "label BEGINSWITH[c] %@", "Options")).firstMatch
            XCTAssertTrue(options.waitForExistence(timeout: 5), photos.debugDescription)
            options.tap()
            let current = photos.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Current")).firstMatch
            XCTAssertTrue(current.waitForExistence(timeout: 5), photos.debugDescription)
            current.tap()
            photos.buttons["Done"].firstMatch.tap()
        }
        let iris = photos.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Iris Chat")).firstMatch
        if !iris.waitForExistence(timeout: 2) || !iris.isHittable {
            let more = photos.buttons["More"].firstMatch
            XCTAssertTrue(more.exists, photos.debugDescription)
            more.tap()
        }
        XCTAssertTrue(iris.waitForExistence(timeout: 5), photos.debugDescription)
        iris.tap()
        let recipient = photos.cells.matching(NSPredicate(format: "label CONTAINS[c] %@", "Note to self")).firstMatch
        XCTAssertTrue(recipient.waitForExistence(timeout: 10), photos.debugDescription)
        recipient.tap()
        photos.buttons["Send"].tap()
        XCTAssertTrue(photos.staticTexts["Queued for Iris Chat"].waitForExistence(timeout: 5))
        capture(photos, "Photos share queued")
        photos.buttons["Done"].tap()

        let chat: XCUIApplication
        if useCurrentFormat {
            app.activate()
            chat = app
        } else {
            chat = launchApp(runId: runID)
        }
        let header = element(chat, "chatHeaderTitleButton")
        XCTAssertTrue(waitUntil(timeout: 20) { header.exists && header.label.contains("Note to self") }, chat.debugDescription)
        let photosInChat = chat.buttons.matching(NSPredicate(
            format: "identifier BEGINSWITH 'chatMessage-' AND (label ENDSWITH[c] '.jpg' OR label ENDSWITH[c] '.jpeg') AND value == 'Sent'"
        ))
        let photo = photosInChat.firstMatch
        XCTAssertTrue(photo.waitForExistence(timeout: 45), chat.debugDescription)
        XCTAssertEqual(photosInChat.count, 1)
        if useCurrentFormat { XCTAssertTrue(photo.label.hasSuffix(".jpg"), photo.label) }
        XCTAssertTrue(photo.images.firstMatch.waitForExistence(timeout: 30), "The photo preview did not load")
        capture(chat, "Photo sent to Note to self")
        photo.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(element(chat, "imageViewerCloseButton").waitForExistence(timeout: 10), chat.debugDescription)
        capture(chat, "Photo opened in Note to self")
    }

    private func capture(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
#endif
