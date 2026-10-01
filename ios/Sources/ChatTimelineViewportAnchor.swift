import SwiftUI
#if os(iOS)
import UIKit

struct ChatKeyboardViewportAnchor {
    let bounds: CGRect
    let viewportHeight: CGFloat
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

    func captureKeyboardViewportAnchor() {
#if os(iOS)
        guard let scrollView, viewportHeight > 0 else { return }
        keyboardViewportAnchor = ChatKeyboardViewportAnchor(bounds: scrollView.bounds, viewportHeight: viewportHeight)
#endif
    }

    func applyPendingViewportResize() {
#if os(iOS)
        guard let scrollView, let pendingViewportResize,
              abs(scrollView.bounds.height - pendingViewportResize.height) < 1 else { return }
        self.pendingViewportResize = nil
        scrollView.contentOffset.y = pendingViewportResize.offset
#endif
    }
}
