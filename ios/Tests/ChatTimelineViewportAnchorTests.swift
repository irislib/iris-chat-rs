#if os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class ChatTimelineViewportAnchorTests: XCTestCase {
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
        coordinator.resizeViewport(from: 762, to: 468)
        scroll.frame.size.height = 465
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
    func testReplyInsetBeforeKeyboardDoesNotBecomeCumulativeNativeResize() throws {
        // Reply changes the safe area preference without resizing the native
        // scroll view. The subsequent keyboard change has its own native delta.
        let visible = CGRect(x: 0, y: 249.66666666666666, width: 393, height: 759)
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 393, height: 759))
        scroll.contentSize = CGSize(width: 393, height: 2_000)
        scroll.bounds = visible
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.viewportHeight = 762
        coordinator.applyPendingViewportResize()
        let replyViewport: CGFloat = 683.6666666666667
        let keyboardViewport: CGFloat = 389.6666666666667
        coordinator.resizeViewport(from: 762, to: replyViewport)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.bounds, visible)
        XCTAssertNotNil(coordinator.pendingViewportResize)

        coordinator.resizeViewport(from: replyViewport, to: keyboardViewport)
        XCTAssertEqual(try XCTUnwrap(coordinator.pendingViewportResize).height, 465, accuracy: 0.000001)
        scroll.frame.size.height = 465
        coordinator.applyPendingViewportResize()
        XCTAssertNil(coordinator.pendingViewportResize)
        XCTAssertEqual(scroll.contentOffset.y, visible.minY + 294, accuracy: 0.000001)
        let anchor = ChatKeyboardViewportAnchor(
            bounds: scroll.bounds, frame: scroll.frame, viewportHeight: coordinator.viewportHeight, animation: nil,
            presentationBounds: visible, presentationFrame: CGRect(x: 0, y: 0, width: 393, height: 759),
            appliedLayout: coordinator.appliedViewportLayout
        )
        XCTAssertEqual(anchor.viewportHeight, replyViewport, accuracy: 0.000001)
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

        coordinator.resizeViewport(from: 600, to: 500)
        coordinator.applyPendingViewportResize()
        XCTAssertEqual(scroll.contentOffset.y, 1_000, "The old UIKit layout must not consume the pending anchor")
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
