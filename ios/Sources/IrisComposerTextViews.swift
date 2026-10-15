import Foundation
import SwiftUI
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif

#if os(iOS)
struct IrisUIKitComposerTextView: UIViewRepresentable {
    private static weak var activeTextView: UITextView?

    static var currentText: String? {
        activeTextView?.text
    }

    @Binding var text: String
    @Binding var isFocused: Bool
    var onPasteAttachments: ((IrisClipboardAttachments) -> Void)? = nil
    var onCompletionUpdate: ((IrisComposerCompletionContext) -> Void)? = nil
    var onCompletionSelectionChange: ((String, NSRange) -> Void)? = nil
    var onCompletionCommand: ((IrisComposerCompletionCommand) -> Bool)? = nil
    var hasCompletions: (() -> Bool)? = nil

    func makeUIView(context: Context) -> UITextView {
        // Explicit legacy layout keeps large paragraphs responsive while
        // retaining native keyboard, selection and undo behavior.
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        let container = NSTextContainer()
        storage.addLayoutManager(layoutManager)
        layoutManager.addTextContainer(container)
        // Use the designated initializer so Swift initializes the subclass's
        // stored properties along with the native text view.
        let textView = IrisComposerUITextView(frame: .zero, textContainer: container)
        textView.onPasteAttachments = onPasteAttachments
        textView.onCompletionCommand = onCompletionCommand
        textView.hasCompletions = hasCompletions
        Self.activeTextView = textView
        textView.delegate = context.coordinator
        textView.backgroundColor = .clear
        textView.font = UIFont.preferredFont(forTextStyle: .body)
        textView.adjustsFontForContentSizeCategory = true
        textView.textColor = UIColor.label
        textView.tintColor = UIColor.tintColor
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        // sizeThatFits owns the one-to-five-line height. Keep scrolling enabled
        // before insertion so a large paste does not change layout modes.
        textView.isScrollEnabled = true
        textView.scrollsToTop = false
        textView.returnKeyType = .default
        textView.keyboardDismissMode = .interactive
        textView.autocapitalizationType = .sentences
        textView.allowsEditingTextAttributes = false
        // Keep the native defaults so the user's keyboard preferences apply.
        textView.accessibilityIdentifier = "chatMessageInput"
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textView.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return textView
    }

