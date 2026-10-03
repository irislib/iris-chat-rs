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
        ])
        submitWelcomeName(app)
        XCTAssertTrue(waitForChatList(app, timeout: 60))
        openSeededChat(app)
        let timeline = element(app, "chatTimeline")
        XCTAssertTrue(timeline.waitForExistence(timeout: 10))
        let messages = timeline.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'seed-msg-'"))
        var observedOlderPage = false
        var observedHeights: [CGFloat] = []

        // The initial page holds messages 81...160. Reaching message70 proves
        // a real SQLite-backed older page was merged, rather than merely
        // scrolling inside the original loaded window.
        for _ in 0..<70 {
            let viewport = timeline.frame
            let visible = messages.allElementsBoundByIndex.filter {
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
            let sameMessage = timeline.staticTexts.matching(NSPredicate(format: "label == %@", label)).firstMatch
            let afterY = sameMessage.exists ? sameMessage.frame.minY : CGFloat.nan
            let tolerance = max(20, viewport.height * 0.06)
            let stayedVisible = sameMessage.exists && viewport.intersects(sameMessage.frame)
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

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
