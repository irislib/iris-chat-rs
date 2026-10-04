import SwiftUI
#if os(iOS)
import UIKit

struct ChatTimelineViewportLayout {
    let nativeHeight: CGFloat
    let viewportHeight: CGFloat
    var offsetY: CGFloat? = nil
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
    func resizeViewport(from previousHeight: CGFloat, to height: CGFloat, preservingPosition: Bool = true) {
#if os(iOS)
        viewportHeight = height
        guard previousHeight > 0, height > 0, let scrollView else { return }
        guard preservingPosition else {
            prepareForExplicitScroll()
            return
        }
        var old = keyboardViewportAnchor?.bounds ?? scrollView.bounds
        if keyboardViewportAnchor == nil, let appliedViewportLayout {
            old.size.height = appliedViewportLayout.nativeHeight
            old.origin.y = appliedViewportLayout.offsetY ?? old.origin.y
        }
        let previous = keyboardViewportAnchor?.viewportHeight ?? appliedViewportLayout?.viewportHeight ?? previousHeight
        // Effective viewport change and native bounds are different inputs:
        // an inset-only change must not invent a taller native clamp range.
        let offset = old.minY + previous - height
        let minimum = -scrollView.adjustedContentInset.top
        let maximum = max(minimum, scrollView.contentSize.height
                          + scrollView.adjustedContentInset.bottom - scrollView.bounds.height)
        // Effective height can change through insets alone. UIKit has already
        // committed its native bounds when this viewport value is published.
        pendingViewportResize = (scrollView.bounds.height, min(maximum, max(minimum, offset)))
        // The content observer and the next main turn cover both a native
        // layout pass and an inset-only update within the same bounds.
        DispatchQueue.main.async { [weak self] in
            self?.applyPendingViewportResize()
        }
#endif
    }

#if os(iOS)
    func prepareForExplicitScroll() {
        // A search, reply, or latest jump supersedes positions captured before
        // it. A later keyboard layout must start from the new native position.
        historyViewportAnchor = nil
        pendingViewportResize = nil
        keyboardViewportAnchor = nil
        if let scrollView, let height = visibleViewportMaxY {
            appliedViewportLayout = ChatTimelineViewportLayout(
                nativeHeight: scrollView.bounds.height, viewportHeight: height,
                offsetY: scrollView.contentOffset.y)
        }
    }

    func recordNativeScrollPosition() {
        anchorTrace?.record(.nativeOffset) { historyTraceSample() }
        guard let scrollView, let layout = appliedViewportLayout,
              abs(scrollView.bounds.height - layout.nativeHeight) < 0.5,
              abs((visibleViewportMaxY ?? 0) - layout.viewportHeight) < 0.5 else { return }
        // Track real dragging and deceleration while this layout is current.
        // A new inset/bounds can clamp the offset before layout notification;
        // that mutation must not overwrite the previous viewport's position.
        appliedViewportLayout?.offsetY = scrollView.contentOffset.y
    }

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
            appliedViewportLayout = ChatTimelineViewportLayout(nativeHeight: scrollView.bounds.height, viewportHeight: viewportHeight, offsetY: scrollView.contentOffset.y)
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
        appliedViewportLayout = ChatTimelineViewportLayout(nativeHeight: scrollView.bounds.height, viewportHeight: viewportHeight, offsetY: scrollView.contentOffset.y)
#endif
    }
}

#if os(iOS)
struct ChatTimelineHistoryAnchor {
    var chatID = ""
    var firstMessageID = ""
    var layoutGeneration = 0
    let messageID: String
    let originalContentY: CGFloat
    var contentY: CGFloat?
    var contentHeight: CGFloat = 0
    var offsetBeforeExtentChange: CGFloat?
    var clampCorrectionY: CGFloat = 0


}

extension ChatTimelineInteractionCoordinator {
    func hasCommittedTimelineExtent(_ height: CGFloat) -> Bool {
        guard height > 0, let scrollView else { return false }
        return abs(scrollView.contentSize.height - height) <= 1
    }

