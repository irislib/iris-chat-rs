import XCTest

final class ChatPaginationUITests: IrisChatUITestCase {
    override func setUpWithError() throws {
        try super.setUpWithError()
        continueAfterFailure = false
    }

    func testMixedHeightHistoryPrependPreservesVisibleMessage() throws {
#if os(macOS)
        throw XCTSkip("Controlled touch-drag pagination is verified on iOS")
#else
        let app = fixtureApp([
            "IRIS_UI_TEST_SEED_PEER": "self",
            "IRIS_UI_TEST_SEED_COUNT": "160",
            "IRIS_UI_TEST_SEED_MIXED_HEIGHTS": "1",
            "IRIS_UI_TEST_TRACE_PAGINATION": "1",
        ])
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60))
        openSeededChat(app)
        let timeline = element(app, "chatTimeline")
        XCTAssertTrue(waitUntil(timeout: 10) {
            guard timeline.exists, let snapshot = try? timeline.snapshot() else { return false }
            return messageSnapshots(in: snapshot).contains { snapshot.frame.intersects($0.frame) }
        }, "The initial message page must be visible before measuring a drag")
        var observedOlderPage = false
        var observedHeights: [CGFloat] = []

        // The initial page holds messages 81...160. Reaching message70 proves
        // a real SQLite-backed older page was merged, rather than merely
        // scrolling inside the original loaded window.
        for _ in 0..<70 {
            // Lazy rows can enter or leave the accessibility tree while a
            // page merges. One immutable snapshot keeps indices and frames
            // from different layout passes out of the same measurement.
            let before = try timeline.snapshot()
            let viewport = before.frame
            let visible = messageSnapshots(in: before).filter {
                !$0.frame.isEmpty && viewport.contains($0.frame)
            }
            observedHeights += visible.map { $0.frame.height }
            if visible.contains(where: { ordinal($0.label).map { $0 <= 70 } ?? false }) {
                observedOlderPage = true
                break
            }
            let candidates = visible.filter { $0.frame.maxY < viewport.maxY - viewport.height * 0.32 }
            guard let anchor = candidates.min(by: {
                abs($0.frame.midY - (viewport.minY + viewport.height * 0.3))
                    < abs($1.frame.midY - (viewport.minY + viewport.height * 0.3))
            }) else {
                capture(app, name: "mixed-history-anchor-candidates")
                let detail = XCTAttachment(string: "viewport=\(viewport) rows=" + messageSnapshots(in: before).map { "\($0.label.prefix(20)): \($0.frame)" }.joined(separator: "\n"))
                detail.lifetime = .keepAlways
                add(detail)
                XCTFail("No visible synthetic message could anchor the next controlled scroll")
                return
            }
            let label = anchor.label
            let beforeY = anchor.frame.minY
            let distance = viewport.height * 0.3
            let start = timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.4))
            let end = timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.7))
            // Holding after a slow drag ends momentum before measuring.
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.25)
            let sameMessage = messageSnapshots(in: try timeline.snapshot()).first { $0.label == label }
            let afterY = sameMessage?.frame.minY ?? CGFloat.nan
            let tolerance = max(20, viewport.height * 0.06)
            let stayedVisible = sameMessage.map { viewport.intersects($0.frame) } ?? false
            if !stayedVisible || abs(afterY - beforeY - distance) > tolerance {
                capture(app, name: "older-page-anchor-discontinuity")
                let detail = XCTAttachment(string: "ordinal=\(ordinal(label) ?? -1) beforeY=\(beforeY) afterY=\(afterY) expectedDelta=\(distance) tolerance=\(tolerance)")
                detail.lifetime = .keepAlways
                add(detail)
            }
            XCTAssertTrue(stayedVisible, "Loading/realizing older rows must not discard the visible anchor")
            XCTAssertEqual(afterY - beforeY, distance, accuracy: tolerance,
                           "An older-page merge must preserve the visible message's offset after the drag")
            XCTAssertTrue(element(app, "chatJumpToBottom").exists,
                          "Realizing mixed-height history must not snap the timeline back to latest")
        }
        XCTAssertTrue(observedOlderPage, "Never reached history outside the initial 80-message page")
        XCTAssertGreaterThan((observedHeights.max() ?? 0) - (observedHeights.min() ?? 0), 30,
                             "The production renderer must exercise genuinely different row heights")
        capture(app, name: "mixed-height-older-page-anchor")
