#if os(macOS)
import AppKit
import SwiftUI

/// Native selectable text keeps link cursor rectangles attached to their glyphs.
/// SwiftUI's selectable Text currently shows an I-beam over its link spans too.
struct IrisDesktopLinkedText: NSViewRepresentable {
    let attributed: AttributedString
    let foreground: Color
    let lineLimit: Int?
    @Environment(\.openURL) private var openURL

    func makeNSView(context: Context) -> NSTextView {
        let storage = NSTextStorage()
        let layout = NSLayoutManager()
        let container = NSTextContainer(size: .zero)
        storage.addLayoutManager(layout)
        layout.addTextContainer(container)
        let view = NSTextView(frame: .zero, textContainer: container)
        view.isEditable = false
        view.isSelectable = true
        view.drawsBackground = false
        view.textContainerInset = .zero
        view.textContainer?.lineFragmentPadding = 0
        view.isHorizontallyResizable = false
        view.isVerticallyResizable = true
        view.textContainer?.widthTracksTextView = false
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view.delegate = context.coordinator
        return view
    }

    func updateNSView(_ view: NSTextView, context: Context) {
        context.coordinator.open = { openURL($0) }
        let text = Self.nativeText(attributed, foreground: NSColor(foreground))
        if view.textStorage?.isEqual(to: text) != true {
            view.textStorage?.setAttributedString(text)
        }
        view.linkTextAttributes = [.cursor: NSCursor.pointingHand]
        view.textContainer?.maximumNumberOfLines = lineLimit ?? 0
        view.textContainer?.lineBreakMode = lineLimit == nil ? .byWordWrapping : .byTruncatingTail
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSTextView, context: Context) -> CGSize? {
        guard let container = nsView.textContainer, let layout = nsView.layoutManager else { return nil }
        let width = max(1, proposal.width ?? 480)
        container.containerSize = CGSize(width: width, height: .greatestFiniteMagnitude)
        layout.ensureLayout(for: container)
        let glyphBounds = layout.boundingRect(forGlyphRange: layout.glyphRange(for: container), in: container)
        let used = layout.usedRect(for: container)
        return CGSize(width: min(width, ceil(glyphBounds.maxX)), height: ceil(max(used.maxY, glyphBounds.maxY)))
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var open: ((URL) -> Void)?

        func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
            guard let url = link as? URL ?? (link as? String).flatMap(URL.init(string:)) else { return false }
            open?(url)
            return true
        }
    }

    static func nativeText(_ attributed: AttributedString, foreground: NSColor) -> NSAttributedString {
        let result = NSMutableAttributedString(attributedString: NSAttributedString(attributed))
        let baseFont = NSFont.preferredFont(forTextStyle: .body)
        let font = baseFont.fontDescriptor.withDesign(.rounded).flatMap { NSFont(descriptor: $0, size: baseFont.pointSize) } ?? baseFont
        result.addAttributes([.font: font, .foregroundColor: foreground], range: NSRange(location: 0, length: result.length))
        for run in attributed.runs {
            let range = NSRange(run.range, in: attributed)
            if let color = run.foregroundColor { result.addAttribute(.foregroundColor, value: NSColor(color), range: range) }
            if run.link != nil { result.addAttribute(.underlineStyle, value: NSUnderlineStyle.single.rawValue, range: range) }
            if run.inlinePresentationIntent?.contains(.stronglyEmphasized) == true {
                result.addAttribute(.font, value: NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask), range: range)
            }
        }
        return result
    }
}
#endif