    var visibleViewportMaxY: CGFloat? {
        guard let scrollView else { return nil }
        return scrollView.bounds.height - scrollView.adjustedContentInset.bottom
    }

    // Capture immediately before publishing the older page, not while its
    // database read is pending and the user can continue scrolling.
    func captureHistoryViewportAnchor(chatID: String, firstMessageID: String, layoutGeneration: Int = 0,
                                      viewportMinY: CGFloat, viewportMaxY: CGFloat) {
        historyViewportAnchor = nil
        guard scrollView != nil, latestPage.chatID == chatID else { return }
        let visible = messageContentFrames.filter { _, frame in
            frame.maxY > viewportMinY && frame.minY < viewportMaxY
        }
        guard let (id, _) = visible.min(by: { $0.value.minY < $1.value.minY }),
              let contentFrame = latestPage.contentFrames[id] else { return }
        historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: chatID, firstMessageID: firstMessageID, layoutGeneration: layoutGeneration,
            messageID: id, originalContentY: contentFrame.minY)
        anchorTrace?.begin { historyTraceSample(anchor: historyViewportAnchor) }
    }

    @discardableResult
    func restoreHistoryViewportAnchor(page: ChatTimelinePageFrames) -> Bool {
        guard var anchor = historyViewportAnchor, scrollView != nil,
              page.chatID == anchor.chatID, let first = page.firstMessageID,
              first != anchor.firstMessageID || page.layoutGeneration != anchor.layoutGeneration,
              let frame = page.contentFrames[anchor.messageID] else { return false }
        anchor.contentY = frame.minY
        anchor.contentHeight = page.contentHeight
        historyViewportAnchor = anchor
        anchorTrace?.record(.restore, origin: .preference) {
            var sample = historyTraceSample(anchor: anchor)
            sample.firstChanged = first != anchor.firstMessageID
            sample.generationChanged = page.layoutGeneration != anchor.layoutGeneration
            return sample
        }
        if applyPendingHistoryViewportAnchor(origin: .preference) { return true }
        // Preference geometry can precede UIKit's new content extent. Stage
        // one correction for the completed native layout, never an old extent.
        if !historyLayoutScheduled {
            historyLayoutScheduled = true
            DispatchQueue.main.async { [weak self] in
                self?.historyLayoutScheduled = false
                self?.applyPendingHistoryViewportAnchor(origin: .mainTurn)
            }
        }
        return false
    }

    @discardableResult
    func applyPendingHistoryViewportAnchor(origin: IrisTimelineAnchorTrace.Origin = .direct) -> Bool {
        guard let anchor = historyViewportAnchor, let scrollView,
              let contentY = anchor.contentY else { return false }
        guard hasCommittedTimelineExtent(anchor.contentHeight) else {
            anchorTrace?.record(.awaitExtent, origin: origin) { historyTraceSample(anchor: anchor) }
            return false
        }
        let offset = scrollView.contentOffset.y + contentY - anchor.originalContentY + anchor.clampCorrectionY
        let traced = anchorTrace?.record(.apply, origin: origin) {
            var sample = historyTraceSample(anchor: anchor)
            sample.candidateOffsetY = offset
            return sample
        } == true
        historyViewportAnchor = nil
        if offset.isFinite, abs(offset - scrollView.contentOffset.y) > 1 {
            UIView.performWithoutAnimation { scrollView.contentOffset.y = offset }
        }
        if traced, let trace = anchorTrace {
            let generation = trace.captureGeneration
            DispatchQueue.main.async { [weak self, weak trace] in
                guard let self, let trace, self.anchorTrace === trace,
                      trace.captureGeneration == generation else { return }
                trace.record(.postApply, origin: .mainTurn) {
                    var sample = self.historyTraceSample(anchor: anchor)
                    sample.candidateOffsetY = offset
                    return sample
                }
            }
        }
        return true
    }

    func historyExtentWillChange() {
        guard let scrollView, historyViewportAnchor != nil else { return }
        historyViewportAnchor?.offsetBeforeExtentChange = scrollView.contentOffset.y
        anchorTrace?.record(.extentWill) { historyTraceSample(anchor: historyViewportAnchor) }
    }

    func historyExtentDidChange() {
        guard let scrollView, let before = historyViewportAnchor?.offsetBeforeExtentChange else { return }
        historyViewportAnchor?.offsetBeforeExtentChange = nil
        let minimum = -scrollView.adjustedContentInset.top
        let maximum = max(minimum, scrollView.contentSize.height
                          + scrollView.adjustedContentInset.bottom - scrollView.bounds.height)
        let clamped = min(maximum, max(minimum, before))
        // Extent changes can clamp the offset before our one layout delta.
        // Keep that correction separate from real dragging/deceleration.
        if abs(clamped - before) > 0.5, abs(scrollView.contentOffset.y - clamped) <= 0.5 {
            historyViewportAnchor?.clampCorrectionY += before - clamped
        }
        anchorTrace?.record(.extentDid) {
            var sample = historyTraceSample(anchor: historyViewportAnchor)
            sample.offsetBeforeExtent = before
            sample.candidateOffsetY = clamped
            return sample
        }
    }

    func recordHistoryPanEnded() {
        anchorTrace?.record(.panEnded) { historyTraceSample(anchor: historyViewportAnchor) }
    }

    private func historyTraceSample(anchor: ChatTimelineHistoryAnchor? = nil) -> IrisTimelineAnchorTrace.Sample {
        guard let scrollView else { return IrisTimelineAnchorTrace.Sample() }
        let pan = scrollView.panGestureRecognizer
        var sample = IrisTimelineAnchorTrace.Sample(
            offsetY: scrollView.contentOffset.y, nativeContentHeight: scrollView.contentSize.height,
            viewportHeight: scrollView.bounds.height, insetTop: scrollView.adjustedContentInset.top,
            insetBottom: scrollView.adjustedContentInset.bottom,
            panY: pan.translation(in: scrollView).y, velocityY: pan.velocity(in: scrollView).y,
            panState: pan.state.rawValue, dragging: scrollView.isDragging, decelerating: scrollView.isDecelerating)
        sample.preferenceContentHeight = latestPage.contentHeight
        if let anchor {
            if anchor.contentY != nil { sample.preferenceContentHeight = anchor.contentHeight }
            sample.anchorViewportY = messageContentFrames[anchor.messageID]?.minY ?? .nan
            sample.originalContentY = anchor.originalContentY
            sample.contentY = anchor.contentY ?? .nan
            sample.clampCorrectionY = anchor.clampCorrectionY
            sample.offsetBeforeExtent = anchor.offsetBeforeExtentChange ?? .nan
            sample.extentCommitted = hasCommittedTimelineExtent(sample.preferenceContentHeight)
        }
        return sample
    }

    @discardableResult
    func alignTimelineBottom(frame: CGRect, viewportMaxY: CGFloat,
                             bottomSpacing: CGFloat, animated: Bool) -> Bool {
        guard let scrollView, viewportMaxY > 0, !frame.isEmpty,
              frame.maxY.isFinite else { return false }
        let correction = frame.maxY + bottomSpacing - viewportMaxY
        let minimum = -scrollView.adjustedContentInset.top
        let maximum = max(minimum, scrollView.contentSize.height
                          + scrollView.adjustedContentInset.bottom - scrollView.bounds.height)
        let offset = min(maximum, max(minimum, scrollView.contentOffset.y + correction))
        guard abs(offset - scrollView.contentOffset.y) > 0.5 else {
            // A clamped estimate cannot prove that the last row was realized.
            return abs(correction) <= 1
        }
        scrollView.setContentOffset(CGPoint(x: scrollView.contentOffset.x, y: offset), animated: animated)
        return true
    }
}
#endif