#endif
    }

    func testFinalGroupNoticeIsVisibleAfterOpeningLongHistory() {
        let app = fixtureApp([
            "IRIS_UI_TEST_SCREENSHOT_FIXTURE": "1",
            "IRIS_UI_TEST_GROUP_NOTICE": "1",
            "IRIS_UI_TEST_GROUP_NOTICE_PREFIX_COUNT": "95",
        ])
        submitWelcomeName(app, name: "Timeline test", assertFocus: false)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        let row = element(app, "chatRow-fx-chat-2")
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        let timeline = element(app, "chatTimeline")
        let notice = timeline.buttons.matching(NSPredicate(format: "label == %@", "Sam Park was added to the group")).firstMatch
        XCTAssertTrue(waitUntil(timeout: 5) {
            timeline.exists && notice.exists && !notice.frame.isEmpty
                && timeline.frame.intersects(notice.frame) && notice.isHittable
        }, "The actual final system row must be visible, not merely an estimated end marker")
        XCTAssertFalse(element(app, "chatJumpToBottom").exists)
#if os(iOS)
        dragHorizontally(notice, from: 0.15, to: 0.85)
        XCTAssertFalse(element(app, "chatReplyComposer").waitForExistence(timeout: 1),
                       "Measured system rows must remain ineligible for reply swipes")
#endif
        capture(app, name: "long-group-final-notice")
    }

    func testGroupSenderAvatarOpensProfileAndReturnsToConversation() {
        let app = fixtureApp([
            "IRIS_UI_TEST_SCREENSHOT_FIXTURE": "1",
            "IRIS_UI_TEST_GROUP_NOTICE": "1",
            "IRIS_UI_TEST_GROUP_NOTICE_PREFIX_COUNT": "2",
        ])
        submitWelcomeName(app, name: "Timeline test", assertFocus: false)
        XCTAssertTrue(waitForChatList(app, timeout: 30))
        element(app, "chatRow-fx-chat-2").tap()
        let avatar = app.buttons["chatSenderAvatar-fx-chat-2-msg-1"]
        XCTAssertTrue(avatar.waitForExistence(timeout: 10))
        XCTAssertTrue(avatar.isHittable)
        capture(app, name: "group-sender-avatar")
        avatar.tap()
        XCTAssertTrue(element(app, "directChatCopyUserIdButton").waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Sam Park"].firstMatch.waitForExistence(timeout: 5))
        capture(app, name: "group-sender-profile")
#if os(macOS)
        element(app, "desktopPaneBackButton").tap()
#else
        element(app, "navigationBackButton").tap()
#endif
        XCTAssertTrue(avatar.waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "chatMessageInput").exists)
    }

    func testDeepHistoryReturnsToLatestAndSendsFromTheBoundedWindow() throws {
#if os(macOS)
        throw XCTSkip("Bounded native timeline window is iOS-specific")
#else
        let app = fixtureApp([
            "IRIS_UI_TEST_SEED_PEER": "self",
            "IRIS_UI_TEST_SEED_COUNT": "400",
            "IRIS_UI_TEST_SEED_MIXED_HEIGHTS": "1",
            "IRIS_PERF_LOG": "1",
        ])
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60))
        openSeededChat(app)
        let timeline = element(app, "chatTimeline")
        XCTAssertTrue(waitUntil(timeout: 10) {
            guard timeline.exists, let snapshot = try? timeline.snapshot() else { return false }
            return messageSnapshots(in: snapshot).contains { snapshot.frame.intersects($0.frame) }
        }, "The initial message page must be visible before measuring a drag")
        var reachedOlderHistory = false
        for _ in 0..<70 {
            let snapshot = try timeline.snapshot()
            let messages = messageSnapshots(in: snapshot)
            XCTAssertLessThanOrEqual(messages.count, 160, "The UI must not accumulate every fetched row")
            if messages.contains(where: {
                snapshot.frame.intersects($0.frame) && (ordinal($0.label).map { $0 <= 70 } ?? false)
            }) {
                reachedOlderHistory = true
                break
            }
            timeline.swipeDown(velocity: .fast)
        }
        XCTAssertTrue(reachedOlderHistory, "The real history path must cross more than two rendering windows")
        capture(app, name: "deep-history-bounded-window")
        // Return through a real eviction boundary, measuring a slow drag just
        // as in the original prepend regression. Fast flings only approach it.
        for _ in 0..<20 {
            let snapshot = try timeline.snapshot()
            if messageSnapshots(in: snapshot).contains(where: {
                snapshot.frame.intersects($0.frame) && (ordinal($0.label).map { $0 >= 100 } ?? false)
            }) { break }
            timeline.swipeUp(velocity: .fast)
        }
        let initialWindowFirst = messageSnapshots(in: try timeline.snapshot()).compactMap { ordinal($0.label) }.min()
        var crossedEviction = false
        for _ in 0..<40 {
            let before = try timeline.snapshot()
            let viewport = before.frame
            let messages = messageSnapshots(in: before)
            XCTAssertLessThanOrEqual(messages.count, 160)
            if messages.compactMap({ ordinal($0.label) }).min() != initialWindowFirst {
                crossedEviction = true
                break
            }
            let candidates = messages.filter {
                viewport.contains($0.frame) && $0.frame.minY > viewport.minY + viewport.height * 0.32
            }
            guard let anchor = candidates.min(by: {
                abs($0.frame.midY - (viewport.minY + viewport.height * 0.7))
                    < abs($1.frame.midY - (viewport.minY + viewport.height * 0.7))
            }) else { return XCTFail("No visible message for the eviction continuity check") }
            let start = timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.7))
            let end = timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.4))
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.25)
            let afterMessages = messageSnapshots(in: try timeline.snapshot())
            let after = afterMessages.first { $0.label == anchor.label }
            let delta = (after?.frame.minY ?? .nan) - anchor.frame.minY
            let tolerance = max(20, viewport.height * 0.06)
            if after == nil || !delta.isFinite || abs(delta + viewport.height * 0.3) > tolerance {
                capture(app, name: "render-window-drag-discontinuity")
                let detail = XCTAttachment(string:
                    "anchor=\(ordinal(anchor.label) ?? -1) beforeFrame=\(anchor.frame) afterFrame=\(String(describing: after?.frame)) "
                    + "viewport=\(viewport) expectedDelta=\(-viewport.height * 0.3) actualDelta=\(delta) tolerance=\(tolerance) "
                    + "beforeWindow=\(messages.compactMap { ordinal($0.label) }.min() ?? -1)...\(messages.compactMap { ordinal($0.label) }.max() ?? -1) "
                    + "afterWindow=\(afterMessages.compactMap { ordinal($0.label) }.min() ?? -1)...\(afterMessages.compactMap { ordinal($0.label) }.max() ?? -1)")
                detail.name = "render-window-drag-geometry"
                detail.lifetime = .keepAlways
                add(detail)
            }
            XCTAssertNotNil(after, "Evicting offscreen rows must retain the visible message")
            XCTAssertEqual((after?.frame.minY ?? .nan) - anchor.frame.minY, -viewport.height * 0.3,
                           accuracy: max(20, viewport.height * 0.06), "A window swap must preserve the ongoing drag")
        }
        XCTAssertTrue(crossedEviction, "Return scrolling must actually replace the older rendering window")
        let jump = element(app, "chatJumpToBottom")
        XCTAssertTrue(jump.exists)
        jump.tap()
        let last = timeline.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'LAST_SCROLL_SENTINEL'")).firstMatch
        let input = editableElement(app, "chatMessageInput")
        let banner = element(app, "directChatCapabilityBar")
        XCTAssertTrue(waitUntil(timeout: 5) {
            let bottom = banner.exists ? banner.frame.minY : input.frame.minY
            return last.exists && timeline.frame.intersects(last.frame) && last.frame.maxY <= bottom + 1
                && !element(app, "chatJumpToBottom").exists
        }, "Jump to latest must show the final line above the composer after leaving the older window")
        typeText("WINDOW_SEND_SENTINEL", into: input, app: app)
        let started = Date()
        element(app, "chatSendButton").tap()
        let sent = timeline.staticTexts["WINDOW_SEND_SENTINEL"]
        XCTAssertTrue(waitUntil(timeout: 5) { sent.exists && timeline.frame.intersects(sent.frame) })
        let timing = XCTAttachment(string: "send_to_visible_with_ui_automation_seconds=\(Date().timeIntervalSince(started))")
        timing.name = "bounded-window-send-timing"
        timing.lifetime = .keepAlways
        add(timing)
        capture(app, name: "deep-history-latest-send")
#endif
    }

    private func fixtureApp(_ environment: [String: String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment = [
            "IRIS_UI_TEST_RESET": "1",
            "IRIS_UI_TEST_RUN_ID": "timeline-pages-\(UUID().uuidString)",
            "IRIS_UI_TEST_BYPASS_KEYCHAIN": "1",
            "IRIS_DISABLE_NOTIFICATIONS": "1",
            "IRIS_DEMO_RELAYS": "ws://127.0.0.1:9",
        ].merging(environment) { _, value in value }
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15))
        ensureMacWindowVisible(app)
        return app
    }

    private func ordinal(_ label: String) -> Int? {
        Int(label.dropFirst("seed-msg-".count).prefix { $0.isNumber })
    }

    private func messageSnapshots(in snapshot: XCUIElementSnapshot) -> [XCUIElementSnapshot] {
        let own = snapshot.elementType == .staticText && snapshot.label.hasPrefix("seed-msg-")
            ? [snapshot] : []
        return own + snapshot.children.flatMap { messageSnapshots(in: $0) }
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
