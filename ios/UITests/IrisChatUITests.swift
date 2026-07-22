import XCTest

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
}

final class IrisChatUITests: IrisChatUITestCase {

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

    func testComposerKeepsSequentialTypingOrder() throws {
#if os(macOS)
        throw XCTSkip("UIKit composer input is iOS-only")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

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

    func testComposerRestoresDraftWhenReopeningChat() throws {
#if os(macOS)
        throw XCTSkip("Covered by the shared draft persistence unit tests on macOS")
#else
        let app = launchCleanApp()
        createAccount(app)
        openChatWithPeer(app)

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

    func testComposerPlusOpensAttachmentMenu() throws {
#if os(macOS)
        throw XCTSkip("macOS opens the file picker directly")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        let attachButton = element(app, "chatAttachButton")
        XCTAssertTrue(attachButton.waitForExistence(timeout: 10))
        attachButton.tap()

        XCTAssertTrue(
            app.buttons["Photo Library"].waitForExistence(timeout: 5) ||
                app.buttons["Files"].waitForExistence(timeout: 5),
            "composer plus did not open the attachment menu"
        )
#endif
    }

    func testMessageBubbleHorizontalSwipesOpenReplyAndInfo() throws {
#if os(macOS)
        throw XCTSkip("Message bubble swipe actions are iOS-only")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        let message = "swipe actions \(UUID().uuidString) lorem ipsum"
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))
        typeText(message, into: editableElement(app, "chatMessageInput"), app: app)
        element(app, "chatSendButton").tap()
        dismissNotificationPromptIfPresent(app: app)

        let messageText = app.staticTexts[message].firstMatch
        XCTAssertTrue(messageText.waitForExistence(timeout: 15))

        dragHorizontally(messageText, from: 0.15, to: 0.98)
        guard element(app, "chatReplyComposer").waitForExistence(timeout: 5) else {
            XCTFail("right swipe on message bubble did not open reply composer")
            return
        }
        let closeReply = element(app, "chatReplyCancelButton").exists
            ? element(app, "chatReplyCancelButton")
            : app.buttons["Close"].firstMatch
        closeReply.tap()
        XCTAssertFalse(element(app, "chatReplyComposer").waitForExistence(timeout: 2))

        dragHorizontally(messageText, from: 0.85, to: 0.02)
        XCTAssertTrue(
            element(app, "messageInfoSheet").waitForExistence(timeout: 5),
            "left swipe on message bubble did not open message details"
        )
#endif
    }

    func testShortReplyBubbleDoesNotExpandToTimelineWidth() throws {
#if os(macOS)
        throw XCTSkip("Mobile reply sheets are covered on iOS")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)

        let original = "tiny \(String(UUID().uuidString.prefix(6)).lowercased())"
        typeText(original, into: editableElement(app, "chatMessageInput"), app: app)
        element(app, "chatSendButton").tap()
        dismissNotificationPromptIfPresent(app: app)

        let originalText = app.staticTexts[original].firstMatch
        XCTAssertTrue(originalText.waitForExistence(timeout: 15))
        originalText.press(forDuration: 0.6)
        XCTAssertTrue(element(app, "messageActionsSheet").waitForExistence(timeout: 5))
        app.buttons["Reply"].firstMatch.tap()
        XCTAssertTrue(element(app, "chatReplyComposer").waitForExistence(timeout: 5))

        let reply = "ok \(String(UUID().uuidString.prefix(4)).lowercased())"
        typeText(reply, into: editableElement(app, "chatMessageInput"), app: app)
        element(app, "chatSendButton").tap()

        let replyText = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", reply)
        ).firstMatch
        XCTAssertTrue(replyText.waitForExistence(timeout: 15))
        let replyPreview = app.buttons.matching(
            NSPredicate(format: "label CONTAINS %@", original)
        ).firstMatch
        XCTAssertTrue(replyPreview.waitForExistence(timeout: 5))

        XCTAssertLessThan(
            replyPreview.frame.width,
            app.windows.firstMatch.frame.width * 0.55,
            "Short reply preview expanded to \(replyPreview.frame.width)pt wide"
        )
#endif
    }

    func testChatListRowHorizontalSwipeStillShowsActions() throws {
#if os(macOS)
        throw XCTSkip("Chat list row swipes are iOS-only")
#else
        let app = launchCleanApp()

        createAccount(app)
        openChatWithPeer(app)
        returnToChatList(app)

        let row = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'chatRow-'"))
            .firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))

        row.swipeLeft()
        XCTAssertTrue(app.buttons["Delete"].waitForExistence(timeout: 5))
