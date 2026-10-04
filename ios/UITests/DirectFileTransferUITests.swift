import XCTest

final class DirectFileTransferUITests: IrisChatUITestCase {
    func testNoteToSelfOffersDirectFilesAndOpensTheNativePicker() {
        let app = launchCleanApp(runId: "direct-files-\(UUID().uuidString)")
        createAccount(app)
        let search = editableElement(app, "chatListSearchField")
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        typeText("note to self", into: search, app: app)
        let result = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Note to self")).firstMatch
        XCTAssertTrue(result.waitForExistence(timeout: 10))
        result.tap()
        let input = editableElement(app, "chatMessageInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        typeText("For my laptop", into: input, app: app)
        let attach = element(app, "chatAttachButton")
        XCTAssertTrue(attach.waitForExistence(timeout: 10))
#if os(macOS)
        let composer = element(app, "chatComposerBar")
        XCTAssertTrue(composer.exists)
        XCTAssertEqual(attach.frame.width, 40, accuracy: 2, "Add must stay a compact circular control")
        XCTAssertEqual(attach.frame.height, 40, accuracy: 2)
        XCTAssertLessThan(input.frame.minX - composer.frame.minX, 160,
                          "The attachment menu must not consume the message field's width")
#endif
        attach.tap()
#if os(iOS)
        if #available(iOS 17.0, *) {
            let photos = element(app, "chatAttachmentRecentPhotos")
            let notice = photos.textViews["Private Access to Photos"].firstMatch
            if notice.waitForExistence(timeout: 3) {
                let acknowledge = photos.buttons["OK"].firstMatch
                XCTAssertTrue(waitUntil(timeout: 5) { acknowledge.isHittable })
                acknowledge.tap()
                XCTAssertTrue(waitUntil(timeout: 5) { !notice.exists })
            }
        }
#endif
#if os(macOS)
        XCTAssertTrue(element(app, "chatAttachmentPhotosButton").exists)
        XCTAssertTrue(element(app, "chatAttachmentFilesButton").exists)
#endif
        let direct = element(app, "chatDirectFileButton")
        XCTAssertTrue(direct.waitForExistence(timeout: 10))
#if os(iOS)
        let sources = element(app, "chatAttachmentSources")
        XCTAssertTrue(sources.exists)
        let files = element(app, "chatAttachmentFilesButton")
        XCTAssertEqual(files.frame.midY, direct.frame.midY, accuracy: 2, "Direct files belong in the same source row")
        let rowScreenshot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        rowScreenshot.name = "direct-file-source-row"
        rowScreenshot.lifetime = .keepAlways
        add(rowScreenshot)
        sources.swipeLeft()
#endif
        XCTAssertTrue(waitUntil(timeout: 5) { direct.isHittable })
        let menuScreenshot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        menuScreenshot.name = "direct-file-choice"
        menuScreenshot.lifetime = .keepAlways
        add(menuScreenshot)
        direct.tap()
        let cancel = app.buttons["Cancel"].firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 10), "Direct files must open the system document picker")
        let pickerScreenshot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        pickerScreenshot.name = "direct-file-native-picker"
        pickerScreenshot.lifetime = .keepAlways
        add(pickerScreenshot)
        XCTAssertTrue(waitUntil(timeout: 5) { cancel.isHittable })
        cancel.tap()
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        XCTAssertEqual(input.value as? String, "For my laptop")
        XCTAssertFalse(element(app, "chatDirectFileMode").exists)
    }
}
