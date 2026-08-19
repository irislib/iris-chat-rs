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

    func testMacComposerFittingSizeDoesNotMutateLiveGeometry() {
        let (scrollView, textView) = makeComposerViews(width: 240, height: 16)
        let lineHeight = IrisAppKitComposerTextView.lineHeight(for: textView)
        textView.string = "first\nsecond"
        textView.setSelectedRange(NSRange(location: 3, length: 2))
        scrollView.layoutSubtreeIfNeeded()

        let originalContainerSize = textView.textContainer?.containerSize
        let originalDocumentFrame = textView.frame
        let originalSelection = textView.selectedRange()
        let originalClipOrigin = scrollView.contentView.bounds.origin

        let zeroProposal = IrisAppKitComposerTextView.fittingSize(
            for: textView,
            proposedWidth: 0,
            actualWidth: 240
        )
        let infiniteProposal = IrisAppKitComposerTextView.fittingSize(
            for: textView,
            proposedWidth: .infinity,
            actualWidth: 240
        )
        let unusableProposal = IrisAppKitComposerTextView.fittingSize(
            for: textView,
            proposedWidth: .infinity,
            actualWidth: 0
        )

        XCTAssertEqual(zeroProposal, CGSize(width: 240, height: lineHeight * 2))
        XCTAssertEqual(infiniteProposal, CGSize(width: 240, height: lineHeight * 2))
        XCTAssertNil(unusableProposal)
        XCTAssertEqual(textView.textContainer?.containerSize, originalContainerSize)
        XCTAssertEqual(textView.frame, originalDocumentFrame)
        XCTAssertEqual(textView.selectedRange(), originalSelection)
        XCTAssertEqual(scrollView.contentView.bounds.origin, originalClipOrigin)
    }

    func testMacComposerScrollLayoutBoundsDocumentAndResetsOverflow() {
        let (scrollView, textView) = makeComposerViews(width: 240, height: 16)
        let lineHeight = IrisAppKitComposerTextView.lineHeight(for: textView)
        scrollView.frame.size.height = lineHeight

        textView.string = "first"
        scrollView.needsLayout = true
        scrollView.layoutSubtreeIfNeeded()
        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight, document: lineHeight)

        textView.string = "first\nsecond"
        scrollView.needsLayout = true
        scrollView.layoutSubtreeIfNeeded()
        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight, document: lineHeight)

        scrollView.frame.size.height = lineHeight * 2
        scrollView.layoutSubtreeIfNeeded()
        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight * 2, document: lineHeight * 2)

        let overflowText = "1\n2\n3\n4\n5\n6"
        let overflowSelection = NSRange(location: 2, length: 1)
        textView.string = overflowText
        textView.setSelectedRange(overflowSelection)
        scrollView.frame.size.height = lineHeight * 5
        scrollView.needsLayout = true
        scrollView.layoutSubtreeIfNeeded()
        assertHeights(
            scrollView: scrollView,
            textView: textView,
            viewport: lineHeight * 5,
            document: lineHeight * 6
        )
        XCTAssertTrue(scrollView.hasVerticalScroller)
        XCTAssertEqual(scrollView.contentView.bounds.origin, .zero)
        XCTAssertEqual(textView.string, overflowText)
        XCTAssertEqual(textView.selectedRange(), overflowSelection)

        textView.setSelectedRange(NSRange(location: (overflowText as NSString).length, length: 0))
        scrollView.revealSelectionAfterLayout(in: textView)
        XCTAssertEqual(scrollView.contentView.bounds.origin.y, lineHeight, accuracy: 0.5)
        scrollView.needsLayout = true
        scrollView.layoutSubtreeIfNeeded()
        XCTAssertEqual(scrollView.contentView.bounds.origin.y, lineHeight, accuracy: 0.5)

        textView.string = "one"
        textView.setSelectedRange(NSRange(location: 3, length: 0))
        scrollView.needsLayout = true
        scrollView.layoutSubtreeIfNeeded()
        assertHeights(
            scrollView: scrollView,
            textView: textView,
            viewport: lineHeight * 5,
            document: lineHeight * 5
        )
        XCTAssertFalse(scrollView.hasVerticalScroller)
        XCTAssertEqual(scrollView.contentView.bounds.origin, .zero)

        scrollView.frame.size.height = lineHeight
        scrollView.layoutSubtreeIfNeeded()
        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight, document: lineHeight)
    }

    func testMacComposerProgrammaticRestoreRevealsFinalSelectionOnce() {
        let (scrollView, textView) = makeComposerViews(width: 240, height: 16)
        let lineHeight = IrisAppKitComposerTextView.lineHeight(for: textView)
        let restoredText = "1\n2\n3\n4\n5\n6"
        scrollView.frame.size.height = lineHeight * 5
        textView.string = restoredText
        textView.setSelectedRange(NSRange(location: (restoredText as NSString).length, length: 0))

        scrollView.revealSelectionAfterNextLayout(in: textView)
        runMainLoop()

        XCTAssertEqual(textView.selectedRange().location, (restoredText as NSString).length)
        XCTAssertEqual(scrollView.contentView.bounds.origin.y, lineHeight, accuracy: 0.5)
        scrollView.needsLayout = true
        scrollView.layoutSubtreeIfNeeded()
        XCTAssertEqual(scrollView.contentView.bounds.origin.y, lineHeight, accuracy: 0.5)
    }

    func testMacComposerSoftWrapAndTrailingNewlineGrowToTwoLinesWithZeroOrigin() {
        let cases = [
            "This is a long line which wraps once at the composer width.",
            "first\n"
        ]

        for text in cases {
            let (scrollView, textView) = makeComposerViews(width: 240, height: 16)
            let lineHeight = IrisAppKitComposerTextView.lineHeight(for: textView)
            scrollView.frame.size.height = lineHeight
            textView.string = text
            scrollView.needsLayout = true
            scrollView.layoutSubtreeIfNeeded()
            assertHeights(
                scrollView: scrollView,
                textView: textView,
                viewport: lineHeight,
                document: lineHeight
            )

            let fittingSize = IrisAppKitComposerTextView.fittingSize(
                for: textView,
                proposedWidth: 240,
                actualWidth: 240
            )
            guard let fittingSize else {
                XCTFail("Missing fitting size for text: \(text)")
                continue
            }
            XCTAssertEqual(fittingSize.height, lineHeight * 2, accuracy: 0.5, "Failed for text: \(text)")

            scrollView.frame.size.height = lineHeight * 2
            scrollView.layoutSubtreeIfNeeded()
            assertHeights(
                scrollView: scrollView,
                textView: textView,
                viewport: lineHeight * 2,
                document: lineHeight * 2
            )
            XCTAssertEqual(scrollView.contentView.bounds.origin, .zero)
            XCTAssertFalse(scrollView.hasVerticalScroller)
        }
    }

    func testMacComposerHostingLayoutGrowsOverflowsShrinksAndReflows() {
        let host = NSHostingView(rootView: IrisComposerHostingHarness())
        host.frame = NSRect(x: 0, y: 0, width: 240, height: 160)
        host.layoutSubtreeIfNeeded()

        guard let scrollView = firstSubview(of: IrisComposerScrollView.self, in: host),
              let textView = scrollView.documentView as? IrisComposerNSTextView else {
            return XCTFail("Composer AppKit views were not installed in the hosting view")
        }
        let lineHeight = IrisAppKitComposerTextView.lineHeight(for: textView)

        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight, document: lineHeight)

        publishNativeText("first\nsecond", in: textView)
        host.layoutSubtreeIfNeeded()
        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight, document: lineHeight)

        runMainLoop()
        host.layoutSubtreeIfNeeded()
        assertHeights(
            scrollView: scrollView,
            textView: textView,
            viewport: lineHeight * 2,
            document: lineHeight * 2
        )

        publishNativeText("1\n2\n3\n4\n5\n6", in: textView)
        runMainLoop()
        host.layoutSubtreeIfNeeded()
        assertHeights(
            scrollView: scrollView,
            textView: textView,
            viewport: lineHeight * 5,
            document: lineHeight * 6
        )
        XCTAssertTrue(scrollView.hasVerticalScroller)
        XCTAssertEqual(scrollView.contentView.bounds.origin.y, lineHeight, accuracy: 0.5)

        publishNativeText("one", in: textView)
        runMainLoop()
        host.layoutSubtreeIfNeeded()
        assertHeights(scrollView: scrollView, textView: textView, viewport: lineHeight, document: lineHeight)
        XCTAssertFalse(scrollView.hasVerticalScroller)
        XCTAssertEqual(scrollView.contentView.bounds.origin, .zero)

        let wrappingText = "This is a long composer line that wraps several times when the available width becomes narrow."
        host.frame.size.width = 80
        publishNativeText(wrappingText, in: textView)
        runMainLoop()
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(scrollView.contentView.bounds.height, lineHeight * 5, accuracy: 0.5)
        XCTAssertGreaterThan(textView.frame.height, lineHeight * 5)
        XCTAssertTrue(scrollView.hasVerticalScroller)

        host.frame.size.width = 360
        host.layoutSubtreeIfNeeded()
        XCTAssertLessThan(scrollView.contentView.bounds.height, lineHeight * 5)
        XCTAssertEqual(textView.frame.height, scrollView.contentView.bounds.height, accuracy: 0.5)
        XCTAssertFalse(scrollView.hasVerticalScroller)
        XCTAssertEqual(scrollView.contentView.bounds.origin, .zero)
    }

    private func makeComposerViews(width: CGFloat, height: CGFloat) -> (IrisComposerScrollView, IrisComposerNSTextView) {
        let scrollView = IrisComposerScrollView(frame: NSRect(x: 0, y: 0, width: width, height: height))
        scrollView.scrollerStyle = .overlay

        let textView = IrisComposerNSTextView(frame: NSRect(x: 0, y: 0, width: width, height: height))
        textView.font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.heightTracksTextView = false
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = false
        textView.autoresizingMask = [.width, .height]
        scrollView.documentView = textView
        return (scrollView, textView)
    }

    private func assertHeights(
        scrollView: NSScrollView,
        textView: NSTextView,
        viewport: CGFloat,
        document: CGFloat,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(scrollView.contentView.bounds.height, viewport, accuracy: 0.5, file: file, line: line)
        XCTAssertEqual(textView.frame.height, document, accuracy: 0.5, file: file, line: line)
    }

    private func publishNativeText(_ text: String, in textView: NSTextView) {
        textView.string = text
        textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
        NotificationCenter.default.post(name: NSText.didChangeNotification, object: textView)
    }

    private func runMainLoop() {
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))
    }

    private func firstSubview<ViewType: NSView>(of type: ViewType.Type, in root: NSView) -> ViewType? {
        if let match = root as? ViewType {
            return match
        }
        for subview in root.subviews {
            if let match = firstSubview(of: type, in: subview) {
                return match
            }
        }
        return nil
    }
}

private struct IrisComposerHostingHarness: View {
    @State private var text = "first"
    @FocusState private var isFocused: Bool

    var body: some View {
        IrisAppKitComposerTextView(
            text: $text,
            isFocused: $isFocused,
            onSubmit: { _ in .rejected }
        )
        .frame(maxWidth: .infinity)
    }
}
#endif
