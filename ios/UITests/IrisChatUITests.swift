import XCTest

final class IrisChatUITests: XCTestCase {
    private let validPeerNpub = "npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj"
    private let validOwnerNsec = "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"

    func testCreateAccountAndOpenProfileSheet() {
        let app = launchCleanApp()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "welcomeRestoreAction").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "welcomeAddDeviceAction").waitForExistence(timeout: 10))
        createAccount(app)

        XCTAssertTrue(waitForChatList(app, timeout: 10))
        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "myProfileQrCode").waitForExistence(timeout: 5))
    }

    func testLaunchExistingAccountAndAcceptNotificationPermission() {
        let runId = UUID().uuidString
        let setupApp = launchCleanApp(runId: runId)
        createAccount(setupApp)
        setupApp.terminate()

        let app = launchApp(runId: runId)
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))

#if os(iOS)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allowButton = springboard.buttons["Allow"]
        if allowButton.waitForExistence(timeout: 5) {
            allowButton.tap()
        }
#endif

        XCTAssertTrue(waitForChatList(app, timeout: 20))
    }

    func testCreateChatAndSendMessageLocally() {
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        XCTAssertTrue(element(app, "chatComposerBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        typeText("hello from ios ui test", into: element(app, "chatMessageInput"), app: app)
        element(app, "chatSendButton").tap()

        XCTAssertTrue(app.staticTexts["hello from ios ui test"].waitForExistence(timeout: 15))
    }

    func testReturnKeyKeepsMobileDraftUnsent() throws {
#if os(macOS)
        throw XCTSkip("Return key sends on macOS; this checks the mobile keyboard behavior")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        XCTAssertTrue(element(app, "chatComposerBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        typeText("hello from return key\n", into: element(app, "chatMessageInput"), app: app)

        XCTAssertFalse(app.staticTexts["hello from return key"].waitForExistence(timeout: 2))
        element(app, "chatSendButton").tap()
        XCTAssertTrue(app.staticTexts["hello from return key"].waitForExistence(timeout: 15))
#endif
    }

    func testCreateGroupAndOpenGroupDetails() {
        let app = launchCleanApp()

        createAccount(app)

        tapNewChat(app)
        XCTAssertTrue(element(app, "newChatNewGroupButton").waitForExistence(timeout: 10))
        element(app, "newChatNewGroupButton").tap()
        XCTAssertTrue(element(app, "newGroupPrimaryCard").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "newGroupNameInput").waitForExistence(timeout: 10))
        typeText("Trip crew", into: element(app, "newGroupNameInput"), app: app)
        typeText(validPeerNpub, into: element(app, "newGroupMemberInput"), app: app)
        element(app, "newGroupAddMemberButton").tap()
        element(app, "newGroupCreateButton").tap()

        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
        openGroupDetails(app)

        XCTAssertTrue(element(app, "groupDetailsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "groupDetailsNameInput").waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "groupDetailsAddMembersButton").waitForExistence(timeout: 5))
    }

    func testDesktopSidebarNewChatAndSettingsDoNotShowDispatchFailure() throws {
        let app = launchCleanApp()
        createAccount(app)

        let newChatRow = element(app, "desktopNewChatRow")
        guard newChatRow.waitForExistence(timeout: 10) else {
            throw XCTSkip("desktop sidebar is not active on this target")
        }

        newChatRow.tap()
        XCTAssertTrue(element(app, "newChatNewGroupButton").waitForExistence(timeout: 10))
        assertNoDispatchFailureToast(app)

        newChatRow.tap()
        XCTAssertTrue(element(app, "newChatNewGroupButton").waitForExistence(timeout: 5))
        assertNoDispatchFailureToast(app)

        element(app, "chatListProfileButton").tap()
        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        assertNoDispatchFailureToast(app)
    }

    private func openChatWithPeer(_ app: XCUIApplication) {
        tapNewChat(app)
        XCTAssertTrue(element(app, "newChatPeerInput").waitForExistence(timeout: 10))
        typeText(validPeerNpub, into: element(app, "newChatPeerInput"), app: app)
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15))
    }

    func testRestoreAccountOpensDedicatedScreenAndEntersChatList() {
        let app = launchCleanApp()

        XCTAssertTrue(element(app, "welcomeRestoreAction").waitForExistence(timeout: 10))
        element(app, "welcomeRestoreAction").tap()

        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "importKeyField").waitForExistence(timeout: 10))
        typeText(validOwnerNsec, into: element(app, "importKeyField"), app: app)
        element(app, "importKeyButton").tap()

        XCTAssertTrue(waitForChatList(app, timeout: 20))
    }

    func testRestoreInvalidSecretKeyShowsInvalidKey() {
        let app = launchCleanApp()

        XCTAssertTrue(element(app, "welcomeRestoreAction").waitForExistence(timeout: 10))
        element(app, "welcomeRestoreAction").tap()

        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "importKeyField").waitForExistence(timeout: 10))
        typeText("not a secret key", into: element(app, "importKeyField"), app: app)
        element(app, "importKeyButton").tap()

        XCTAssertTrue(app.staticTexts["Invalid key."].waitForExistence(timeout: 10))
    }

    func testLogoutReturnsToWelcomeChooser() {
        let app = launchCleanApp()

        createAccount(app)

        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "myProfileLogoutButton").waitForExistence(timeout: 10))
        element(app, "myProfileLogoutButton").tap()
        XCTAssertTrue(element(app, "myProfileConfirmLogoutButton").waitForExistence(timeout: 10))
        element(app, "myProfileConfirmLogoutButton").tap()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "chatListHeroCard").exists)
    }

    func testScanOwnerQrEntersAwaitingApprovalScreen() throws {
#if os(macOS)
        throw XCTSkip("Camera QR scanning is covered by the iOS UI lane")
#else
        let app = launchCleanApp(qrValue: validPeerNpub)

        XCTAssertTrue(element(app, "welcomeAddDeviceAction").waitForExistence(timeout: 10))
        element(app, "welcomeAddDeviceAction").tap()

        XCTAssertTrue(element(app, "addDeviceScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "linkOwnerScanQrButton").waitForExistence(timeout: 10))
        element(app, "linkOwnerScanQrButton").tap()
        XCTAssertTrue(element(app, "linkExistingAccountButton").waitForExistence(timeout: 10))
        element(app, "linkExistingAccountButton").tap()

        XCTAssertTrue(element(app, "awaitingApprovalScreen").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "awaitingApprovalDeviceQrCode").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "awaitingApprovalDeviceNpub").waitForExistence(timeout: 10))
#endif
    }

    private func launchCleanApp(
        runId: String = UUID().uuidString,
        qrValue: String? = nil,
        profilePicturePath: String? = nil
    ) -> XCUIApplication {
        launchApp(runId: runId, reset: true, qrValue: qrValue, profilePicturePath: profilePicturePath)
    }

    private func launchApp(
        runId: String,
        reset: Bool = false,
        qrValue: String? = nil,
        profilePicturePath: String? = nil
    ) -> XCUIApplication {
        let app = XCUIApplication()
        if reset {
            app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        }
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = runId
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        if let qrValue {
            app.launchEnvironment["IRIS_QR_TEST_VALUE"] = qrValue
        }
        if let profilePicturePath {
            app.launchEnvironment["IRIS_UI_TEST_PROFILE_PICTURE_PATH"] = profilePicturePath
        }
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        return app
    }

    func testUploadProfilePictureUpdatesAvatarsInSettingsAndChatList() throws {
#if os(macOS)
        throw XCTSkip("Profile picture upload is covered outside the default macOS lane")
#else
        let bundle = Bundle(for: type(of: self))
        let fixturePath = bundle.path(forResource: "cat", ofType: "jpg")
            ?? bundle.path(forResource: "cat", ofType: "jpg", inDirectory: "Fixtures")
        guard let fixturePath else {
            throw XCTSkip("cat.jpg fixture not bundled with UI test target")
        }

        let app = launchCleanApp(profilePicturePath: fixturePath)
        createAccount(app)

        // Chat list top avatar exists, has no picture yet.
        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        XCTAssertFalse(element(app, "chatListProfileAvatarImage").exists)

        // Open settings; profile picture viewer should not be reachable yet.
        element(app, "chatListProfileButton").tap()
        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "myProfileUploadPictureButton").waitForExistence(timeout: 5))
        XCTAssertFalse(element(app, "myProfileAvatarImage").exists)

        // Trigger upload via the test escape hatch (env-var supplies the file path,
        // bypassing the file picker). Upload calls a real Blossom server, so allow
        // generous time for the round trip.
        element(app, "myProfileUploadPictureButton").tap()

        // The settings avatar must actually render the uploaded image — not just have
        // a URL set in state. A successfully-loaded image gets loadedImageIdentifier.
        XCTAssertTrue(
            element(app, "myProfileAvatarImage").waitForExistence(timeout: 90),
            "settings avatar did not render the uploaded image"
        )

        returnToChatList(app)
        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))

        // The chat list top avatar must render the same image.
        XCTAssertTrue(
            element(app, "chatListProfileAvatarImage").waitForExistence(timeout: 30),
            "chat list top avatar did not render the uploaded image"
        )
