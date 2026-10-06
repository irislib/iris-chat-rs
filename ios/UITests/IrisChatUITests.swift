import XCTest
#if os(iOS)
import UIKit
#endif

class IrisChatUITestCase: XCTestCase {
    let validPeerNpub = "npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj"
    let validOwnerNsec = "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"
    let invalidCompleteOwnerNsec = "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"

    func launchNearbyFixtureApp(firstPeerOwnerHex: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RESET"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "nearby-tap-\(UUID().uuidString)"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_DISABLE_NOTIFICATIONS"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_SCREENSHOT_FIXTURE"] = "1"
        app.launchEnvironment["IRIS_UI_TEST_NEARBY_TAPPABLE_FIRST_PEER_HEX"] = firstPeerOwnerHex
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30))

        // Mirrors ScreenshotTests.createAccount(in:): no keyboard-focus
        // assertion and no inner waitForChatList — fixture mode triggers
        // a longer round-trip than the regular create flow, so we wait
        // for the chat list back in the caller with a generous timeout.
        submitWelcomeName(app, name: "Alex Rivera", assertFocus: false)
        return app
    }

    func openChatWithPeer(_ app: XCUIApplication) {
        tapNewChat(app)
        XCTAssertTrue(element(app, "newChatPeerInput").waitForExistence(timeout: 10))
        typeText(validPeerNpub, into: editableElement(app, "newChatPeerInput"), app: app)
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 15))
    }

    func openSelfOnlyGroup(_ app: XCUIApplication) {
        tapNewChat(app)
        XCTAssertTrue(element(app, "newChatNewGroupButton").waitForExistence(timeout: 10))
        element(app, "newChatNewGroupButton").tap()
        XCTAssertTrue(element(app, "newGroupNextButton").waitForExistence(timeout: 10))
        element(app, "newGroupNextButton").tap()
        XCTAssertTrue(element(app, "newGroupNameInput").waitForExistence(timeout: 10))
        typeText("Composer notes", into: editableElement(app, "newGroupNameInput"), app: app)
        element(app, "newGroupCreateButton").tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
    }
}

final class IrisChatUITests: IrisChatUITestCase {

    func testInviteJoinChoiceStaysInSyncWithItsCodeSheet() {
        let app = launchCleanApp()
        createAccount(app)
        tapNewChat(app)
        let toggle = element(app, "inviteOpenOnJoinToggle")
        XCTAssertTrue(toggle.waitForExistence(timeout: 15))
        XCTAssertEqual(toggle.value as? String, "1")
        toggle.tap()
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value == %@", "0"), object: toggle
        )], timeout: 5), .completed)
        element(app, "newChatInviteQrButton").tap()
        let modal = element(app, "profileQrModal")
        XCTAssertTrue(modal.waitForExistence(timeout: 10))
        let sheetToggle = modal.descendants(matching: .any).matching(identifier: "inviteOpenOnJoinToggle").firstMatch
        XCTAssertTrue(sheetToggle.waitForExistence(timeout: 5))
        XCTAssertEqual(sheetToggle.value as? String, "0")
        sheetToggle.tap()
        element(app, "profileQrDoneButton").tap()
        XCTAssertTrue(toggle.waitForExistence(timeout: 5))
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value == %@", "1"), object: toggle
        )], timeout: 5), .completed)
    }

    func testGrantNotificationPermissionForProductionPushE2E() throws {
#if os(macOS)
        throw XCTSkip("Production APNs permission setup is iOS-only")
#else
        let app = XCUIApplication()
        app.launchEnvironment["IRIS_UI_TEST_RUN_ID"] = "production-push-permission"
        app.launchEnvironment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] = "1"
        app.launchEnvironment["IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION"] = "1"
        app.launchEnvironment["IRIS_REQUEST_NOTIFICATION_PERMISSION_FOR_AUTOMATION"] = "1"
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))

        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allowButton = springboard.buttons["Allow"]
        if allowButton.waitForExistence(timeout: 15) {
            allowButton.tap()
            XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10))
        }
