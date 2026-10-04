import XCTest

final class ChatTimelineUITests: IrisChatUITestCase {
    func testMessageBubbleHorizontalSwipesOpenReplyAndInfo() throws {
#if os(macOS)
        throw XCTSkip("Message bubble swipe actions are iOS-only")
#else
        let app = launchCleanApp()

        createAccount(app)
        openSelfOnlyGroup(app)

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
        openSelfOnlyGroup(app)

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
        let timeline = element(app, "chatTimeline")
        let latest = timeline.staticTexts.matching(NSPredicate(
            format: "label BEGINSWITH 'LAST_SCROLL_SENTINEL' OR value BEGINSWITH 'LAST_SCROLL_SENTINEL'"
        )).firstMatch
        XCTAssertTrue(waitUntil(timeout: 5) {
            latest.exists && !latest.frame.isEmpty && timeline.frame.intersects(latest.frame)
        }, "The actual final message must land inside the viewport after opening")
        let composer = element(app, "chatMessageInput")
        let banner = element(app, "directChatCapabilityBar")
        XCTAssertTrue(waitUntil(timeout: 5) {
            let bottom = banner.exists ? banner.frame.minY : composer.frame.minY
            return latest.exists && latest.frame.maxY <= bottom + 1
        }, "The final line must clear the composer and any messaging status bar")

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

        XCTAssertFalse(element(app, "chatHeaderSearchButton").exists)
        element(app, "chatHeaderTitleButton").tap()
        XCTAssertTrue(element(app, "chatDetailsSearchButton").waitForExistence(timeout: 10))
        let pin = element(app, "chatDetailsPinButton")
        XCTAssertTrue(pin.waitForExistence(timeout: 5))
        pin.tap()
        XCTAssertTrue(app.buttons["Unpin chat"].waitForExistence(timeout: 5))
        let details = XCTAttachment(screenshot: app.screenshot())
        details.name = "chat-details-search-and-pin"
        details.lifetime = .keepAlways
        add(details)
        element(app, "chatDetailsSearchButton").tap()
        let searchField = editableElement(app, "inChatSearchField")
        XCTAssertTrue(searchField.waitForExistence(timeout: 10))
        typeText("FIRST_SCROLL_SENTINEL", into: searchField, app: app)

        let oldestSearchHit = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'inChatMessageHit-'")).firstMatch
        XCTAssertTrue(oldestSearchHit.waitForExistence(timeout: 15))
        XCTAssertTrue(
            oldestSearchHit.staticTexts["ios tester"].exists || oldestSearchHit.label.contains("ios tester"),
            "Outgoing search result must show the sender, not the other participant"
        )
        let searchResult = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        searchResult.name = "in-chat-search-outgoing-author"
        searchResult.lifetime = .keepAlways
        add(searchResult)
        oldestSearchHit.tap()
        XCTAssertTrue(waitUntil(timeout: 5) { !searchField.exists })
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))

        let oldestTimelineMessage = app.staticTexts.matching(
            NSPredicate(
                format: "label BEGINSWITH 'FIRST_SCROLL_SENTINEL' OR value BEGINSWITH 'FIRST_SCROLL_SENTINEL'"
            )
        ).firstMatch
        let landed = waitUntil(timeout: 15) {
            oldestTimelineMessage.exists && oldestTimelineMessage.isHittable
                && element(app, "chatTimeline").frame.intersects(oldestTimelineMessage.frame)
        }
        let landing = XCTAttachment(screenshot: app.screenshot())
        landing.name = "historical-search-landing"
        landing.lifetime = .keepAlways
        add(landing)
        XCTAssertTrue(landed, "Search must load and visibly land on the message outside the initial 80-message page")
    }

    func testDesktopSidebarGroupedSearchOpensHistoricalMessage() throws {
#if os(macOS)
        let app = launchCleanApp(seedPeer: validPeerNpub, seedCount: 120)

        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60), "seed helper never returned to the chat list")

        let searchField = editableElement(app, "chatListSearchField")
        XCTAssertTrue(searchField.waitForExistence(timeout: 10))
        typeText("FIRST_SCROLL_SENTINEL", into: searchField, app: app)

        let searchHit = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH 'messageHit-'")
        ).firstMatch
        XCTAssertTrue(
            searchHit.waitForExistence(timeout: 15),
            "desktop sidebar did not render the Rust message-search result"
        )
        searchHit.tap()
        XCTAssertTrue(element(app, "chatMessageInput").waitForExistence(timeout: 10))

        let timeline = element(app, "chatTimeline")
        XCTAssertTrue(timeline.waitForExistence(timeout: 10))
        let historicalMessage = timeline.staticTexts.matching(
            NSPredicate(
                format: "label BEGINSWITH 'FIRST_SCROLL_SENTINEL' OR value BEGINSWITH 'FIRST_SCROLL_SENTINEL'"
            )
        ).firstMatch
        XCTAssertTrue(
            historicalMessage.waitForExistence(timeout: 15),
            "desktop search hit did not open the historical message"
        )
#else
        throw XCTSkip("Desktop sidebar is macOS-only")
#endif
    }
}
