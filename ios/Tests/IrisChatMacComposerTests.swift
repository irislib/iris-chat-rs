import XCTest

#if os(macOS)
import AppKit
import SwiftUI
@testable import IrisChatMac

final class IrisChatMacComposerTests: XCTestCase {
    private final class TextBox {
        var value: String

        init(_ value: String) {
            self.value = value
        }
    }

    private func makeCoordinator(
        bindingText: String,
        onSubmit: @escaping (String) -> IrisComposerSubmitResult = { _ in .rejected }
    ) -> (
        box: TextBox,
        coordinator: IrisAppKitComposerTextView.Coordinator
    ) {
        makeCoordinator(box: TextBox(bindingText), onSubmit: onSubmit)
    }

    private func makeCoordinator(
        box: TextBox,
        onSubmit: @escaping (String) -> IrisComposerSubmitResult
    ) -> (
        box: TextBox,
        coordinator: IrisAppKitComposerTextView.Coordinator
    ) {
        let focusState = FocusState<Bool>()
        let parent = IrisAppKitComposerTextView(
            text: Binding(
                get: { box.value },
                set: { box.value = $0 }
            ),
            isFocused: focusState.projectedValue,
            onSubmit: onSubmit
        )
        return (box, parent.makeCoordinator())
    }

    func testMacComposerIgnoresStaleBindingEcho() {
        let harness = makeCoordinator(bindingText: "hell")
        let textView = NSTextView()
        textView.string = "hello"
        textView.setSelectedRange(NSRange(location: 5, length: 0))
        harness.coordinator.lastPublishedNativeText = "hello"

        harness.coordinator.reconcile(textView)

        XCTAssertEqual(textView.string, "hello")
        XCTAssertEqual(textView.selectedRange(), NSRange(location: 5, length: 0))
        XCTAssertEqual(harness.coordinator.lastPublishedNativeText, "hello")
    }

    func testMacComposerIgnoresIntermediateEchoThenAcknowledgesLatestText() {
        let harness = makeCoordinator(bindingText: "ab")
        let textView = NSTextView()
        textView.string = "abc"
        textView.setSelectedRange(NSRange(location: 3, length: 0))
        harness.coordinator.lastPublishedNativeText = "abc"

        harness.coordinator.reconcile(textView)
        XCTAssertEqual(textView.string, "abc")
        XCTAssertEqual(harness.coordinator.lastPublishedNativeText, "abc")

        harness.box.value = "abc"
        harness.coordinator.reconcile(textView)

        XCTAssertEqual(textView.string, "abc")
        XCTAssertNil(harness.coordinator.lastPublishedNativeText)
    }

    func testMacComposerStaleEchoPreservesMiddleSelection() {
        let harness = makeCoordinator(bindingText: "hello")
        let textView = NSTextView()
        textView.string = "heXllo"
        textView.setSelectedRange(NSRange(location: 3, length: 0))
        harness.coordinator.lastPublishedNativeText = "heXllo"

        harness.coordinator.reconcile(textView)

        XCTAssertEqual(textView.string, "heXllo")
        XCTAssertEqual(textView.selectedRange(), NSRange(location: 3, length: 0))
    }

    func testMacComposerMarkedTextBlocksParentReconciliationAndCommittedTextWins() {
        let harness = makeCoordinator(bindingText: "draft")
        let textView = NSTextView()
        textView.string = "draft"
        textView.setSelectedRange(NSRange(location: 0, length: 5))
        textView.setMarkedText(
            "候補",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: 0, length: 5)
        )
        XCTAssertTrue(textView.hasMarkedText())
        let markedText = textView.string
        let markedRange = textView.markedRange()
        let selectedRange = textView.selectedRange()
        harness.box.value = "restored parent"

        harness.coordinator.reconcile(textView)

        XCTAssertEqual(textView.string, markedText)
        XCTAssertEqual(textView.markedRange(), markedRange)
        XCTAssertEqual(textView.selectedRange(), selectedRange)

