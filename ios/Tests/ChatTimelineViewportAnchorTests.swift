#if os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class ChatTimelineViewportAnchorTests: XCTestCase {
    func testKeyboardAnimationKeepsSystemCurveAndUsesOnlyRemainingTime() throws {
        let notification = Notification(name: UIResponder.keyboardWillChangeFrameNotification, userInfo: [
            UIResponder.keyboardAnimationDurationUserInfoKey: NSNumber(value: 0.35),
            UIResponder.keyboardAnimationCurveUserInfoKey: NSNumber(value: 7)
        ])
        let animation = try XCTUnwrap(ChatKeyboardAnimation(notification: notification, now: 10))
        XCTAssertEqual(animation.remainingDuration(at: 10.1), 0.25, accuracy: 0.0001)
        XCTAssertEqual(animation.remainingDuration(at: 11), 0)
        XCTAssertTrue(animation.options.contains(.beginFromCurrentState))
        XCTAssertTrue(animation.options.contains(.allowUserInteraction))
        XCTAssertEqual(animation.options.rawValue & (7 << 16), 7 << 16)

        let interactive = Notification(name: notification.name, userInfo: [
            UIResponder.keyboardAnimationDurationUserInfoKey: NSNumber(value: 0),
            UIResponder.keyboardAnimationCurveUserInfoKey: NSNumber(value: 7)
        ])
        XCTAssertEqual(try XCTUnwrap(ChatKeyboardAnimation(notification: interactive, now: 10)).remainingDuration(at: 10), 0)
        XCTAssertNil(ChatKeyboardAnimation(notification: nil, now: 10))
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