#endif
    }

    /// Regression: constructing CBCentralManager / CBPeripheralManager
    /// in the root view's onAppear was triggering the iOS Bluetooth
    /// permission alert before the user ever opened the Nearby modal.
    /// Apple's UGC review notes flag unsolicited permission prompts.
    /// The simulator persists Bluetooth + Local-Network grants per
    /// bundle id across launches and `simctl privacy` doesn't expose a
    /// reset for either — `scripts/test_no_unsolicited_permissions.sh`
    /// erases the sim before invoking this so the test starts from
    /// "permission not determined".
    func testNoUnsolicitedPermissionPromptsOnFirstLaunch() throws {
#if os(macOS)
        throw XCTSkip("Permission prompts are iOS-only")
#else
        let app = launchCleanApp()
        // The Bluetooth / Local Network prompts (if they regress) fire
        // from the root view's `.onAppear`, so they'd be on screen by
        // the time the welcome chooser paints — no account-creation
        // round-trip needed for this test.
        XCTAssertTrue(
            element(app, "welcomeChooserCard").waitForExistence(timeout: 20),
            "welcome chooser never appeared"
        )
        Thread.sleep(forTimeInterval: 3)

        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let alert = springboard.alerts.firstMatch
        if alert.waitForExistence(timeout: 2) {
            let label = alert.label
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.lifetime = .keepAlways
            attachment.name = "unsolicited-permission-alert"
            add(attachment)
            XCTFail(
                "System permission alert appeared on first launch before Nearby was opened. Alert label: \(label)"
            )
        }
#endif
    }

    func testCreateAccountAndOpenProfileSheet() {
        let app = launchCleanApp()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "onboardingTermsNotice").exists)
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "welcomeRestoreAction").waitForExistence(timeout: 10))
        createAccount(app)

        XCTAssertTrue(waitForChatList(app, timeout: 10))
        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "settingsProfileQrButton").waitForExistence(timeout: 5))
        element(app, "settingsProfileQrButton").tap()
        XCTAssertTrue(element(app, "profileQrModal").waitForExistence(timeout: 5))
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

    func testRelaunchExistingAccountShowsChatListQuickly() {
        let runId = UUID().uuidString
        let setupApp = launchCleanApp(runId: runId)
        createAccount(setupApp)
        setupApp.terminate()

        let budget = TimeInterval(
            Double(ProcessInfo.processInfo.environment["IRIS_UI_TEST_REOPEN_BUDGET_SECONDS"] ?? "") ?? 10
        )
        let app = launchApp(runId: runId)
        let startedAt = Date()
        XCTAssertTrue(
            waitForChatList(app, timeout: budget),
            "existing account did not reopen to the chat list within \(budget)s"
        )
        let elapsed = Date().timeIntervalSince(startedAt)
        XCTAssertLessThanOrEqual(
            elapsed,
            budget,
            "existing account reopen took \(String(format: "%.2f", elapsed))s; budget \(budget)s"
        )
        XCTAssertFalse(
            element(app, "welcomeChooserCard").exists,
            "existing account relaunch must not show the logged-out welcome screen"
        )
    }

    func testChatListSearchCloseButtonDismissesKeyboard() {
#if os(macOS)
        return
#else
        let app = launchCleanApp()

        createAccount(app)

        let searchField = element(app, "chatListSearchField")
        XCTAssertTrue(searchField.waitForExistence(timeout: 10))
        searchField.tap()

        let closeButton = element(app, "chatListSearchCloseButton")
        XCTAssertTrue(closeButton.waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))

        closeButton.tap()
        XCTAssertFalse(closeButton.waitForExistence(timeout: 2))
        XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 2))
#endif
    }

    func testChatListSearchXClearsQueryAndResults() throws {
        continueAfterFailure = false
#if os(macOS)
        throw XCTSkip("UIKit search control is iOS-only")
#else
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 3)
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60))

        let searchField = editableElement(app, "chatListSearchField")
        let searchHit = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH 'messageHit-'")
        ).firstMatch
        let chatRow = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH 'chatRow-'")
        ).firstMatch
        let clearButton = searchField.buttons["Clear text"]
        typeText("FIRST_SCROLL_SENTINEL", into: searchField, app: app)
        XCTAssertTrue(searchHit.waitForExistence(timeout: 15))
        captureSearch(app, named: "search-query-before-clear")
        clearButton.tap()
        captureSearch(app, named: "search-query-after-clear")
        XCTAssertTrue(waitUntil(timeout: 5) {
            (searchField.value as? String ?? "") == (searchField.placeholderValue ?? "")
                || (searchField.value as? String ?? "").isEmpty
        }, "X must clear the entered search text")
        XCTAssertTrue(waitUntil(timeout: 5) { !searchHit.exists })
        XCTAssertTrue(chatRow.waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.firstMatch.exists, "Clearing should keep the keyboard ready")

        typeText("FIRST_SCROLL_SENTINEL", into: searchField, app: app)
        XCTAssertTrue(searchHit.waitForExistence(timeout: 15))
        searchField.typeText("\n")
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists })
        XCTAssertTrue(clearButton.exists, "A submitted query must still have a clear button")
        clearButton.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !searchHit.exists })
        XCTAssertTrue(chatRow.waitForExistence(timeout: 5))
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        XCTAssertFalse(clearButton.exists)
        captureSearch(app, named: "search-submitted-query-cleared")

        typeText("FIRST_SCROLL_SENTINEL", into: searchField, app: app)
        XCTAssertTrue(searchHit.waitForExistence(timeout: 15))
        let closeButton = element(app, "chatListSearchCloseButton")
        XCTAssertGreaterThanOrEqual(closeButton.frame.width, 44)
        XCTAssertGreaterThanOrEqual(closeButton.frame.minX, searchField.frame.maxX)
        closeButton.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !searchHit.exists && !app.keyboards.firstMatch.exists })
        XCTAssertTrue(chatRow.waitForExistence(timeout: 5))
        XCTAssertFalse(closeButton.exists)