        textView.unmarkText()
        textView.string = "確定"
        textView.setSelectedRange(NSRange(location: 2, length: 0))
        harness.coordinator.textDidChange(
            Notification(name: NSText.didChangeNotification, object: textView)
        )
        harness.box.value = "候"

        harness.coordinator.reconcile(textView)

        XCTAssertEqual(textView.string, "確定")
        XCTAssertEqual(textView.selectedRange(), NSRange(location: 2, length: 0))
    }

    func testMacComposerEqualDidChangeDoesNotBlockGenuineParentRestore() {
        let harness = makeCoordinator(bindingText: "draft")
        let textView = NSTextView()
        textView.string = "draft"

        harness.coordinator.textDidChange(
            Notification(name: NSText.didChangeNotification, object: textView)
        )
        XCTAssertNil(harness.coordinator.lastPublishedNativeText)

        harness.box.value = "restored 🙂 draft"

        harness.coordinator.reconcile(textView)

        XCTAssertEqual(textView.string, "restored 🙂 draft")
        XCTAssertEqual(
            textView.selectedRange(),
            NSRange(location: (textView.string as NSString).length, length: 0)
        )
        XCTAssertNil(harness.coordinator.lastPublishedNativeText)
    }

    func testMacComposerAcceptedSubmitUsesNativeTextAndClearsLocally() {
        var submitted: [String] = []
        let box = TextBox("stale")
        let harness = makeCoordinator(box: box) { text in
            submitted.append(text)
            box.value = ""
            return .acceptedAndClear
        }
        let textView = NSTextView()
        textView.string = "exact native text"
        textView.setSelectedRange(NSRange(location: 5, length: 0))
        harness.coordinator.lastPublishedNativeText = textView.string

        harness.coordinator.composerTextViewDidSubmit(textView)

        XCTAssertEqual(submitted, ["exact native text"])
        XCTAssertEqual(textView.string, "")
        XCTAssertEqual(textView.selectedRange(), NSRange(location: 0, length: 0))
        XCTAssertNil(harness.coordinator.lastPublishedNativeText)

        harness.coordinator.reconcile(textView)
        XCTAssertEqual(textView.string, "")
    }

    func testMacComposerRejectedSubmitLeavesNativeTextAndSelectionAlone() {
        var submitted: [String] = []
        let harness = makeCoordinator(bindingText: "stale") { text in
            submitted.append(text)
            return .rejected
        }
        let textView = NSTextView()
        textView.string = "keep this"
        textView.setSelectedRange(NSRange(location: 4, length: 0))

        harness.coordinator.composerTextViewDidSubmit(textView)

        XCTAssertEqual(submitted, ["keep this"])
        XCTAssertEqual(textView.string, "keep this")
        XCTAssertEqual(textView.selectedRange(), NSRange(location: 4, length: 0))
    }

    func testMacComposerEmojiInsertionUsesSelectedCursorPosition() {
        let textView = NSTextView()
        textView.string = "hello world"
        textView.setSelectedRange(NSRange(location: 6, length: 0))

        let updated = IrisAppKitComposerTextView.insertTextAtSelection("🙂", into: textView)

        XCTAssertEqual(updated, "hello 🙂world")
        XCTAssertEqual(textView.selectedRange().location, 6 + ("🙂" as NSString).length)
        XCTAssertEqual(textView.selectedRange().length, 0)
    }

    func testMacComposerEmojiInsertionReplacesSelection() {
        let textView = NSTextView()
        textView.string = "abcdef"
        textView.setSelectedRange(NSRange(location: 2, length: 3))

        let updated = IrisAppKitComposerTextView.insertTextAtSelection("🔥", into: textView)

        XCTAssertEqual(updated, "ab🔥f")
        XCTAssertEqual(textView.selectedRange().location, 2 + ("🔥" as NSString).length)
        XCTAssertEqual(textView.selectedRange().length, 0)
    }
}
#endif
