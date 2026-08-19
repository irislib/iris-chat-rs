#if canImport(AppKit)
import AppKit

func irisReconcileComposerText(
    _ textView: NSTextView,
    bindingText: String,
    lastPublishedNativeText: inout String?
) {
    let nativeText = textView.string
    if nativeText == bindingText {
        lastPublishedNativeText = nil
        return
    }
    guard !textView.hasMarkedText(), lastPublishedNativeText != nativeText else {
        return
    }
    textView.string = bindingText
    textView.setSelectedRange(
        NSRange(location: (bindingText as NSString).length, length: 0)
    )
    lastPublishedNativeText = nil
}
#endif