#endif
    }

    private func captureSearch(_ app: XCUIApplication, named name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testCreateChatAndSendMessageLocally() {
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        XCTAssertTrue(element(app, "chatComposerBar").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        let messageInput = editableElement(app, "chatMessageInput")
        typeText("hello from ios ui test", into: messageInput, app: app)
#if os(macOS)
        app.typeKey(.return, modifierFlags: [])
#else
        element(app, "chatSendButton").tap()
        dismissNotificationPromptIfPresent(app: app)
#endif

        let messageText = app.staticTexts["hello from ios ui test"].firstMatch
        XCTAssertTrue(messageText.waitForExistence(timeout: 15))

        // Regression guard for a TruncatableMessageBody bug that made
        // single-line bubbles render half-screen tall: SwiftUI promoted
        // the .frame(maxHeight:320) proposed to ViewThatFits into an
        // enforced height, and the inner Text stretched to match. The
        // staticText accessibility frame surfaces the rendered text
        // size — when the bubble blows up to 320pt, the wrapped Text
        // does too, so 60pt is a comfortable ceiling above one line
        // (~25pt) and far below the broken state.
        XCTAssertLessThan(
            messageText.frame.height,
            60,
            "Single-line bubble text rendered \(messageText.frame.height)pt tall — should be ~25pt"
        )

#if os(iOS)
        app.staticTexts["hello from ios ui test"].press(forDuration: 0.6)
        XCTAssertTrue(element(app, "messageActionsSheet").waitForExistence(timeout: 5))
#else
        app.staticTexts["hello from ios ui test"].tap()
        Thread.sleep(forTimeInterval: 0.15)
        let moreButton = element(app, "messageMoreButton")
        XCTAssertTrue(moreButton.exists)
        let actionGap = messageText.frame.minX - moreButton.frame.maxX
        XCTAssertGreaterThan(
            actionGap,
            0,
            "Outgoing message action dock should sit to the left of the bubble"
        )
        XCTAssertLessThan(
            actionGap,
            90,
            "Outgoing message action dock drifted \(actionGap)pt from the bubble"
        )
        let infoButton = element(app, "messageInfoButton")
        XCTAssertTrue(infoButton.exists)
        infoButton.tap()
#endif
        #if os(macOS)
        XCTAssertTrue(element(app, "messageInfoStatus").waitForExistence(timeout: 5))
        #else
        let messageInfoAction = app.buttons["Info"].firstMatch
        XCTAssertTrue(messageInfoAction.waitForExistence(timeout: 5))
        messageInfoAction.tap()
        XCTAssertTrue(element(app, "messageInfoSheet").waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "messageInfoStatus").waitForExistence(timeout: 5))
        #endif
    }

    func testMacComposerMultilineViewportGrowsAndCapsAtFiveLines() throws {
#if os(macOS)
        let app = launchCleanApp()
        createAccount(app)
        openChatWithPeer(app)

        let editor = editableElement(app, "chatMessageInput")
        let viewport = app.scrollViews.matching(identifier: "chatMessageInput").firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        XCTAssertTrue(viewport.waitForExistence(timeout: 10))

        typeText("line 1", into: editor, app: app)
        var previousHeight = viewport.frame.height
        XCTAssertGreaterThan(previousHeight, 0)

        for line in 2...5 {
            app.typeKey(.return, modifierFlags: .shift)
            typeText("line \(line)", into: editor, app: app)
            XCTAssertTrue(
                waitUntil(timeout: 2) { viewport.frame.height > previousHeight + 1 },
                "composer did not grow when line \(line) was inserted"
            )
            previousHeight = viewport.frame.height
        }

        app.typeKey(.return, modifierFlags: .shift)
        typeText("line 6", into: editor, app: app)
        XCTAssertTrue(
            waitUntil(timeout: 2) { (editor.value as? String)?.contains("line 6") == true },
            "sixth line was not committed to the native editor"
        )
        let cappedHeight = viewport.frame.height
        XCTAssertEqual(cappedHeight, previousHeight, accuracy: 1)
        RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        XCTAssertEqual(
            viewport.frame.height,
            cappedHeight,
            accuracy: 1,
            "composer viewport grew beyond five lines"
        )
#else
        throw XCTSkip("AppKit composer layout is macOS-only")
#endif
    }

    func testComposerKeepsSequentialTypingOrder() throws {
#if os(macOS)
        throw XCTSkip("UIKit composer input is iOS-only")
#else
        let app = launchCleanApp()

        createAccount(app)
        openSelfOnlyGroup(app)

        let input = element(app, "chatMessageInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap()

        var expected = ""
        for character in "hello" {
            expected.append(character)
            input.typeText(String(character))
            XCTAssertTrue(
                waitUntil(timeout: 2) {
                    (input.value as? String) == expected
                },
                "composer value after typing \(character) was \((input.value as? String) ?? "<nil>"), expected \(expected)"
            )
        }

        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        element(app, "chatTimeline")
            .coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
            .tap()
        XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 2))

        element(app, "chatSendButton").tap()
        XCTAssertTrue(app.staticTexts["hello"].firstMatch.waitForExistence(timeout: 15))
#endif
    }

    func testQuickReactionPillStaysTappableAfterReacting() throws {
#if os(macOS)
        throw XCTSkip("Message reaction sheet is iOS-only")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        let message = "reaction \(UUID().uuidString)"
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        typeText(message, into: editableElement(app, "chatMessageInput"), app: app)
        element(app, "chatSendButton").tap()
        dismissNotificationPromptIfPresent(app: app)

        let messageText = app.staticTexts[message].firstMatch
        XCTAssertTrue(messageText.waitForExistence(timeout: 15))
        messageText.press(forDuration: 0.6)

        XCTAssertTrue(element(app, "messageActionsSheet").waitForExistence(timeout: 5))
        app.buttons["❤️"].firstMatch.tap()

        let reactionRow = element(app, "chatReactionRow")
        XCTAssertTrue(reactionRow.waitForExistence(timeout: 10))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.lifetime = .keepAlways
        attachment.name = "reaction-pill-position"
        add(attachment)

        reactionRow.tap()
        XCTAssertTrue(element(app, "messageReactorsSheet").waitForExistence(timeout: 5))
#endif
    }

}