#endif
    }

    private func createAccount(_ app: XCUIApplication) {
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 15))
        element(app, "welcomeCreateAction").tap()

        XCTAssertTrue(element(app, "createAccountScreen").waitForExistence(timeout: 15))
        let nameField = element(app, "signupNameField")
        XCTAssertTrue(nameField.waitForExistence(timeout: 15))
        assertKeyboardFocused(nameField)
        typeText("ios tester", into: nameField, app: app)
        element(app, "generateKeyButton").tap()

        XCTAssertTrue(waitForChatList(app, timeout: 20), "chat list never appeared after account creation")
    }

    private func waitForChatList(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        waitForAnyElement(app, identifiers: ["chatListNewChatButton", "desktopNewChatRow"], timeout: timeout) != nil
    }

    private func tapNewChat(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        guard let newChat = waitForAnyElement(
            app,
            identifiers: ["chatListNewChatButton", "desktopNewChatRow"],
            timeout: 10
        ) else {
            XCTFail("New chat control never appeared", file: file, line: line)
            return
        }
        newChat.tap()
    }

    private func returnToChatList(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        let backButton = element(app, "navigationBackButton")
        if backButton.exists {
            backButton.tap()
        } else if let newChat = waitForAnyElement(
            app,
            identifiers: ["desktopNewChatRow", "chatListNewChatButton"],
            timeout: 5
        ) {
            newChat.tap()
        } else {
            XCTFail("Could not return to chat list", file: file, line: line)
            return
        }
        XCTAssertTrue(waitForChatList(app, timeout: 10), file: file, line: line)
    }

    private func waitForAnyElement(
        _ app: XCUIApplication,
        identifiers: [String],
        timeout: TimeInterval
    ) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            for identifier in identifiers {
                let candidate = element(app, identifier)
                if candidate.exists {
                    return candidate
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return nil
    }

    private func openGroupDetails(_ app: XCUIApplication) {
        let header = element(app, "chatHeaderTitleButton")
        XCTAssertTrue(header.waitForExistence(timeout: 5))
        header.tap()
    }

    private func assertNoDispatchFailureToast(
        _ app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let toast = app.staticTexts["Action failed. Copy support bundle in Settings."]
        XCTAssertFalse(toast.waitForExistence(timeout: 1), "dispatch failure toast appeared", file: file, line: line)
    }

    private func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }

    private func typeText(_ text: String, into target: XCUIElement, app: XCUIApplication) {
#if os(macOS)
        app.activate()
#endif
        target.tap()
        target.typeText(text)
    }

    private func assertKeyboardFocused(
        _ target: XCUIElement,
        timeout: TimeInterval = 5,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
#if os(macOS)
        let predicate = NSPredicate(format: "hasKeyboardFocus == true")
        let expectation = expectation(for: predicate, evaluatedWith: target)
        let result = XCTWaiter.wait(for: [expectation], timeout: timeout)
        XCTAssertEqual(result, .completed, "field did not autofocus", file: file, line: line)
#endif
    }
}
