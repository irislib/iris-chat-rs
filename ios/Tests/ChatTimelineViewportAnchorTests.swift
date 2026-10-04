#if os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class ChatTimelineViewportAnchorTests: XCTestCase {
    func testTopGeometryReductionKeepsTheMeasuredValueAcrossEmptySiblings() {
        var value: CGFloat = -435
        ChatTimelineTopMinYPreferenceKey.reduce(value: &value) { ChatTimelineTopMinYPreferenceKey.defaultValue }
        XCTAssertEqual(value, -435)
    }

    @MainActor
    func testVisibleBottomUsesNativeInsetOnceAndIgnoresTheScrollFrameOrigin() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 62, width: 390, height: 778))
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        scroll.contentInset.bottom = 56
        XCTAssertEqual(coordinator.visibleViewportMaxY, 722)
        scroll.contentInset.bottom = 98
        XCTAssertEqual(coordinator.visibleViewportMaxY, 680, "A 42-point bar removes exactly 42 points")
        scroll.contentOffset.y = 3_000
        XCTAssertEqual(coordinator.visibleViewportMaxY, 680, "Content offset and outer frame origin are not visible height")
    }

    @MainActor
    func testHistoryAnchorWaitsForTheNewNativeContentExtent() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize = CGSize(width: 390, height: 1_000)
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "81", messageID: "83", originalContentY: 220)
        let loaded = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "1",
            frames: ["83": CGRect(x: 0, y: 920, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 1_020, width: 100, height: 80)], contentHeight: 3_000)
        XCTAssertFalse(coordinator.restoreHistoryViewportAnchor(page: loaded))
        XCTAssertEqual(scroll.contentOffset.y, 100, "Never correct against the previous page's clamp range")
        XCTAssertNotNil(coordinator.historyViewportAnchor)
        scroll.contentSize.height = 3_000
        XCTAssertTrue(coordinator.applyPendingHistoryViewportAnchor())
        XCTAssertEqual(scroll.contentOffset.y, 900)
        XCTAssertFalse(coordinator.applyPendingHistoryViewportAnchor())
    }

    @MainActor
    func testHistoryAnchorWaitsForAShrinkingNativeContentExtent() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 1_000
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "1", messageID: "83", originalContentY: 1_120)
        let page = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "21",
            frames: ["83": CGRect(x: 0, y: -280, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 720, width: 100, height: 80)], contentHeight: 2_600)
        XCTAssertFalse(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 1_000)
        XCTAssertNotNil(coordinator.historyViewportAnchor)
        scroll.contentSize.height = 2_600
        XCTAssertTrue(coordinator.applyPendingHistoryViewportAnchor())
        XCTAssertEqual(scroll.contentOffset.y, 600)
    }

    @MainActor
    func testHistoryAndKeyboardCorrectionsComposeWithoutUndoingTheResize() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize = CGSize(width: 390, height: 3_000)
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 600
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "81", messageID: "83", originalContentY: 220,
            contentY: 1_020, contentHeight: 3_000)
        coordinator.applyPendingViewportResize()
        scroll.bounds.size.height = 300
        coordinator.resizeViewport(from: 600, to: 300)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 400)
        XCTAssertTrue(coordinator.applyPendingHistoryViewportAnchor())
        XCTAssertEqual(scroll.contentOffset.y, 1_200,
                       "The history delta must preserve the keyboard's 300-point composer-relative shift")
    }

    @MainActor
    func testCancelledHistoryLayoutCannotMoveAnotherChat() {
        let coordinator = ChatTimelineInteractionCoordinator()
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 1_000
        coordinator.scrollView = scroll
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "1", messageID: "3", originalContentY: 80,
            contentY: 900, contentHeight: 2_000)
        coordinator.stopScrolling()
        scroll.contentSize.height = 2_000
        XCTAssertFalse(coordinator.applyPendingHistoryViewportAnchor())
        XCTAssertEqual(scroll.contentOffset.y, 0)
    }

    @MainActor
    func testHistoryCorrectionPreservesNativeMovementAfterCapture() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 160
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "81", messageID: "83", originalContentY: 220)
        let page = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "1",
            frames: ["83": CGRect(x: 0, y: 860, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 1_020, width: 100, height: 80)], contentHeight: 3_000)
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 960, "Keep UIKit's 60-point deceleration movement")
    }

    @MainActor
    func testHistoryCorrectionDoesNotCountMovementAfterGeometryMeasurementTwice() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "81", messageID: "83", originalContentY: 220)
        // SwiftUI measured the new row at offset 100. UIKit can continue the
        // drag before that preference reaches the history correction.
        let page = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "1",
            frames: ["83": CGRect(x: 0, y: 920, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 1_020, width: 100, height: 80)], contentHeight: 3_000)
        scroll.contentOffset.y = 160
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 960,
                       "Apply only the 800-point layout change to the live 160-point offset")
    }

    @MainActor
    func testHistoryCaptureDoesNotMixAnOlderViewportFrameWithTheLiveOffset() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 160
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        let measured = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "81",
            frames: ["83": CGRect(x: 0, y: 120, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 220, width: 100, height: 80)])
        coordinator.messageContentFrames = measured.frames
        coordinator.latestPage = measured
        coordinator.captureHistoryViewportAnchor(chatID: "chat", firstMessageID: "81",
                                                  viewportMinY: 0, viewportMaxY: 600)
        XCTAssertEqual(coordinator.historyViewportAnchor?.originalContentY, 220,
                       "Capture intrinsic geometry without adding movement since its measurement")
        let page = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "1",
            contentFrames: ["83": CGRect(x: 0, y: 1_020, width: 100, height: 80)], contentHeight: 3_000)
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 960)
    }

    @MainActor
    func testHistoryDeltaDoesNotIncludeAnAutomaticExtentClamp() throws {
        let window = try makeWindow()
        defer { window.isHidden = true }
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentInsetAdjustmentBehavior = .never
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 2_200
        window.rootViewController!.view.addSubview(scroll)
        let coordinator = ChatTimelineInteractionCoordinator()
        let observer = ChatTimelineScrollObserverView()
        observer.timelineCoordinator = coordinator
        scroll.addSubview(observer)
        observer.bindToEnclosingScrollView()
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "1", messageID: "83", originalContentY: 2_300)
        scroll.contentSize.height = 2_600
        scroll.layoutIfNeeded()
        XCTAssertEqual(scroll.contentOffset.y, 2_000, "Exercise a real UIKit extent clamp before the layout delta")
        let page = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "21",
            frames: ["83": CGRect(x: 0, y: 1_900 - scroll.contentOffset.y, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 1_900, width: 100, height: 80)],
            contentHeight: 2_600)
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 1_800, "Apply the -400 layout delta to the pre-clamp position")
        observer.unbind()
    }

    @MainActor
    func testHistoryAnchorOnlyConsumesGeometryFromThePublishedOlderPage() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize = CGSize(width: 390, height: 3_000)
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "chat", firstMessageID: "81", messageID: "83", originalContentY: 220)
        let old = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "81",
            frames: ["83": CGRect(x: 0, y: 120, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 220, width: 100, height: 80)])
        XCTAssertFalse(coordinator.restoreHistoryViewportAnchor(page: old))
        XCTAssertNotNil(coordinator.historyViewportAnchor)
        let loaded = ChatTimelinePageFrames(chatID: "chat", firstMessageID: "1",
            frames: ["83": CGRect(x: 0, y: 920, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 1_020, width: 100, height: 80)], contentHeight: 3_000)
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: loaded))
        XCTAssertEqual(scroll.contentOffset.y, 900)
        XCTAssertNil(coordinator.historyViewportAnchor)
        XCTAssertFalse(coordinator.restoreHistoryViewportAnchor(page: loaded))
        XCTAssertEqual(scroll.contentOffset.y, 900, "A repeated layout must not apply the offset twice")
    }

    @MainActor
    func testBottomAlignmentUsesInsetsAndFallsBackWhenEstimatedEndCannotReachLastRow() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize = CGSize(width: 390, height: 1_000)
        scroll.contentInset.bottom = 80
        scroll.contentOffset.y = 400
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        XCTAssertTrue(coordinator.alignTimelineBottom(frame: CGRect(x: 0, y: 570, width: 100, height: 30),
                                                     viewportMaxY: 520, bottomSpacing: 0, animated: false))
        XCTAssertEqual(scroll.contentOffset.y, 480)
        XCTAssertTrue(coordinator.alignTimelineBottom(frame: CGRect(x: 0, y: 490, width: 100, height: 30),
                                                     viewportMaxY: 520, bottomSpacing: 0, animated: false))
        XCTAssertEqual(scroll.contentOffset.y, 480)
        XCTAssertFalse(coordinator.alignTimelineBottom(frame: CGRect(x: 0, y: 650, width: 100, height: 30),
                                                      viewportMaxY: 520, bottomSpacing: 0, animated: false))
    }

    func testKeyboardAnimationKeepsFullSystemTimingIndependentOfLayoutDelay() throws {
        let notification = Notification(name: UIResponder.keyboardWillChangeFrameNotification, userInfo: [
            UIResponder.keyboardAnimationDurationUserInfoKey: NSNumber(value: 0.35),
            UIResponder.keyboardAnimationCurveUserInfoKey: NSNumber(value: 7)
        ])
        let animation = try XCTUnwrap(ChatKeyboardAnimation(notification: notification, now: 10))
        XCTAssertEqual(animation.duration, 0.35)
        XCTAssertEqual(animation.remainingDuration(at: 10.1), 0.25, accuracy: 0.0001)
        XCTAssertEqual(animation.remainingDuration(at: 11), 0)
        XCTAssertFalse(animation.options.contains(.beginFromCurrentState), "The original anchor supplies the from-values")
        XCTAssertTrue(animation.options.contains(.allowUserInteraction))
        XCTAssertTrue(animation.options.contains(.overrideInheritedDuration))
        XCTAssertTrue(animation.options.contains(.overrideInheritedCurve))
        XCTAssertEqual(animation.options.rawValue & (7 << 16), 7 << 16)

        let interactive = Notification(name: notification.name, userInfo: [
            UIResponder.keyboardAnimationDurationUserInfoKey: NSNumber(value: 0),
            UIResponder.keyboardAnimationCurveUserInfoKey: NSNumber(value: 7)
        ])
        XCTAssertEqual(try XCTUnwrap(ChatKeyboardAnimation(notification: interactive, now: 10)).remainingDuration(at: 10), 0)
        XCTAssertNil(ChatKeyboardAnimation(notification: nil, now: 10))
    }

    @MainActor
    func testBackdatingPreservesUIKitGeneratedGeometryAnimations() throws {
        let window = try makeWindow()
        defer { window.isHidden = true }
        // Curve 7 is supplied by the keyboard. Also exercise a public UIKit
        // curve so the test does not depend on the keyboard's animation class.
        for curve in [7, UIView.AnimationCurve.easeInOut.rawValue] {
            let view = UIView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
            window.rootViewController!.view.addSubview(view)
            defer { view.removeFromSuperview() }
            let clock = CACurrentMediaTime()
            let keyboard = try XCTUnwrap(ChatKeyboardAnimation(notification: keyboardNotification(curve: curve), now: clock - 0.1))
            // Exercise conversion through a nontrivial layer-local clock.
            view.layer.beginTime = clock - 2
            view.layer.speed = 0.5
            view.layer.timeOffset = 3
            UIView.animate(withDuration: keyboard.duration, delay: 0, options: keyboard.options) {
                view.frame.size.height = 300
                view.bounds.origin.y = 300
            }
            let keys = chatTimelineGeometryAnimationKeys(in: view.layer)
            XCTAssertFalse(keys.isEmpty, "The assertion must inspect real UIKit-generated animations")
            let original = try keys.map { try XCTUnwrap(view.layer.animation(forKey: $0)?.copy() as? CAAnimation) }
            keyboard.alignGeometryAnimations(in: view.layer)
            for (key, before) in zip(keys, original) {
                let after = try XCTUnwrap(view.layer.animation(forKey: key))
                XCTAssertEqual(after.beginTime, view.layer.convertTime(keyboard.startedAt, from: nil), accuracy: 0.000001)
                assertSameAnimationExceptStart(before, after)
            }
        }
    }

    @MainActor
    func testDelayedAndCoalescedLayoutsKeepOriginalAnimationPhase() throws {
        let window = try makeWindow()
        defer { window.isHidden = true }
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        window.rootViewController!.view.addSubview(scroll)
        scroll.contentSize = CGSize(width: 390, height: 2_000)
        scroll.contentOffset.y = 1_000
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 600
        let startedAt = CACurrentMediaTime() - 0.1
        coordinator.captureKeyboardViewportAnchor(notification: keyboardNotification(curve: 0), now: startedAt)

        let resizes: [(CGFloat, CGFloat)] = [(600, 300), (300, 350)]
        for (previous, target) in resizes {
            UIView.performWithoutAnimation { scroll.frame.size.height = target }
            coordinator.resizeViewport(from: previous, to: target)
            coordinator.applyPendingViewportResize()
            XCTAssertEqual(scroll.contentOffset.y, 1_600 - target)
            let keys = chatTimelineGeometryAnimationKeys(in: scroll.layer)
            XCTAssertFalse(keys.isEmpty)
            for key in keys {
                let animation = try XCTUnwrap(scroll.layer.animation(forKey: key))
                XCTAssertEqual(animation.duration, 0.35, accuracy: 0.000001, "Layout delay must not compress the easing curve")
                XCTAssertEqual(animation.beginTime, scroll.layer.convertTime(startedAt, from: nil), accuracy: 0.000001,
                               "A coalesced layout must not restart the curve")
            }
        }
    }

    func testInterruptedTransitionUsesVisibleBoundsAndViewportTogether() {
        let anchor = ChatKeyboardViewportAnchor(
            bounds: CGRect(x: 0, y: 1_300, width: 390, height: 300),
            frame: CGRect(x: 0, y: 0, width: 390, height: 300), viewportHeight: 300, animation: nil,
            presentationBounds: CGRect(x: 0, y: 1_150, width: 390, height: 450),
            presentationFrame: CGRect(x: 0, y: 0, width: 390, height: 450)
        )
        XCTAssertEqual(anchor.viewportHeight, 450)
        XCTAssertEqual(anchor.bounds.height, anchor.frame.height)
        var reopened = anchor.bounds
        reopened.size.height += 600 - anchor.viewportHeight
        XCTAssertEqual(reopened.height, 600)
        XCTAssertEqual(chatTimelineOffsetAfterResize(oldBounds: anchor.bounds, newBounds: reopened,
                                                     contentHeight: 2_000, inset: .zero), 1_000)
    }

    @MainActor
    func testKeyboardHideCapturesVisibleBoundsAfterUIKitAlreadyExpanded() {
        // Exact traced order: model height 759 before will-change, visible
        // height 465, preference still 468. Adding 294 to model height would
        // wait forever for an impossible 1053-point native viewport.
        let visible = CGRect(x: 0, y: 3_410, width: 393, height: 465)
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 393, height: 465))
        scroll.contentSize = CGSize(width: 393, height: 5_000)
        scroll.bounds = visible
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 468
        coordinator.applyPendingViewportResize()
        scroll.frame.size.height = 759
        coordinator.keyboardViewportAnchor = ChatKeyboardViewportAnchor(
            bounds: scroll.bounds, frame: scroll.frame, viewportHeight: coordinator.viewportHeight, animation: nil,
            presentationBounds: visible, presentationFrame: CGRect(x: 0, y: 0, width: 393, height: 465),
            appliedLayout: coordinator.appliedViewportLayout
        )
        XCTAssertEqual(coordinator.keyboardViewportAnchor?.viewportHeight, 468)
        coordinator.resizeViewport(from: 468, to: 762)
        XCTAssertEqual(coordinator.pendingViewportResize?.height, 759)
        coordinator.applyPendingViewportResize()
        XCTAssertNil(coordinator.pendingViewportResize)
        XCTAssertEqual(scroll.contentOffset.y, 3_116, "Hide must reverse the exact 294-point lift")
        XCTAssertEqual(coordinator.appliedViewportLayout?.viewportHeight, 762)
    }

    @MainActor
    func testKeyboardShowRecoversStartAfterPreferenceAndOffsetAlreadyApplied() {
        let visible = CGRect(x: 0, y: 3_116, width: 393, height: 759)
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 393, height: 759))
        scroll.contentSize = CGSize(width: 393, height: 5_000)
        scroll.bounds = visible
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 762
        coordinator.applyPendingViewportResize()
        scroll.frame.size.height = 465
        coordinator.resizeViewport(from: 762, to: 468)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 3_410)
        let anchor = ChatKeyboardViewportAnchor(
            bounds: scroll.bounds, frame: scroll.frame, viewportHeight: coordinator.viewportHeight, animation: nil,
            presentationBounds: visible, presentationFrame: CGRect(x: 0, y: 0, width: 393, height: 759),
            appliedLayout: coordinator.appliedViewportLayout
        )
        XCTAssertEqual(anchor.bounds, visible)
        XCTAssertEqual(anchor.viewportHeight, 762, "A late notification must recover the visible start, not the model target")
    }

    func testPendingPreferenceDoesNotChangeCompletedLayoutReference() {
        let visible = CGRect(x: 0, y: 1_000, width: 390, height: 600)
        let anchor = ChatKeyboardViewportAnchor(
            bounds: visible, frame: CGRect(x: 0, y: 0, width: 390, height: 600),
            viewportHeight: 300, animation: nil,
            presentationBounds: visible, presentationFrame: CGRect(x: 0, y: 0, width: 390, height: 600),
            appliedLayout: ChatTimelineViewportLayout(nativeHeight: 600, viewportHeight: 600)
        )
        XCTAssertEqual(anchor.viewportHeight, 600, "An early preference must not be paired with native bounds from an older layout")
    }

    @MainActor
    func testExplicitSearchJumpSupersedesPendingKeyboardPosition() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 477))
        scroll.contentInset.bottom = 118
        scroll.contentSize.height = 14_500
        scroll.contentOffset.y = 4_857
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 359
        coordinator.applyPendingViewportResize()
        coordinator.captureKeyboardViewportAnchor()
        coordinator.resizeViewport(from: 359, to: 359)

        coordinator.prepareForExplicitScroll()
        scroll.contentOffset.y = 0
        coordinator.recordNativeScrollPosition()
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 0, "An already queued resize must not undo the search jump")

        scroll.bounds.size.height = 778
        coordinator.resizeViewport(from: 359, to: 660)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 0, "Keyboard dismissal must preserve the new target instead of the old bottom")
    }

    @MainActor
    func testExplicitJumpOwnsPositionWhenNativeBoundsCommitFirst() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 477))
        scroll.contentInset.bottom = 118
        scroll.contentSize.height = 14_500
        scroll.contentOffset.y = 4_857
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 359
        coordinator.applyPendingViewportResize()
        coordinator.captureKeyboardViewportAnchor()
        coordinator.prepareForExplicitScroll()

        scroll.bounds.size.height = 778
        scroll.contentOffset.y = 1_200
        coordinator.recordNativeScrollPosition()
        coordinator.resizeViewport(from: 359, to: 660, preservingPosition: false)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_200, "An explicit target owns position when layout commits before its offset")

        coordinator.prepareForExplicitScroll()
        coordinator.resizeViewport(from: 359, to: 660)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_200, "Landing establishes the current layout even before its SwiftUI preference arrives")
    }

    @MainActor
    func testInsetOnlyResizePreservesBrowsingAndLatestPositionsInBothDirections() {
        for initialOffset: CGFloat in [1_000, 2_278] {
            let scroll = UIScrollView(frame: CGRect(x: 0, y: 62, width: 390, height: 778))
            scroll.contentSize.height = 3_000
            scroll.contentInset.bottom = 56
            scroll.contentOffset.y = initialOffset
            let coordinator = ChatTimelineInteractionCoordinator()
            coordinator.scrollView = scroll
            coordinator.viewportHeight = 722
            coordinator.applyPendingViewportResize()
            let observer = ChatTimelineScrollObserverView()
            observer.timelineCoordinator = coordinator
            scroll.addSubview(observer)
            observer.bindToEnclosingScrollView()
            defer { observer.unbind() }
            scroll.contentInset.bottom = 98
            coordinator.resizeViewport(from: 722, to: 680)
            XCTAssertEqual(coordinator.pendingViewportResize?.height, 778)
            coordinator.applyPendingViewportResize()
            XCTAssertNil(coordinator.pendingViewportResize)
            XCTAssertEqual(scroll.contentOffset.y, initialOffset + 42)
            XCTAssertEqual(scroll.bounds.height, 778)
            scroll.contentInset.bottom = 56
            coordinator.resizeViewport(from: 680, to: 722)
            coordinator.applyPendingViewportResize()
            XCTAssertNil(coordinator.pendingViewportResize)
            XCTAssertEqual(scroll.contentOffset.y, initialOffset)
        }
    }

    @MainActor
    func testReplyInsetThenKeyboardEachAdjustsOnlyItsOwnViewportDelta() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 778))
        scroll.contentSize.height = 3_000
        scroll.contentInset.bottom = 56
        scroll.contentOffset.y = 1_000
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 722
        coordinator.applyPendingViewportResize()
        scroll.contentInset.bottom = 98
        coordinator.resizeViewport(from: 722, to: 680)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_042)
        coordinator.captureKeyboardViewportAnchor()
        scroll.frame.size.height = 478
        coordinator.resizeViewport(from: 680, to: 380)
        coordinator.applyPendingViewportResize()
        XCTAssertNil(coordinator.pendingViewportResize)
        XCTAssertEqual(scroll.contentOffset.y, 1_342)
        coordinator.clearKeyboardViewportAnchor()
        coordinator.captureKeyboardViewportAnchor()
        scroll.frame.size.height = 778
        coordinator.resizeViewport(from: 380, to: 680)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_042)
    }

    @MainActor
    func testInteractiveResizeStopsGeometryButPreservesUnrelatedAnimations() throws {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 300))
        scroll.contentSize = CGSize(width: 390, height: 2_000)
        scroll.contentOffset.y = 1_300
        for path in ["bounds.origin", "bounds.size", "position", "opacity"] {
            let animation = CABasicAnimation(keyPath: path)
            animation.duration = 10
            scroll.layer.add(animation, forKey: path)
        }
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 300
        // Use model geometry here; presentation capture has its own test.
        coordinator.keyboardViewportAnchor = ChatKeyboardViewportAnchor(
            bounds: scroll.bounds, frame: scroll.frame, viewportHeight: 300,
            animation: try XCTUnwrap(ChatKeyboardAnimation(notification: keyboardNotification(duration: 0), now: CACurrentMediaTime()))
        )
        scroll.frame.size.height = 600
        coordinator.resizeViewport(from: 300, to: 600)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_000)
        XCTAssertTrue(chatTimelineGeometryAnimationKeys(in: scroll.layer).isEmpty)
        XCTAssertNotNil(scroll.layer.animation(forKey: "opacity"))
    }

    private func keyboardNotification(duration: TimeInterval = 0.35, curve: Int = 7) -> Notification {
        Notification(name: UIResponder.keyboardWillChangeFrameNotification, userInfo: [
            UIResponder.keyboardAnimationDurationUserInfoKey: NSNumber(value: duration),
            UIResponder.keyboardAnimationCurveUserInfoKey: NSNumber(value: curve)
        ])
    }

    @MainActor
    private func makeWindow() throws -> UIWindow {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 700)
        window.rootViewController = UIViewController()
        window.isHidden = false
        window.rootViewController!.view.layoutIfNeeded()
        return window
    }

    private func assertSameAnimationExceptStart(_ before: CAAnimation, _ after: CAAnimation, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(NSStringFromClass(type(of: before)), NSStringFromClass(type(of: after)), file: file, line: line)
        XCTAssertEqual(after.duration, before.duration, file: file, line: line)
        XCTAssertEqual(after.speed, before.speed, file: file, line: line)
        XCTAssertEqual(after.timeOffset, before.timeOffset, file: file, line: line)
        XCTAssertEqual(after.repeatCount, before.repeatCount, file: file, line: line)
        XCTAssertEqual(after.repeatDuration, before.repeatDuration, file: file, line: line)
        XCTAssertEqual(after.autoreverses, before.autoreverses, file: file, line: line)
        XCTAssertEqual(after.fillMode, before.fillMode, file: file, line: line)
        XCTAssertEqual(after.isRemovedOnCompletion, before.isRemovedOnCompletion, file: file, line: line)
        XCTAssertEqual(after.timingFunction, before.timingFunction, file: file, line: line)
        let beforeProperty = before as? CAPropertyAnimation
        let afterProperty = after as? CAPropertyAnimation
        XCTAssertEqual(afterProperty?.keyPath, beforeProperty?.keyPath, file: file, line: line)
        XCTAssertEqual(afterProperty?.isAdditive, beforeProperty?.isAdditive, file: file, line: line)
        XCTAssertEqual(afterProperty?.isCumulative, beforeProperty?.isCumulative, file: file, line: line)
        let beforeBasic = before as? CABasicAnimation
        let afterBasic = after as? CABasicAnimation
        XCTAssertEqual(afterBasic?.fromValue as? NSObject, beforeBasic?.fromValue as? NSObject, file: file, line: line)
        XCTAssertEqual(afterBasic?.toValue as? NSObject, beforeBasic?.toValue as? NSObject, file: file, line: line)
        XCTAssertEqual(afterBasic?.byValue as? NSObject, beforeBasic?.byValue as? NSObject, file: file, line: line)
        if let beforeSpring = before as? CASpringAnimation, let afterSpring = after as? CASpringAnimation {
            XCTAssertEqual(afterSpring.mass, beforeSpring.mass, file: file, line: line)
            XCTAssertEqual(afterSpring.stiffness, beforeSpring.stiffness, file: file, line: line)
            XCTAssertEqual(afterSpring.damping, beforeSpring.damping, file: file, line: line)
            XCTAssertEqual(afterSpring.initialVelocity, beforeSpring.initialVelocity, file: file, line: line)
        }
    }

    func testKeyboardResizePreservesDistanceFromBottomAndReversesOnHide() {
        for offset: CGFloat in [500, 1_400] {
            let old = CGRect(x: 0, y: offset, width: 390, height: 600)
            let smaller = CGRect(x: 0, y: offset, width: 390, height: 300)
            let lifted = chatTimelineOffsetAfterResize(oldBounds: old, newBounds: smaller, contentHeight: 2_000, inset: .zero)
            XCTAssertEqual(lifted, offset + 300)
            let reopened = CGRect(x: 0, y: lifted, width: 390, height: 300)
            XCTAssertEqual(chatTimelineOffsetAfterResize(oldBounds: reopened, newBounds: old, contentHeight: 2_000, inset: .zero), offset)
        }
    }

    func testResizeClampsShortConversationAndIgnoresAlreadyAdjustedOrigin() {
        let old = CGRect(x: 0, y: 800, width: 390, height: 300)
        let alreadyClamped = CGRect(x: 0, y: 400, width: 390, height: 700)
        XCTAssertEqual(chatTimelineOffsetAfterResize(oldBounds: old, newBounds: alreadyClamped, contentHeight: 1_100, inset: .zero), 400)
        XCTAssertEqual(chatTimelineOffsetAfterResize(oldBounds: old, newBounds: alreadyClamped, contentHeight: 100, inset: .zero), 0)
    }

    @MainActor
    func testKeyboardAnchorSurvivesUIKitResizingFirstAndCoalescesUpdates() {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize = CGSize(width: 390, height: 2_000)
        scroll.contentOffset.y = 1_000
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 600
        coordinator.captureKeyboardViewportAnchor()

        scroll.frame.size.height = 500
        coordinator.resizeViewport(from: 600, to: 500)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_100, "The first committed viewport applies its exact delta")
        scroll.frame.size.height = 300
        coordinator.resizeViewport(from: 500, to: 300)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_300, "Coalesced updates preserve the original bottom distance once")
        XCTAssertNil(coordinator.pendingViewportResize)

        coordinator.keyboardViewportAnchor = nil
        coordinator.captureKeyboardViewportAnchor()
        scroll.frame.size.height = 600
        scroll.contentOffset.y = 1_000 // UIKit may already clamp while expanding.
        coordinator.resizeViewport(from: 300, to: 600)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_000, "Keyboard hide must not double-apply UIKit's adjustment")
    }
}
#endif