final class IrisChatComposerUITests: IrisChatUITestCase {

    func testSentMessageStaysOutOfComposerWhenReopeningChat() throws {
#if os(macOS)
        throw XCTSkip("Covered by the shared composer state tests on macOS")
#else
        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)
        let message = "sent draft \(UUID().uuidString)"
        let input = editableElement(app, "chatMessageInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        typeText(message, into: input, app: app)
        element(app, "chatSendButton").tap()
        XCTAssertTrue(app.staticTexts[message].firstMatch.waitForExistence(timeout: 15))
        XCTAssertEqual(input.value as? String, "")

        returnToChatList(app)
        let row = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'chatRow-'"))
            .firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        XCTAssertTrue(app.staticTexts[message].firstMatch.waitForExistence(timeout: 10))
        let reopenedInput = editableElement(app, "chatMessageInput")
        XCTAssertTrue(reopenedInput.waitForExistence(timeout: 10))
        XCTAssertEqual(reopenedInput.value as? String, "")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "sent-message-empty-composer"
        screenshot.lifetime = .keepAlways
        add(screenshot)
#endif
    }

    func testComposerRestoresDraftWhenReopeningChat() throws {
#if os(macOS)
        throw XCTSkip("Covered by the shared draft persistence unit tests on macOS")
#else
        let app = launchCleanApp()
        createAccount(app)
        openSelfOnlyGroup(app)

        let draft = "draft \(UUID().uuidString)"
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        typeText(draft, into: editableElement(app, "chatMessageInput"), app: app)

        returnToChatList(app)
        let row = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'chatRow-'"))
            .firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()

        let input = element(app, "chatMessageInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        XCTAssertTrue(
            waitUntil(timeout: 10) {
                ((input.value as? String) ?? "").contains(draft)
            },
            "composer did not restore the draft"
        )
#endif
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
        typeText("hello from return key\n", into: editableElement(app, "chatMessageInput"), app: app)

        XCTAssertFalse(app.staticTexts["hello from return key"].waitForExistence(timeout: 2))
        element(app, "chatSendButton").tap()
        dismissNotificationPromptIfPresent(app: app)
        XCTAssertTrue(app.staticTexts["hello from return key"].waitForExistence(timeout: 15))
#endif
    }

