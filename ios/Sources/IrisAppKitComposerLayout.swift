#if canImport(AppKit)
import AppKit

final class IrisComposerScrollView: NSScrollView {
    override func layout() {
        super.layout()

        guard let textView = documentView as? NSTextView else {
            return
        }
        let viewportSize = contentView.bounds.size
        guard viewportSize.width.isFinite, viewportSize.width > 0 else {
            return
        }

        var documentFrame = textView.frame
        if documentFrame.width != viewportSize.width {
            documentFrame.size.width = viewportSize.width
            textView.setFrameSize(documentFrame.size)
        }

        guard let layoutManager = textView.layoutManager,
              let textContainer = textView.textContainer else {
            return
        }
        layoutManager.ensureLayout(for: textContainer)
        let lineHeight = IrisAppKitComposerTextView.lineHeight(for: textView)
        let usedRect = layoutManager.usedRect(for: textContainer)
        let extraLineRect = layoutManager.extraLineFragmentRect
        let contentHeight = ceil(max(usedRect.maxY, extraLineRect.maxY, lineHeight))
        let isOverflowing = contentHeight > IrisAppKitComposerTextView.maxHeight(for: textView)
        let documentHeight = isOverflowing ? contentHeight : viewportSize.height

        documentFrame = textView.frame
        if documentFrame.height != documentHeight {
            documentFrame.size.height = documentHeight
            textView.setFrameSize(documentFrame.size)
        }
        if hasVerticalScroller != isOverflowing {
            hasVerticalScroller = isOverflowing
        }
        if !isOverflowing, contentView.bounds.origin != .zero {
            contentView.scroll(to: .zero)
            reflectScrolledClipView(contentView)
        }
    }

    func revealSelectionAfterLayout(in textView: NSTextView) {
        layoutSubtreeIfNeeded()
        guard hasVerticalScroller, !textView.hasMarkedText() else { return }
        textView.scrollRangeToVisible(textView.selectedRange())
    }

    func revealSelectionAfterNextLayout(in textView: NSTextView) {
        needsLayout = true
        DispatchQueue.main.async { [weak self, weak textView] in
            guard let self, let textView else { return }
            self.revealSelectionAfterLayout(in: textView)
        }
    }
}

extension IrisAppKitComposerTextView {
    static func fittingSize(
        for textView: NSTextView,
        proposedWidth: CGFloat?,
        actualWidth: CGFloat
    ) -> CGSize? {
        let width: CGFloat
        if let proposedWidth, proposedWidth.isFinite, proposedWidth > 0 {
            width = proposedWidth
        } else if actualWidth.isFinite, actualWidth > 0 {
            width = actualWidth
        } else {
            return nil
        }

        let measuredHeight = textView.attributedString().boundingRect(
            with: NSSize(width: width, height: CGFloat.greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading]
        ).height
        let height = min(
            max(ceil(measuredHeight), lineHeight(for: textView)),
            maxHeight(for: textView)
        )
        return CGSize(width: width, height: height)
    }

    static func lineHeight(for textView: NSTextView) -> CGFloat {
        let font = textView.font ?? NSFont.systemFont(ofSize: NSFont.systemFontSize)
        if let layoutManager = textView.layoutManager {
            return ceil(layoutManager.defaultLineHeight(for: font))
        }
        return ceil(font.ascender - font.descender + font.leading)
    }

    static func maxHeight(for textView: NSTextView) -> CGFloat {
        lineHeight(for: textView) * 5
    }
}
#endif