#endif
    }

    // Opening a chat with enough messages to overflow the viewport
    // must land scrolled at the latest message. The oracle is the
    // "jump to bottom" affordance — it only renders when the timeline
    // is *not* near the bottom, so its absence proves the initial
    // scroll succeeded. Regression guard for the SwiftUI `LazyVStack`
    // case where lazy rows above the viewport haven't been measured at
    // scroll time and the manual `proxy.scrollTo(.bottom)` lands too
    // high — fixed by `defaultScrollAnchor(.bottom)`.
    //
    // Uses the IRIS_UI_TEST_SEED_* escape hatch to dispatch outgoing
    // messages directly through AppManager once the account exists —
    // XCUITest's typeText+tap loop gets flaky past ~12 sends, which
    // can't reliably build a chat tall enough to test the lazy-row case.
    func testReopeningLongChatLandsAtBottom() {
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 30)

        // Walk the welcome → create-account flow without the trailing
        // chatList wait — the seed dispatches createChat right after
        // the account exists, so the core navigates straight into the
        // new chat and the chat list never paints until the seed pops
        // back at the end.
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 45), "seed helper never returned to the chat list")

        openSeededChat(app, rowTimeout: 30)
        Thread.sleep(forTimeInterval: 1.5)

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.lifetime = .keepAlways
        attachment.name = "timeline-on-open"
        add(attachment)

        // The bug: the chat opens scrolled mid-timeline so older
        // messages are visible at the bottom and the user can't see
        // the most recent one. The `chatJumpToBottom` button only
        // renders when the timeline is *not* near the bottom, so its
        // absence is proof the initial scroll succeeded.
        XCTAssertFalse(
            element(app, "chatJumpToBottom").exists,
            "chat opened without scrolling to the latest message — the jump-to-bottom button is visible"
        )
    }

}

final class IrisChatTimelineUITests: IrisChatUITestCase {

    func testDaySeparatorHandoffKeepsYesterdayUntilTodayHeaderReachesTop() throws {
#if os(macOS)
        throw XCTSkip("Timeline sticky date header behavior is iOS-specific")
#else
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 48, seedDaySplitIndex: 24)

        submitWelcomeName(app)
        guard waitForAnyElement(
            app,
            identifiers: ["chatListNewChatButton", "desktopNewChatRow", "chatMessageInput"],
            timeout: 75
        ) != nil else {
            XCTFail("seed helper never reached the chat list or opened seeded chat")
            return
        }

        if !element(app, "chatMessageInput").exists {
            let chatRowPreview = seededChatRowPreview(app)
            XCTAssertTrue(chatRowPreview.waitForExistence(timeout: 45), "seeded split-day chat never appeared")
            chatRowPreview.tap()
        }
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))

        let timeline = app.scrollViews["chatTimeline"].firstMatch
        XCTAssertTrue(timeline.waitForExistence(timeout: 10))

        var sawBoundary = false
        for _ in 0..<18 {
            let floating = element(app, "chatFloatingDaySeparator")
            let todayInline = inlineDaySeparator(app, label: "Today")
            if floating.exists,
               todayInline.exists,
               todayInline.frame.minY > floating.frame.maxY + 4 {
                sawBoundary = true
                XCTAssertEqual(
                    floating.label,
                    "Yesterday",
                    "floating header handed off while the Today inline header was still below it"
                )
                break
            }
            dragVertically(timeline, x: 0.75, fromY: 0.52, toY: 0.88)
            RunLoop.current.run(until: Date().addingTimeInterval(0.15))
        }

        if !sawBoundary {
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.lifetime = .keepAlways
            attachment.name = "day-separator-handoff-boundary-not-found"
            add(attachment)
            XCTFail("did not find the split-day boundary with Today visible below the floating header")
        }
#endif
    }

    func testJumpToBottomDoesNotPinTimelineAfterUserScrollsAgain() throws {
#if os(macOS)
        throw XCTSkip("Scroll gesture lock regression is iOS-specific")
#else
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 120)

        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60), "seed helper never returned to the chat list")

        openSeededChat(app)

        let timeline = app.scrollViews["chatTimeline"].firstMatch
        XCTAssertTrue(timeline.waitForExistence(timeout: 10))
        // Seeded messages are outgoing/right-aligned; this starts inside
        // a visible bubble instead of the empty timeline gutter.
        dragVertically(timeline, x: 0.75, fromY: 0.55, toY: 0.9)
        dragVertically(timeline, x: 0.75, fromY: 0.55, toY: 0.9)

        XCTAssertTrue(
            element(app, "chatJumpToBottom").waitForExistence(timeout: 5),
            "timeline did not move away from bottom before the jump test"
        )
        element(app, "chatJumpToBottom").tap()
        XCTAssertTrue(
            waitUntil(timeout: 3) { !element(app, "chatJumpToBottom").exists },
            "jump-to-bottom button did not disappear after tapping it"
        )

        dragVertically(timeline, x: 0.75, fromY: 0.55, toY: 0.9)
        XCTAssertTrue(
            element(app, "chatJumpToBottom").waitForExistence(timeout: 2),
            "timeline stayed pinned after a manual jump-to-bottom followed by user scroll"
        )
