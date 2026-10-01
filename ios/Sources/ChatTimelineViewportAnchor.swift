import SwiftUI
#if os(iOS)
import UIKit

struct ChatKeyboardViewportAnchor {
    let bounds: CGRect
    let frame: CGRect
    let viewportHeight: CGFloat
    let animation: ChatKeyboardAnimation?
}

struct ChatKeyboardAnimation {
    let startedAt: CFTimeInterval
    let duration: TimeInterval
    let options: UIView.AnimationOptions

    init?(notification: Notification?, now: CFTimeInterval) {
        guard let info = notification?.userInfo,
              let duration = info[UIResponder.keyboardAnimationDurationUserInfoKey] as? NSNumber,
              let curve = info[UIResponder.keyboardAnimationCurveUserInfoKey] as? NSNumber else { return nil }
        self.startedAt = now
        self.duration = max(0, duration.doubleValue)
        self.options = [UIView.AnimationOptions(rawValue: curve.uintValue << 16), .beginFromCurrentState, .allowUserInteraction]
    }

    func remainingDuration(at now: CFTimeInterval) -> TimeInterval {
        max(0, duration - max(0, now - startedAt))
    }
}

/// Keep the same conversation position relative to the composer while the
/// keyboard resizes the viewport. Using the old bounds also avoids applying
/// the offset twice when UIScrollView has already clamped it during a resize.
func chatTimelineOffsetAfterResize(
    oldBounds: CGRect,
    newBounds: CGRect,
    contentHeight: CGFloat,
    inset: UIEdgeInsets
) -> CGFloat {
    let minimum = -inset.top
    let maximum = max(minimum, contentHeight + inset.bottom - newBounds.height)
    return min(maximum, max(minimum, oldBounds.minY + oldBounds.height - newBounds.height))
}
#endif


extension ChatTimelineInteractionCoordinator {
    func resizeViewport(from previousHeight: CGFloat, to height: CGFloat) {
#if os(iOS)
        viewportHeight = height
        guard previousHeight > 0, height > 0, let scrollView else { return }
        let old = keyboardViewportAnchor?.bounds ?? scrollView.bounds
        let previous = keyboardViewportAnchor?.viewportHeight ?? previousHeight
        var resized = old
        resized.size.height += height - previous
        guard resized.height > 0 else { return }
        let offset = chatTimelineOffsetAfterResize(
            oldBounds: old, newBounds: resized,
            contentHeight: scrollView.contentSize.height,
            inset: scrollView.adjustedContentInset
        )
        pendingViewportResize = (resized.height, offset)
        // SwiftUI can publish geometry before UIKit applies its bounds. The
        // content observer's matching layout applies this anchor; the next
        // main turn also handles updates that share the observer's layout.
        DispatchQueue.main.async { [weak self] in
            self?.applyPendingViewportResize()
        }
#endif
    }

#if os(iOS)
    func captureKeyboardViewportAnchor(notification: Notification? = nil, now: CFTimeInterval = CACurrentMediaTime()) {
        guard let scrollView, viewportHeight > 0 else { return }
        keyboardViewportAnchor = ChatKeyboardViewportAnchor(
            bounds: scrollView.bounds, frame: scrollView.frame, viewportHeight: viewportHeight,
            animation: ChatKeyboardAnimation(notification: notification, now: now)
        )
    }
#endif

    func applyPendingViewportResize() {
#if os(iOS)
        guard let scrollView, let pendingViewportResize,
              abs(scrollView.bounds.height - pendingViewportResize.height) < 1 else { return }
        self.pendingViewportResize = nil
        let finalFrame = scrollView.frame
        let update = {
            scrollView.frame = finalFrame
            scrollView.contentOffset.y = pendingViewportResize.offset
        }
        if let animation = keyboardViewportAnchor?.animation,
           animation.remainingDuration(at: CACurrentMediaTime()) > 0 {
            // SwiftUI can resize the clipping viewport without animating its
            // native frame. Animate that frame together with the content so
            // no empty strip opens between the messages and the composer.
            if let anchor = keyboardViewportAnchor,
               scrollView.layer.animation(forKey: "bounds") == nil {
                UIView.performWithoutAnimation {
                    scrollView.frame = anchor.frame
                    scrollView.bounds = anchor.bounds
                }
            }
            // Layout may arrive after the keyboard animation has started. End
            // together instead of restarting its full duration from here.
            UIView.animate(withDuration: animation.remainingDuration(at: CACurrentMediaTime()),
                           delay: 0, options: animation.options, animations: update)
        } else {
            // Interactive keyboard changes have no animation duration.
            update()
        }
#endif
    }
}