    func testCreateGroupAndOpenGroupDetails() {
        continueAfterFailure = false
        let app = launchCleanApp()

        createAccount(app)

        tapNewChat(app)
        XCTAssertTrue(element(app, "newChatInviteShareButton").waitForExistence(timeout: 15))
        XCTAssertTrue(element(app, "newChatNewGroupButton").waitForExistence(timeout: 10))
        element(app, "newChatNewGroupButton").tap()
        XCTAssertTrue(element(app, "newGroupMemberStep").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "newGroupNextButton").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "newGroupPasteButton").exists)
        XCTAssertFalse(element(app, "newGroupScanQrButton").exists)
        XCTAssertFalse(element(app, "newGroupAddMemberButton").exists)
        typeText(validPeerNpub, into: editableElement(app, "newGroupMemberInput"), app: app)
        XCTAssertTrue(element(app, "memberChipRemove").waitForExistence(timeout: 5))
        element(app, "newGroupNextButton").tap()
        XCTAssertTrue(element(app, "newGroupDetailsStep").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "newGroupNameInput").waitForExistence(timeout: 10))
        typeText("Trip crew", into: editableElement(app, "newGroupNameInput"), app: app)
        element(app, "newGroupCreateButton").tap()

        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
        openGroupDetails(app)

        XCTAssertTrue(element(app, "groupDetailsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "groupDetailsNameInput").waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "groupDetailsAddMembersButton").waitForExistence(timeout: 5))

        let adminButton = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'groupDetailsAdminMember-' AND label == 'Make admin'")).firstMatch
        XCTAssertTrue(adminButton.exists)
        for _ in 0..<5 where !adminButton.isHittable {
#if os(macOS)
            app.scrollViews.firstMatch.scroll(byDeltaX: 0, deltaY: 400)
#else
            app.swipeUp()
#endif
        }
        adminButton.tap()
        let confirmation = app.alerts.firstMatch
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5), "Granting admin access must require confirmation")
        confirmation.buttons["Cancel"].tap()
        XCTAssertEqual(adminButton.label, "Make admin", "Cancel must leave the member's role unchanged")

        adminButton.tap()
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "confirm-group-admin-promotion"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        confirmation.buttons["groupDetailsConfirmAdminButton"].firstMatch.tap()
        XCTAssertTrue(app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'groupDetailsAdminMember-' AND label == 'Dismiss admin'")).firstMatch.waitForExistence(timeout: 10))
    }

}

final class IrisChatFlowUITests: IrisChatUITestCase {

    func testGroupNameKeyboardDismissalPreservesDraftAndAllowsRefocus() throws {
#if os(macOS)
        throw XCTSkip("On-screen keyboard dismissal is iOS-specific")
#else
        continueAfterFailure = false
        let app = launchCleanApp()
        createAccount(app)
        tapNewChat(app)
        element(app, "newChatNewGroupButton").tap()
        element(app, "newGroupNextButton").tap()

        let input = editableElement(app, "newGroupNameInput")
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        input.typeText("Trip crew\n")
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists }, "Return should dismiss the group name keyboard")
        XCTAssertEqual(input.value as? String, "Trip crew")
        XCTAssertTrue(element(app, "newGroupCreateButton").exists, "Done must not create the group")

        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        input.typeText(" weekend")
        app.staticTexts["Group details"].tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists }, "Tapping outside the name should dismiss the keyboard")
        XCTAssertEqual(input.value as? String, "Trip crew weekend")

        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let create = element(app, "newGroupCreateButton")
        create.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1)).withOffset(CGVector(dx: 0, dy: 30)).tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists }, "Tapping the empty background should dismiss the keyboard")

        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "group-name-keyboard-dismissed"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.buttons["Done"].exists)
        app.keyboards.buttons["Done"].tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !app.keyboards.firstMatch.exists })
        create.tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
#endif
    }

    func testGroupPhotoButtonsOpenFilePickerOnMac() throws {
#if os(macOS)
        continueAfterFailure = false
        let app = launchCleanApp()
        createAccount(app)
        tapNewChat(app)
        element(app, "newChatNewGroupButton").tap()
        element(app, "newGroupNextButton").tap()
        typeText("Photo group", into: editableElement(app, "newGroupNameInput"), app: app)

        for photoButton in ["newGroupPhotoButton", "groupDetailsChangePhotoButton"] {
            element(app, photoButton).tap()
            let openButton = app.buttons["Open"].firstMatch
            XCTAssertTrue(openButton.waitForExistence(timeout: 5), "Group photos should open the native file picker directly")
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.name = "\(photoButton)-file-picker"
            screenshot.lifetime = .keepAlways
            add(screenshot)
            if photoButton == "newGroupPhotoButton" {
                let image = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "cat", withExtension: "jpg", subdirectory: "Fixtures"))
                app.typeKey("g", modifierFlags: [.command, .shift])
                app.typeText(image.path)
                app.typeKey(.return, modifierFlags: [])
                XCTAssertTrue(waitUntil(timeout: 5) { openButton.isEnabled })
                openButton.tap()
                XCTAssertTrue(element(app, "newGroupRemovePhotoButton").waitForExistence(timeout: 10), "Selecting a disk image should add it to the group draft")
                // Keep group creation local; uploading the photo is covered separately.
                element(app, "newGroupRemovePhotoButton").tap()
                element(app, "newGroupCreateButton").tap()
                XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
                openGroupDetails(app)
            } else {
                app.buttons["Cancel"].firstMatch.tap()
            }
        }
