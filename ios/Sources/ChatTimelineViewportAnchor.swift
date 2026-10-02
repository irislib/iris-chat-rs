import SwiftUI
#if os(iOS)
import UIKit

struct ChatTimelineViewportLayout {
    let nativeHeight: CGFloat
    let viewportHeight: CGFloat
}

struct ChatKeyboardViewportAnchor {
    let bounds: CGRect
    let frame: CGRect
    let viewportHeight: CGFloat
    let animation: ChatKeyboardAnimation?

    init(bounds: CGRect, frame: CGRect, viewportHeight: CGFloat, animation: ChatKeyboardAnimation?,
         presentationBounds: CGRect? = nil, presentationFrame: CGRect? = nil,
         appliedLayout: ChatTimelineViewportLayout? = nil) {
        self.bounds = presentationBounds ?? bounds
        self.frame = presentationFrame ?? frame
        // SwiftUI's preference and UIKit's model bounds can change on either
        // side of the keyboard notification. Relate visible geometry to one
        // completed layout, not a mixture of old preference and new bounds.
        self.viewportHeight = presentationBounds.map {
            (appliedLayout?.viewportHeight ?? viewportHeight)
                + $0.height - (appliedLayout?.nativeHeight ?? bounds.height)
        } ?? viewportHeight
        self.animation = animation
    }
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
        self.options = [UIView.AnimationOptions(rawValue: curve.uintValue << 16), .allowUserInteraction,
                        .overrideInheritedDuration, .overrideInheritedCurve]
    }

    func remainingDuration(at now: CFTimeInterval) -> TimeInterval {
        max(0, duration - max(0, now - startedAt))
    }

    func alignGeometryAnimations(in layer: CALayer) {
        // UIKit supplies the keyboard curve (including its spring parameters).
        // Backdate that complete animation instead of restarting its easing
        // over a shorter duration after SwiftUI's layout arrives.
        for key in chatTimelineGeometryAnimationKeys(in: layer) {
            guard let animation = layer.animation(forKey: key)?.copy() as? CAAnimation else { continue }
            animation.beginTime = layer.convertTime(startedAt, from: nil)
            layer.add(animation, forKey: key)
        }
    }
}

func chatTimelineGeometryAnimationKeys(in layer: CALayer) -> [String] {
    (layer.animationKeys() ?? []).filter { key in
        guard let path = (layer.animation(forKey: key) as? CAPropertyAnimation)?.keyPath else { return false }
        return path == "bounds" || path.hasPrefix("bounds.") || path == "position" || path.hasPrefix("position.")
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
        let previousAnchor = keyboardViewportAnchor
        let presentation = scrollView.layer.presentation()
        keyboardViewportAnchor = ChatKeyboardViewportAnchor(
            bounds: scrollView.bounds, frame: scrollView.frame, viewportHeight: viewportHeight,
            animation: ChatKeyboardAnimation(notification: notification, now: now),
            presentationBounds: presentation?.bounds, presentationFrame: presentation?.frame,
            appliedLayout: appliedViewportLayout
        )
        // On show, both geometry and the offset can arrive before will-change.
        // The presentation still contains the visible start. Put that already
        // applied target on the keyboard timeline instead of leaving a jump.
        if previousAnchor == nil, pendingViewportResize == nil,
           let presentation, let layout = appliedViewportLayout,
           abs(layout.nativeHeight - scrollView.bounds.height) < 1,
           abs(presentation.bounds.height - scrollView.bounds.height) > 1 {
            pendingViewportResize = (scrollView.bounds.height, scrollView.contentOffset.y)
            applyPendingViewportResize()
        }
    }

    func clearKeyboardViewportAnchor() {
        keyboardViewportAnchor = nil
    }
#endif

    func applyPendingViewportResize() {
#if os(iOS)
        guard let scrollView else { return }
        if appliedViewportLayout == nil, pendingViewportResize == nil, viewportHeight > 0 {
            appliedViewportLayout = ChatTimelineViewportLayout(nativeHeight: scrollView.bounds.height, viewportHeight: viewportHeight)
        }
        guard let pendingViewportResize,
              abs(scrollView.bounds.height - pendingViewportResize.height) < 1 else { return }
        self.pendingViewportResize = nil
        let finalFrame = scrollView.frame
        let update = {
            scrollView.frame = finalFrame
            scrollView.contentOffset.y = pendingViewportResize.offset
        }
        if let anchor = keyboardViewportAnchor, let animation = anchor.animation,
           animation.remainingDuration(at: CACurrentMediaTime()) > 0 {
            // SwiftUI can resize the clipping viewport without animating its
            // native frame. Animate that frame together with the content so
            // no empty strip opens between the messages and the composer.
            for key in chatTimelineGeometryAnimationKeys(in: scrollView.layer) {
                scrollView.layer.removeAnimation(forKey: key)
            }
            UIView.performWithoutAnimation {
                scrollView.frame = anchor.frame
                scrollView.bounds = anchor.bounds
            }
            UIView.animate(withDuration: animation.duration,
                           delay: 0, options: animation.options, animations: update)
            animation.alignGeometryAnimations(in: scrollView.layer)
        } else {
            // Interactive keyboard changes have no animation duration. End a
            // prior keyboard animation so it cannot fight the current drag.
            if keyboardViewportAnchor != nil {
                for key in chatTimelineGeometryAnimationKeys(in: scrollView.layer) {
                    scrollView.layer.removeAnimation(forKey: key)
                }
            }
            UIView.performWithoutAnimation(update)
        }
        appliedViewportLayout = ChatTimelineViewportLayout(nativeHeight: scrollView.bounds.height, viewportHeight: viewportHeight)
#endif
    }
}