#endif
    }

    func testJumpToBottomRespondsDuringTimelineFlick() throws {
#if os(macOS)
        throw XCTSkip("Timeline deceleration tap regression is iOS-specific")
#else
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 120)

        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60), "seed helper never returned to the chat list")

        openSeededChat(app)

        let timeline = app.scrollViews["chatTimeline"].firstMatch
        XCTAssertTrue(timeline.waitForExistence(timeout: 10))
        dragVertically(timeline, x: 0.75, fromY: 0.55, toY: 0.9)
        XCTAssertTrue(
            element(app, "chatJumpToBottom").waitForExistence(timeout: 5),
            "timeline did not move away from bottom before the deceleration jump test"
        )

        flickVertically(timeline, x: 0.75, fromY: 0.55, toY: 0.95)
        element(app, "chatJumpToBottom").tap()
        XCTAssertTrue(
            waitUntil(timeout: 3) { !element(app, "chatJumpToBottom").exists },
            "jump-to-bottom button ignored a tap while the timeline was still settling"
        )
#endif
    }

    func testSearchHitInSeededLongChatOpensInTimeline() {
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 120)

        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60), "seed helper never returned to the chat list")

        openSeededChat(app)

        XCTAssertTrue(element(app, "chatHeaderSearchButton").waitForExistence(timeout: 10))
        element(app, "chatHeaderSearchButton").tap()
        let searchField = editableElement(app, "inChatSearchField")
        XCTAssertTrue(searchField.waitForExistence(timeout: 10))
        typeText("FIRST_SCROLL_SENTINEL", into: searchField, app: app)

        let oldestSearchHit = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'inChatMessageHit-'")).firstMatch
        XCTAssertTrue(oldestSearchHit.waitForExistence(timeout: 15))
        oldestSearchHit.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !searchField.exists })
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))

        let oldestTimelineMessage = app.staticTexts.matching(
            NSPredicate(
                format: "label BEGINSWITH 'FIRST_SCROLL_SENTINEL' OR value BEGINSWITH 'FIRST_SCROLL_SENTINEL'"
            )
        ).firstMatch
        XCTAssertTrue(
            oldestTimelineMessage.waitForExistence(timeout: 15),
            "search hit outside the initial 80-message page did not load into the chat timeline"
        )
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
    }

}

final class IrisChatFlowUITests: IrisChatUITestCase {

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
#if os(iOS)
        acceptOnboardingTermsIfNeeded(app)
#endif
        XCTAssertFalse(element(app, "importKeyButton").exists)
        XCTAssertTrue(element(app, "importKeyField").waitForExistence(timeout: 10))
        typeText(validOwnerNsec, into: editableElement(app, "importKeyField"), app: app)

        XCTAssertTrue(waitForChatList(app, timeout: 20))
    }

    func testRestoreInvalidSecretKeyShowsInvalidKey() {
        let app = launchCleanApp()

        tapWelcomeAction(app, "welcomeRestoreAction")

        XCTAssertTrue(element(app, "restoreAccountScreen").waitForExistence(timeout: 10))
#if os(iOS)
        acceptOnboardingTermsIfNeeded(app)
#endif
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
        XCTAssertTrue(element(app, "addDeviceScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "navigationBackButton").waitForExistence(timeout: 5))
        XCTAssertFalse(element(app, "onboardingBackButton").exists)
    }

    func testDeleteLocalDataReturnsToWelcomeChooser() {
        let app = launchCleanApp()

        createAccount(app)

        XCTAssertTrue(element(app, "chatListProfileButton").waitForExistence(timeout: 15))
        element(app, "chatListProfileButton").tap()

        XCTAssertTrue(element(app, "settingsScreen").waitForExistence(timeout: 10))
        openSettingsPage(app, "settingsAccountDataRow")
        XCTAssertTrue(element(app, "myProfileDeleteLocalDataButton").waitForExistence(timeout: 10))
        element(app, "myProfileDeleteLocalDataButton").tap()
        XCTAssertTrue(element(app, "myProfileConfirmDeleteLocalDataButton").waitForExistence(timeout: 10))
        app.buttons["myProfileConfirmDeleteLocalDataButton"].firstMatch.tap()

        XCTAssertTrue(element(app, "welcomeChooserCard").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "welcomeCreateAction").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "chatListHeroCard").exists)
    }

    func testLinkDeviceShowsScannableCode() throws {
        let app = launchCleanApp()

        tapWelcomeAction(app, "welcomeRestoreAction")
        let linkAction = element(app, "restoreLinkDeviceAction")
        XCTAssertTrue(linkAction.waitForExistence(timeout: 10))
#if os(iOS)
        XCTAssertFalse(linkAction.isEnabled)
        acceptOnboardingTermsIfNeeded(app)
        XCTAssertTrue(waitUntil(timeout: 5) { linkAction.isEnabled })
#endif
        linkAction.tap()

        XCTAssertTrue(element(app, "addDeviceScreen").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "linkDeviceQrCode").waitForExistence(timeout: 20))
        XCTAssertTrue(element(app, "linkDeviceCopyButton").waitForExistence(timeout: 10))
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