#else
        throw XCTSkip("iOS offers photo sources; macOS opens files directly")
#endif
    }

    func testCreateSelfOnlyGroup() {
        let app = launchCleanApp()

        createAccount(app)

        tapNewChat(app)
        XCTAssertTrue(element(app, "newChatNewGroupButton").waitForExistence(timeout: 10))
        element(app, "newChatNewGroupButton").tap()
        XCTAssertTrue(element(app, "newGroupMemberStep").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "newGroupNextButton").isEnabled)
        element(app, "newGroupNextButton").tap()
        XCTAssertTrue(element(app, "newGroupDetailsStep").waitForExistence(timeout: 10))
        typeText("Solo notes", into: editableElement(app, "newGroupNameInput"), app: app)
        XCTAssertTrue(element(app, "newGroupCreateButton").isEnabled)
        element(app, "newGroupCreateButton").tap()

        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 45))
        openGroupDetails(app)
        XCTAssertTrue(element(app, "groupDetailsScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "groupDetailsNameInput").waitForExistence(timeout: 5))
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

    func testDesktopSettingsShowsStartAtLogin() throws {
#if os(macOS)
        let app = launchCleanApp()
        createAccount(app)

        element(app, "chatListProfileButton").tap()
        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        openSettingsPage(app, "settingsGeneralRow")
        XCTAssertTrue(
            element(app, "myProfileStartupAtLoginToggle").waitForExistence(timeout: 5),
            "General settings should expose Start at login on macOS"
        )
#else
        throw XCTSkip("Start at login is available on desktop platforms")
#endif
    }

    func testDesktopNearbyModalDismissesFromCloseButtonAndOutsideClick() throws {
#if os(macOS)
        let app = launchCleanApp()
        createAccount(app)

        let nearbyRow = app.buttons.matching(identifier: "desktopNearbyRow").firstMatch
        XCTAssertTrue(nearbyRow.waitForExistence(timeout: 10))

        nearbyRow.tap()
        let closeButton = element(app, "nearbyCloseButton")
        XCTAssertTrue(closeButton.waitForExistence(timeout: 10))
        closeButton.tap()
        XCTAssertFalse(closeButton.waitForExistence(timeout: 2))

        nearbyRow.tap()
        XCTAssertTrue(closeButton.waitForExistence(timeout: 10))
        app.windows.firstMatch
            .coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.12))
            .tap()
        XCTAssertFalse(closeButton.waitForExistence(timeout: 2))
#else
        throw XCTSkip("Nearby uses the native mobile sheet on iOS")
#endif
    }

    func testTappingNearbyPreviewPeerOpensChat() throws {
#if os(macOS)
        throw XCTSkip("Mobile nearby preview regression")
#else
        let app = launchNearbyFixtureApp(firstPeerOwnerHex: "fx-chat-1")
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        let peer = element(app, "nearbyPreviewPeer-fx-near-1")
        XCTAssertTrue(peer.waitForExistence(timeout: 10))
        XCTAssertTrue(peer.isHittable)
        let preview = XCTAttachment(screenshot: app.screenshot())
        preview.name = "nearby-preview-before-tap"
        preview.lifetime = .keepAlways
        add(preview)
        peer.tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10),
                      "tapping the nearby user icon in the chat list should open their chat")
        assertNoDispatchFailureToast(app)
        let chat = XCTAttachment(screenshot: app.screenshot())
        chat.name = "nearby-preview-opened-chat"
        chat.lifetime = .keepAlways
        add(chat)
