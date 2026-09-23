import XCTest

final class IrisAttachmentPickerUITests: IrisChatUITestCase {

    private func prepareAttachmentPickerForInteraction(_ app: XCUIApplication) {
#if os(iOS)
        if #available(iOS 17.0, *) {
            let recentPhotos = element(app, "chatAttachmentRecentPhotos")
            XCTAssertTrue(recentPhotos.waitForExistence(timeout: 5))
            let privacyNotice = recentPhotos.textViews["Private Access to Photos"].firstMatch
            if privacyNotice.waitForExistence(timeout: 5) {
                let acknowledgeButton = recentPhotos.buttons["OK"].firstMatch
                XCTAssertTrue(waitUntil(timeout: 5) { acknowledgeButton.isHittable })
                acknowledgeButton.tap()
                XCTAssertTrue(waitUntil(timeout: 5) { !privacyNotice.exists })
            }
        }
#endif
        XCTAssertTrue(waitUntil(timeout: 5) {
            element(app, "chatAttachmentCloseButton").isHittable
        })
        let hierarchy = XCTAttachment(string: app.debugDescription)
        hierarchy.name = "attachment-picker-ready-hierarchy"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)

        XCTAssertGreaterThan(
            element(app, "chatAttachmentPicker").frame.minY,
            app.windows.firstMatch.frame.height * 0.3,
            "The attachment picker should open as a bottom sheet"
        )
    }

    func testComposerPlusOpensAttachmentSheetAndPreservesDraft() throws {
#if os(macOS)
        throw XCTSkip("macOS opens the file picker directly")
#else
        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)

        let draft = "Attachment draft"
        let input = editableElement(app, "chatMessageInput")
        typeText(draft, into: input, app: app)

        let attachButton = element(app, "chatAttachButton")
        XCTAssertTrue(attachButton.waitForExistence(timeout: 10))
        attachButton.tap()

        let picker = element(app, "chatAttachmentPicker")
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        prepareAttachmentPickerForInteraction(app)
        if #available(iOS 17.0, *) {
            XCTAssertTrue(element(app, "chatAttachmentRecentPhotos").waitForExistence(timeout: 5))
        }
        let cameraButton = element(app, "chatAttachmentCameraButton")
        XCTAssertTrue(cameraButton.exists)
#if targetEnvironment(simulator)
        XCTAssertFalse(cameraButton.isEnabled, "the simulator has no camera")
#endif
        XCTAssertTrue(element(app, "chatAttachmentPhotosButton").isHittable)
        XCTAssertTrue(element(app, "chatAttachmentFilesButton").isHittable)

        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "composer-attachment-sheet"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        element(app, "chatAttachmentCloseButton").tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !picker.exists })
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, draft)
#endif
    }

    func testAttachmentRecentPhotoStagesWithoutSending() throws {
#if os(macOS)
        throw XCTSkip("The attachment sheet is iOS-only")
#else
        guard #available(iOS 17.0, *) else {
            throw XCTSkip("Recent photos require iOS 17 or later")
        }

        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)

        let caption = "Photo caption stays in draft"
        let input = editableElement(app, "chatMessageInput")
        typeText(caption, into: input, app: app)
        element(app, "chatAttachButton").tap()

        let picker = element(app, "chatAttachmentPicker")
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        prepareAttachmentPickerForInteraction(app)
        let recentPhotos = element(app, "chatAttachmentRecentPhotos")
        XCTAssertTrue(recentPhotos.waitForExistence(timeout: 5))

        let firstPhoto = recentPhotos.images.matching(identifier: "PXGGridLayout-Info").firstMatch
        let hasPhoto = firstPhoto.waitForExistence(timeout: 10)
        guard hasPhoto else {
            throw XCTSkip("Recent-photo selection needs a seeded photo in the system photo library")
        }
        firstPhoto.tap()

        XCTAssertTrue(waitUntil(timeout: 5) { !picker.exists })
        let selectedAttachments = element(app, "chatSelectedAttachments")
        XCTAssertTrue(selectedAttachments.waitForExistence(timeout: 15))
        let removeButtons = app.buttons.matching(identifier: "chatSelectedAttachmentRemove")
        XCTAssertEqual(removeButtons.count, 1)
        XCTAssertEqual(input.value as? String, caption)
        XCTAssertFalse(app.staticTexts[caption].firstMatch.waitForExistence(timeout: 2))

        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "recent-photo-staged-with-caption"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        removeButtons.firstMatch.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !selectedAttachments.exists })
        XCTAssertEqual(input.value as? String, caption)
#endif
    }

    func testAttachmentPhotosPickerCancelPreservesDraft() throws {
#if os(macOS)
        throw XCTSkip("The attachment sheet is iOS-only")
#else
        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)

        let draft = "Keep this while choosing a photo"
        let input = editableElement(app, "chatMessageInput")
        typeText(draft, into: input, app: app)
        element(app, "chatAttachButton").tap()

        XCTAssertTrue(element(app, "chatAttachmentPicker").waitForExistence(timeout: 5))
        prepareAttachmentPickerForInteraction(app)
        let photosButton = element(app, "chatAttachmentPhotosButton")
        XCTAssertTrue(photosButton.waitForExistence(timeout: 5))
        photosButton.tap()

        let cancelButton = app.buttons["Cancel"].firstMatch
        XCTAssertTrue(
            cancelButton.waitForExistence(timeout: 10),
            "Photos did not present the system photo picker"
        )
        XCTAssertTrue(waitUntil(timeout: 5) { !element(app, "chatAttachmentPicker").exists })
        cancelButton.tap()

        XCTAssertTrue(waitUntil(timeout: 5) { !cancelButton.exists })
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, draft)
#endif
    }

    func testAttachmentFilesPickerCancelPreservesDraft() throws {
#if os(macOS)
        throw XCTSkip("The attachment sheet is iOS-only")
#else
        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)

        let draft = "Keep this while choosing a file"
        let input = editableElement(app, "chatMessageInput")
        typeText(draft, into: input, app: app)
        element(app, "chatAttachButton").tap()

        XCTAssertTrue(element(app, "chatAttachmentPicker").waitForExistence(timeout: 5))
        prepareAttachmentPickerForInteraction(app)
        let filesButton = element(app, "chatAttachmentFilesButton")
        XCTAssertTrue(filesButton.waitForExistence(timeout: 5))
        filesButton.tap()

        let cancelButton = app.buttons["Cancel"].firstMatch
        XCTAssertTrue(
            cancelButton.waitForExistence(timeout: 10),
            "Files did not present the system document picker"
        )
        XCTAssertTrue(waitUntil(timeout: 5) { !element(app, "chatAttachmentPicker").exists })
        cancelButton.tap()

        XCTAssertTrue(waitUntil(timeout: 5) { !cancelButton.exists })
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, draft)
#endif
    }
}