    func updateUIView(_ uiView: UITextView, context: Context) {
        (uiView as? IrisComposerUITextView)?.onPasteAttachments = onPasteAttachments
        (uiView as? IrisComposerUITextView)?.onCompletionCommand = onCompletionCommand
        (uiView as? IrisComposerUITextView)?.hasCompletions = hasCompletions
        Self.activeTextView = uiView
        context.coordinator.parent = self
        var needsSelectionReveal = false
        if uiView.markedTextRange == nil, uiView.text != text {
            needsSelectionReveal = true
            let selectedRange = uiView.selectedRange
            uiView.text = text
            let textLength = (text as NSString).length
            if uiView.isFirstResponder, selectedRange.location <= textLength {
                uiView.selectedRange = NSRange(
                    location: selectedRange.location,
                    length: min(selectedRange.length, textLength - selectedRange.location)
                )
            } else {
                uiView.selectedRange = NSRange(location: textLength, length: 0)
            }
        }
        if needsSelectionReveal { (uiView as? IrisComposerUITextView)?.revealSelectionAfterNextLayout() }
        if isFocused && !uiView.isFirstResponder {
            DispatchQueue.main.async { [weak uiView, weak coordinator = context.coordinator] in
                guard let uiView, coordinator?.parent.isFocused == true else { return }
                uiView.becomeFirstResponder()
            }
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        let width = proposal.width ?? uiView.bounds.width
        guard width > 0 else { return nil }
        let height = min(max(measuredHeight(for: uiView, width: width), minHeight(for: uiView)), maxHeight(for: uiView))
        return CGSize(width: width, height: height)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    private func measuredHeight(for textView: UITextView, width: CGFloat) -> CGFloat {
        let measurement = (textView as? IrisComposerUITextView)?.composerMeasurement ?? IrisComposerTextMeasurement()
        return measurement.height(for: textView.textStorage, width: width, lineHeight: minHeight(for: textView))
    }

    private func minHeight(for textView: UITextView) -> CGFloat {
        ceil((textView.font ?? UIFont.preferredFont(forTextStyle: .body)).lineHeight)
    }

    private func maxHeight(for textView: UITextView) -> CGFloat {
        ceil((textView.font ?? UIFont.preferredFont(forTextStyle: .body)).lineHeight * 5)
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: IrisUIKitComposerTextView

        init(parent: IrisUIKitComposerTextView) {
            self.parent = parent
        }

        func textView(
            _ textView: UITextView,
            shouldChangeTextIn range: NSRange,
            replacementText text: String
        ) -> Bool {
            // Let UIKit apply the edit before publishing it back to SwiftUI.
            // Publishing here makes `updateUIView` race the in-flight selection.
            return true
        }

        func textViewDidChange(_ textView: UITextView) {
            if parent.text != textView.text { parent.text = textView.text }
            (textView as? IrisComposerUITextView)?.revealSelectionAfterNextLayout()
            publishCompletion(textView)
        }

        func textViewDidChangeSelection(_ textView: UITextView) {
            parent.onCompletionSelectionChange?(textView.text ?? "", textView.selectedRange)
        }

        private func publishCompletion(_ textView: UITextView) {
            let text = textView.text ?? ""
            let selection = textView.selectedRange
            parent.onCompletionUpdate?(IrisComposerCompletionContext(
                text: text, selection: selection, hasMarkedText: textView.markedTextRange != nil,
                isUndoOrRedo: textView.undoManager?.isUndoing == true || textView.undoManager?.isRedoing == true,
                replace: { [weak textView, weak self] range, replacement in
                    guard let textView, textView.text == text, textView.selectedRange == selection,
                          textView.markedTextRange == nil,
                          let start = textView.position(from: textView.beginningOfDocument, offset: range.location),
                          let end = textView.position(from: start, offset: range.length),
                          let nativeRange = textView.textRange(from: start, to: end) else { return false }
                    textView.replace(nativeRange, withText: replacement)
                    textView.selectedRange = NSRange(location: range.location + replacement.utf16.count, length: 0)
                    self?.textViewDidChange(textView)
                    return true
                }
            ))
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            parent.isFocused = true
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            parent.isFocused = false
        }
    }
}
#endif

#if canImport(AppKit)
struct IrisAppKitComposerTextView: NSViewRepresentable {
    private static weak var activeTextView: NSTextView?

    static var currentText: String? {
        activeTextView?.string
    }

    @discardableResult
    static func insertTextAtSelection(_ replacement: String) -> String? {
        guard let textView = activeTextView else {
            return nil
        }
        return insertTextAtSelection(replacement, into: textView)
    }

    @discardableResult
    static func insertTextAtSelection(_ replacement: String, into textView: NSTextView) -> String {
        textView.insertText(replacement, replacementRange: textView.selectedRange())
        return textView.string
    }

    @Binding var text: String
    @Binding var isFocused: Bool
    let onSubmit: (String) -> IrisComposerSubmitResult
    var onPasteAttachments: ((IrisClipboardAttachments) -> Void)? = nil
    var onCompletionUpdate: ((IrisComposerCompletionContext) -> Void)? = nil
    var onCompletionSelectionChange: ((String, NSRange) -> Void)? = nil
    var onCompletionCommand: ((IrisComposerCompletionCommand) -> Bool)? = nil

    func makeNSView(context: Context) -> IrisComposerScrollView {
        let scrollView = IrisComposerScrollView()
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasVerticalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.verticalScrollElasticity = .none
        scrollView.setAccessibilityIdentifier("chatMessageInput")

        let textView = IrisComposerNSTextView()
        textView.onPasteAttachments = onPasteAttachments
        textView.onCompletionCommand = onCompletionCommand
        Self.activeTextView = textView
        textView.drawsBackground = false
        textView.backgroundColor = .clear
        textView.font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        textView.textColor = .labelColor
        textView.insertionPointColor = .controlAccentColor
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.heightTracksTextView = false
        textView.isRichText = false
        textView.importsGraphics = false
        textView.allowsUndo = true
        textView.isEditable = true
        textView.isSelectable = true
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = false
        textView.minSize = NSSize(width: 0, height: Self.lineHeight(for: textView))
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.autoresizingMask = [.width, .height]
        // Setting spelling options here overrides AppKit's system preferences.
        textView.setAccessibilityIdentifier("chatMessageInput")

        textView.string = text
        textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
        textView.composerCommandDelegate = context.coordinator
        textView.delegate = context.coordinator

        scrollView.documentView = textView
        textView.composerFocusRequested = isFocused
        scrollView.revealSelectionAfterNextLayout(in: textView)
        return scrollView
    }

    func updateNSView(_ nsView: IrisComposerScrollView, context: Context) {
        guard let textView = nsView.documentView as? IrisComposerNSTextView else {
            return
        }

        Self.activeTextView = textView
        context.coordinator.parent = self
        textView.onPasteAttachments = onPasteAttachments
        textView.onCompletionCommand = onCompletionCommand
        textView.composerCommandDelegate = context.coordinator
        textView.delegate = context.coordinator

        let nativeText = textView.string
        let nativeSelection = textView.selectedRange()
        context.coordinator.reconcile(textView)

        nsView.needsLayout = true
        if textView.string != nativeText || textView.selectedRange() != nativeSelection {
            nsView.revealSelectionAfterNextLayout(in: textView)
        }

        textView.composerFocusRequested = isFocused
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: IrisComposerScrollView, context: Context) -> CGSize? {
        guard let textView = nsView.documentView as? NSTextView else {
            return nil
        }
        return Self.fittingSize(
            for: textView,
            proposedWidth: proposal.width,
            actualWidth: nsView.contentView.bounds.width
        )
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    final class Coordinator: NSObject, NSTextViewDelegate, IrisComposerNSTextViewCommandDelegate {
        var parent: IrisAppKitComposerTextView
        var lastPublishedNativeText: String?

        init(parent: IrisAppKitComposerTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else {
                return
            }
            let nativeText = textView.string
            let shouldPublish = parent.text != nativeText
            lastPublishedNativeText = shouldPublish ? nativeText : nil

            if let scrollView = textView.enclosingScrollView as? IrisComposerScrollView {
                scrollView.revealSelectionAfterNextLayout(in: textView)
            } else {
                textView.enclosingScrollView?.needsLayout = true
            }

            if shouldPublish {
                parent.text = nativeText
            }
            publishCompletion(textView)
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            parent.onCompletionSelectionChange?(textView.string, textView.selectedRange())
        }

        private func publishCompletion(_ textView: NSTextView) {
            let text = textView.string
            let selection = textView.selectedRange()
            parent.onCompletionUpdate?(IrisComposerCompletionContext(
                text: text, selection: selection, hasMarkedText: textView.hasMarkedText(),
                isUndoOrRedo: textView.undoManager?.isUndoing == true || textView.undoManager?.isRedoing == true,
                replace: { [weak textView] range, replacement in
                    guard let textView, textView.string == text, textView.selectedRange() == selection,
                          !textView.hasMarkedText() else { return false }
                    textView.breakUndoCoalescing()
                    textView.insertText(replacement, replacementRange: range)
                    return true
                }
            ))
        }

        func reconcile(_ textView: NSTextView) {
            irisReconcileComposerText(
                textView,
                bindingText: parent.text,
                lastPublishedNativeText: &lastPublishedNativeText
            )
        }

        func textDidBeginEditing(_ notification: Notification) {
            parent.isFocused = true
        }

        func textDidEndEditing(_ notification: Notification) {
            parent.isFocused = false
        }

        func composerTextViewDidSubmit(_ textView: NSTextView) {
            guard !textView.hasMarkedText() else {
                return
            }

            guard parent.onSubmit(textView.string) == .acceptedAndClear else {
                return
            }

            lastPublishedNativeText = nil
            textView.string = ""
            textView.setSelectedRange(NSRange(location: 0, length: 0))
            textView.enclosingScrollView?.needsLayout = true
        }
    }
}

#endif