#endif
    }

    func testTappingUnknownNearbyPreviewPeerOpensProfile() throws {
#if os(macOS)
        throw XCTSkip("Mobile nearby preview regression")
#else
        let peerHex = String(repeating: "ab", count: 32)
        let app = launchNearbyFixtureApp(firstPeerOwnerHex: peerHex)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        let peer = element(app, "nearbyPreviewPeer-fx-near-1")
        XCTAssertTrue(peer.waitForExistence(timeout: 10))
        XCTAssertTrue(peer.isHittable)
        peer.tap()
        XCTAssertTrue(element(app, "directChatCopyUserIdButton").waitForExistence(timeout: 10),
                      "a nearby user without an existing chat should open their profile")
        assertNoDispatchFailureToast(app)
        let profile = XCTAttachment(screenshot: app.screenshot())
        profile.name = "nearby-preview-opened-profile"
        profile.lifetime = .keepAlways
        add(profile)
#endif
    }

    /// Regression: tapping a nearby peer must navigate into a chat with
    /// them, not just create a chat-list row. The previous implementation
    /// dispatched `.createChat`, which has no optimistic-navigation path,
    /// so the sheet's `onClose()` ran sync while the Rust round-trip to
    /// flip `screen_stack = [.chat]` was still in flight, and the user
    /// landed back on the chat list. The fix uses `.openChat`, which is
    /// wired into `handleOptimisticNavigation`.
    func testTappingNearbyPeerOpensChat() throws {
#if os(macOS)
        throw XCTSkip("Nearby modal on macOS isn't a sheet; covered by other tests")
#else
        let app = launchNearbyFixtureApp(firstPeerOwnerHex: "fx-chat-1")
        XCTAssertTrue(waitForChatList(app, timeout: 30), "chat list never appeared after fixture launch")

        let nearbyRow = element(app, "nearbyChatRow")
        XCTAssertTrue(nearbyRow.waitForExistence(timeout: 10), "nearby chat row missing")
        nearbyRow.tap()

        let firstPeer = element(app, "nearbyPeer-fx-near-1")
        XCTAssertTrue(firstPeer.waitForExistence(timeout: 10), "first nearby peer never appeared")
        XCTAssertTrue(firstPeer.isHittable, "first nearby peer should be tappable when ownerPubkeyHex is set")
        firstPeer.tap()

        XCTAssertTrue(
            element(app, "chatMessageInput").waitForExistence(timeout: 10),
            "tapping a nearby peer should navigate into a chat — composer never appeared"
        )
        assertNoDispatchFailureToast(app)
#endif
    }

    func testRestoreAccountOpensDedicatedScreenAndEntersChatList() {
        let app = launchCleanApp()

        tapWelcomeAction(app, "welcomeRestoreAction")

        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "importKeyButton").exists)
        XCTAssertTrue(element(app, "importKeyField").waitForExistence(timeout: 10))
        typeText(validOwnerNsec, into: editableElement(app, "importKeyField"), app: app)

        XCTAssertTrue(waitForChatList(app, timeout: 20))
    }

    func testRestoreInvalidSecretKeyShowsInvalidKey() {
        let app = launchCleanApp()

        tapWelcomeAction(app, "welcomeRestoreAction")

        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "importKeyField").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "importKeyButton").exists)
        typeText(invalidCompleteOwnerNsec, into: editableElement(app, "importKeyField"), app: app)

        XCTAssertTrue(app.staticTexts["Invalid key."].waitForExistence(timeout: 10))
    }

    func testOnboardingScreensUseHeaderBackOnly() {
        let app = launchCleanApp()

        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        assertOnboardingScreenUsesHeaderBack(
            app,
            actionIdentifier: "welcomeRestoreAction",
            screenIdentifier: "restoreAccountScreen"
        )
        tapWelcomeAction(app, "welcomeRestoreAction")
        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
#if os(iOS)
        acceptOnboardingTermsIfNeeded(app)
#endif
        XCTAssertTrue(element(app, "restoreLinkDeviceAction").waitForExistence(timeout: 10))
        element(app, "restoreLinkDeviceAction").tap()
        XCTAssertTrue(element(app, "remoteSignerScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "navigationBackButton").waitForExistence(timeout: 5))
        XCTAssertFalse(element(app, "onboardingBackButton").exists)
    }

    func testLogOutFromSettingsExplainsLocalRemovalAndReturnsToWelcome() {
        let app = launchCleanApp()

        createAccount(app)

        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        openSettingsPage(app, "settingsLogOutButton")
        XCTAssertTrue(element(app, "settingsConfirmLogOutButton").waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["This removes secret keys, messages, and cached files from this device. Your profile and other devices stay unchanged."].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "settings-log-out-confirmation"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["Cancel"].firstMatch.tap()
        XCTAssertTrue(element(app, "settingsScreen").exists)
        XCTAssertFalse(element(app, "welcomeChooserCard").exists)
        openSettingsPage(app, "settingsLogOutButton")
        app.buttons["settingsConfirmLogOutButton"].firstMatch.tap()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "chatListHeroCard").exists)
    }

    func testLinkDeviceHistoryChoiceCanBeCancelled() throws {
#if os(macOS)
        throw XCTSkip("The device scanner is available on iOS")
#else
        let code = "nostr-identity://device-approval/eyJkZXZpY2VBcHBLZXlOcHViIjoibnB1YjFwMzRlZnpta2V3d2Rza3NtcHAycjB0azdxdWtlOWpjZmR6MnpsN2V6azh3bnNqNDN1ejJzOHg1c3A0IiwicmVxdWVzdE5wdWIiOiJucHViMTh3MzVnNmduNDdxd21yeXVseHp2ZnVjbXVqdnJxcWxqanBhcHlsOHgwcnFhbGpoNmYydXNtbDc3ZGoiLCJyZXF1ZXN0U2VjcmV0IjoiQVFFQkFRRUJBUUVCQVFFQkFRRUJBUUVCQVFFQkFRRUJBUUVCQVFFQkFRRSJ9"
        let app = launchCleanApp(qrValue: code)
        createAccount(app)
        element(app, "chatListProfileButton").tap()
        openSettingsPage(app, "settingsDevicesRow")
        let scan = element(app, "deviceRosterScanButton")
        XCTAssertTrue(scan.waitForExistence(timeout: 10))
        scan.tap()
        let history = app.alerts.buttons["deviceRosterConfirmAdd"].firstMatch
        XCTAssertTrue(history.waitForExistence(timeout: 10))
        XCTAssertEqual(history.label, "Include message history")
        XCTAssertTrue(app.buttons["Chats and groups only"].exists)
        XCTAssertTrue(app.staticTexts["Both options include your chats, groups, and new messages."].exists)
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "link-device-history-choice"
        attachment.lifetime = .keepAlways
        add(attachment)
        app.buttons["Cancel"].tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !history.exists })
        XCTAssertTrue(scan.exists)
