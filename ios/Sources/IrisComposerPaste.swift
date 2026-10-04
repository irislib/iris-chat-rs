#if os(iOS)
import UIKit

final class IrisComposerUITextView: UITextView {
    let composerMeasurement = IrisComposerTextMeasurement()
    var onPasteAttachments: ((IrisClipboardAttachments) -> Void)?
    private var pendingSelectionReveal: NSRange?
    private var selectionRevealScheduled = false
    #if DEBUG
    var onLayoutTiming: ((String, Double) -> Void)?
    #endif

    func revealSelectionAfterNextLayout() {
        guard isFirstResponder, markedTextRange == nil else { return }
        pendingSelectionReveal = selectedRange
        guard !selectionRevealScheduled else { return }
        selectionRevealScheduled = true
        setNeedsLayout()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.selectionRevealScheduled = false
            let selection = self.pendingSelectionReveal
            self.pendingSelectionReveal = nil
            guard let selection, self.isFirstResponder, self.isScrollEnabled, self.markedTextRange == nil,
                  self.selectedRange == selection else { return }
            #if DEBUG
            let layoutStart = self.onLayoutTiming == nil ? nil : ProcessInfo.processInfo.systemUptime
            #endif
            self.layoutIfNeeded()
            #if DEBUG
            if let layoutStart { self.onLayoutTiming?("deferred-editor-layout", (ProcessInfo.processInfo.systemUptime - layoutStart) * 1_000) }
            let revealStart = self.onLayoutTiming == nil ? nil : ProcessInfo.processInfo.systemUptime
            #endif
            self.scrollRangeToVisible(self.selectedRange)
            #if DEBUG
            if let revealStart { self.onLayoutTiming?("deferred-selection-reveal", (ProcessInfo.processInfo.systemUptime - revealStart) * 1_000) }
            #endif
        }
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)), onPasteAttachments != nil,
           UIPasteboard.general.types(forItemSet: nil)?.contains(where: {
               IrisClipboardAttachments.preferredType($0) != nil
           }) == true { return true }
        return super.canPerformAction(action, withSender: sender)
    }

    override func paste(_ sender: Any?) {
        if pasteAttachments(UIPasteboard.general.itemProviders) { return }
        super.paste(sender)
    }

    override func paste(itemProviders: [NSItemProvider]) {
        if pasteAttachments(itemProviders) { return }
        super.paste(itemProviders: itemProviders)
    }

    override func canPaste(_ itemProviders: [NSItemProvider]) -> Bool {
        if IrisClipboardAttachments(providers: itemProviders) != nil { return onPasteAttachments != nil }
        return super.canPaste(itemProviders)
    }

    @discardableResult
    func pasteAttachments(_ providers: [NSItemProvider]) -> Bool {
        guard let attachments = IrisClipboardAttachments(providers: providers) else { return false }
        onPasteAttachments?(attachments)
        return true
    }
}
#elseif os(macOS)
import AppKit

protocol IrisComposerNSTextViewCommandDelegate: AnyObject {
    func composerTextViewDidSubmit(_ textView: NSTextView)
}

final class IrisComposerNSTextView: NSTextView {
    // SwiftUI may request focus before AppKit has attached this view to a
    // window. Retry on attachment, and discard deferred requests after blur.
    var composerFocusRequested = false {
        didSet { applyComposerFocusIfNeeded() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        applyComposerFocusIfNeeded()
    }

    private func applyComposerFocusIfNeeded() {
        guard composerFocusRequested, window != nil else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.composerFocusRequested, let window = self.window,
                  window.firstResponder !== self else { return }
            window.makeFirstResponder(self)
        }
    }

    let composerMeasurement = IrisComposerTextMeasurement()
    weak var composerCommandDelegate: IrisComposerNSTextViewCommandDelegate?
    var onPasteAttachments: ((IrisClipboardAttachments) -> Void)?

    override func paste(_ sender: Any?) {
        if pasteAttachments(from: .general) { return }
        super.paste(sender)
    }

    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        if item.action == #selector(paste(_:)), onPasteAttachments != nil,
           NSPasteboard.general.pasteboardItems?.contains(where: {
               IrisClipboardAttachments.preferredType($0.types.map(\.rawValue)) != nil
           }) == true { return true }
        return super.validateUserInterfaceItem(item)
    }

    @discardableResult
    func pasteAttachments(from pasteboard: NSPasteboard) -> Bool {
        guard let attachments = IrisClipboardAttachments(pasteboard: pasteboard) else { return false }
        onPasteAttachments?(attachments)
        return true
    }

    override func doCommand(by selector: Selector) {
        if selector == #selector(NSResponder.insertNewline(_:)),
           !hasMarkedText(), !shouldInsertLineBreakForCurrentEvent {
            composerCommandDelegate?.composerTextViewDidSubmit(self)
            return
        }
        super.doCommand(by: selector)
    }

    private var shouldInsertLineBreakForCurrentEvent: Bool {
        guard let event = NSApp.currentEvent, event.type == .keyDown else { return false }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        return flags.contains(.shift) || flags.contains(.option)
    }
}
#endif
