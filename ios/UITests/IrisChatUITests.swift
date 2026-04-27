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

        XCTAssertTrue(element(app, "navigationTopBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatListHeroCard").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "myProfileQrCode").waitForExistence(timeout: 5))
    }

    func testLaunchExistingAccountAndAcceptNotificationPermission() {
        let app = XCUIApplication()
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))

        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allowButton = springboard.buttons["Allow"]
        if allowButton.waitForExistence(timeout: 5) {
            allowButton.tap()
        }

        XCTAssertTrue(element(app, "navigationTopBar").waitForExistence(timeout: 20))
    }

    func testCreateChatAndSendMessageLocally() {
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        XCTAssertTrue(element(app, "chatComposerBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        element(app, "chatMessageInput").tap()
        element(app, "chatMessageInput").typeText("hello from ios ui test")
        element(app, "chatSendButton").tap()

        XCTAssertTrue(app.staticTexts["hello from ios ui test"].waitForExistence(timeout: 15))
    }

    func testSubmittedMessagesStayPinnedToLatest() {
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        XCTAssertTrue(element(app, "chatComposerBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))

        let messagePrefix = "scroll pin \(Int(Date().timeIntervalSince1970 * 1000))"
        for index in 0..<18 {
            let message = "\(messagePrefix) \(index)"
            element(app, "chatMessageInput").tap()
            element(app, "chatMessageInput").typeText(message)
            element(app, "chatSendButton").tap()
            let row = app.staticTexts[message]
            XCTAssertTrue(row.waitForExistence(timeout: 8))
            XCTAssertTrue(row.isHittable)
        }

        XCTAssertFalse(element(app, "chatJumpToBottom").exists)
    }

    func testReturnKeyKeepsMobileDraftUnsent() {
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        XCTAssertTrue(element(app, "chatComposerBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        element(app, "chatMessageInput").tap()
        element(app, "chatMessageInput").typeText("hello from return key\n")

        XCTAssertFalse(app.staticTexts["hello from return key"].waitForExistence(timeout: 2))
        element(app, "chatSendButton").tap()
        XCTAssertTrue(app.staticTexts["hello from return key"].waitForExistence(timeout: 15))
    }

    func testCreateGroupAndOpenGroupDetails() {
        let app = launchCleanApp()

        createAccount(app)

        element(app, "chatListNewGroupButton").tap()
        XCTAssertTrue(element(app, "newGroupPrimaryCard").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "newGroupNameInput").waitForExistence(timeout: 10))
        element(app, "newGroupNameInput").tap()
        element(app, "newGroupNameInput").typeText("Trip crew")
        element(app, "newGroupMemberInput").tap()
        element(app, "newGroupMemberInput").typeText(validPeerNpub)
        element(app, "newGroupAddMemberButton").tap()
        element(app, "newGroupCreateButton").tap()

        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15))
        openGroupDetails(app)

        XCTAssertTrue(element(app, "groupDetailsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "groupDetailsNameInput").waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "groupDetailsAddMembersButton").waitForExistence(timeout: 5))
    }

    private func openChatWithPeer(_ app: XCUIApplication) {
        element(app, "chatListNewChatButton").tap()
        XCTAssertTrue(element(app, "newChatPrimaryCard").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "newChatPeerInput").waitForExistence(timeout: 10))
        element(app, "newChatPeerInput").tap()
        element(app, "newChatPeerInput").typeText(validPeerNpub)
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15))
    }

    func testRestoreAccountOpensDedicatedScreenAndEntersChatList() {
        let app = launchCleanApp()

        XCTAssertTrue(element(app, "welcomeRestoreAction").waitForExistence(timeout: 10))
        element(app, "welcomeRestoreAction").tap()

        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "importKeyField").waitForExistence(timeout: 10))
        element(app, "importKeyField").tap()
        element(app, "importKeyField").typeText(validOwnerNsec)
        element(app, "importKeyButton").tap()

        XCTAssertTrue(element(app, "chatListNewChatButton").waitForExistence(timeout: 20))
    }

    func testLogoutReturnsToWelcomeChooser() {
        let app = launchCleanApp()

        createAccount(app)

        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "myProfileLogoutButton").waitForExistence(timeout: 10))
        element(app, "myProfileLogoutButton").tap()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "chatListHeroCard").exists)
    }

    func testScanOwnerQrEntersAwaitingApprovalScreen() {
        let app = launchCleanApp(qrValue: validPeerNpub)

        XCTAssertTrue(element(app, "welcomeAddDeviceAction").waitForExistence(timeout: 10))
        element(app, "welcomeAddDeviceAction").tap()

        XCTAssertTrue(element(app, "addDeviceScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "addDeviceQrPlaceholder").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "linkOwnerScanQrButton").waitForExistence(timeout: 10))
        element(app, "linkOwnerScanQrButton").tap()
        XCTAssertTrue(element(app, "linkExistingAccountButton").waitForExistence(timeout: 10))
        element(app, "linkExistingAccountButton").tap()

        XCTAssertTrue(element(app, "awaitingApprovalScreen").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "awaitingApprovalDeviceQrCode").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "awaitingApprovalDeviceNpub").waitForExistence(timeout: 10))
    }

    private func launchCleanApp(
        qrValue: String? = nil,
        profilePicturePath: String? = nil
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["NDR_UI_TEST_RESET"] = "1"
        app.launchEnvironment["NDR_UI_TEST_RUN_ID"] = UUID().uuidString
        if let qrValue {
            app.launchEnvironment["NDR_QR_TEST_VALUE"] = qrValue
        }
        if let profilePicturePath {
            app.launchEnvironment["NDR_UI_TEST_PROFILE_PICTURE_PATH"] = profilePicturePath
        }
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        return app
    }

    func testUploadProfilePictureUpdatesAvatarsInSettingsAndChatList() throws {
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

        // The chat list top avatar must render the same image. On the desktop layout
        // the sidebar stays visible while settings is shown, so we don't need to
        // navigate back first.
        XCTAssertTrue(
            element(app, "chatListProfileAvatarImage").waitForExistence(timeout: 30),
            "chat list top avatar did not render the uploaded image"
        )
    }

    private func createAccount(_ app: XCUIApplication) {
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 15))
        element(app, "welcomeCreateAction").tap()

        XCTAssertTrue(element(app, "createAccountScreen").waitForExistence(timeout: 15))
        let nameField = element(app, "signupNameField")
        XCTAssertTrue(nameField.waitForExistence(timeout: 15))
        nameField.tap()
        nameField.typeText("ios tester")
        element(app, "generateKeyButton").tap()

        // Mobile shows chatListNewChatButton in the top bar; the desktop chrome shows
        // desktopNewChatRow in the sidebar. Either one signals that we've reached the
        // chat list after account creation.
        let newChatTopBar = element(app, "chatListNewChatButton")
        let newChatSidebar = element(app, "desktopNewChatRow")
        let landed = newChatTopBar.waitForExistence(timeout: 20)
            || newChatSidebar.waitForExistence(timeout: 5)
        XCTAssertTrue(landed, "chat list never appeared after account creation")
    }

    private func openGroupDetails(_ app: XCUIApplication) {
        element(app, "chatOverflowButton").tap()
        let item = app.buttons["Group details"]
        XCTAssertTrue(item.waitForExistence(timeout: 5))
        item.tap()
    }

    private func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }
}