#endif
    }

    func testDeviceLinkShowsProgressAfterHistoryApproval() throws {
#if os(macOS)
        throw XCTSkip("The device scanner is available on iOS")
#else
        let code = "nostrconnect://79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            + "?relay=ws%3A%2F%2F127.0.0.1%3A1&secret=progress-test&perms=sign_event%3A37368"
        let app = launchCleanApp(qrValue: code)
        createAccount(app)
        element(app, "chatListProfileButton").tap()
        openSettingsPage(app, "settingsDevicesRow")
        element(app, "deviceRosterScanButton").tap()
        let history = app.buttons["Include message history"]
        XCTAssertTrue(history.waitForExistence(timeout: 10))
        history.tap()
        XCTAssertTrue(element(app, "deviceLinkProgress").waitForExistence(timeout: 5))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "device-link-in-progress"
        attachment.lifetime = .keepAlways
        add(attachment)
#endif
    }

    func testLinkDeviceShowsScannableCode() throws {
        let app = launchCleanApp()

        tapWelcomeAction(app, "welcomeRestoreAction")
        let linkAction = element(app, "restoreLinkDeviceAction")
        XCTAssertTrue(linkAction.waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "restoreSignerAction").exists)
#if os(iOS)
        XCTAssertFalse(linkAction.isEnabled)
        acceptOnboardingTermsIfNeeded(app)
        XCTAssertTrue(waitUntil(timeout: 5) { linkAction.isEnabled })
#endif
        linkAction.tap()

        XCTAssertTrue(element(app, "remoteSignerScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "remoteSignerCode").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "remoteSignerCopyLink").waitForExistence(timeout: 10))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "unified-link-this-device"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testPastedDeviceLinkRequiresHistoryConfirmation() {
        let app = launchCleanApp()
        createAccount(app)
        element(app, "chatListProfileButton").tap()
        openSettingsPage(app, "settingsDevicesRow")
        let code = "nostrconnect://" + String(repeating: "ab", count: 32)
            + "?relay=wss%3A%2F%2Fexample.invalid&secret=test-link"
#if os(iOS)
        UIPasteboard.general.string = code
        let input = editableElement(app, "deviceRosterAddInput")
        input.tap()
        input.press(forDuration: 1)
        let paste = app.menuItems["Paste"]
        XCTAssertTrue(paste.waitForExistence(timeout: 5))
        paste.tap()
#else
        typeText(code, into: editableElement(app, "deviceRosterAddInput"), app: app)
#endif
        let history = app.buttons["Include message history"]
        XCTAssertTrue(history.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Chats and groups only"].exists)
        app.buttons["Cancel"].tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !history.exists })
        XCTAssertTrue(element(app, "deviceRosterAddInput").exists)
    }

    func testLinkDeviceShowsScannableCodeAfterLogOut() throws {
        let app = launchCleanApp()
        createAccount(app)

        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()
        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        openSettingsPage(app, "settingsLogOutButton")
        XCTAssertTrue(element(app, "settingsConfirmLogOutButton").waitForExistence(timeout: 10))
        app.buttons["settingsConfirmLogOutButton"].firstMatch.tap()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 20))
        tapWelcomeAction(app, "welcomeRestoreAction")
        let linkAction = element(app, "restoreLinkDeviceAction")
        XCTAssertTrue(linkAction.waitForExistence(timeout: 10))
#if os(iOS)
        if !linkAction.isEnabled {
            acceptOnboardingTermsIfNeeded(app)
        }
        XCTAssertTrue(waitUntil(timeout: 5) { linkAction.isEnabled })
#endif
        linkAction.tap()

        XCTAssertTrue(element(app, "remoteSignerScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "remoteSignerCode").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "remoteSignerCopyLink").waitForExistence(timeout: 10))
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
        openSettingsPage(app, "settingsProfileRow")
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

}
