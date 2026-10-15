import XCTest
#if os(macOS)
import AppKit
import SwiftUI
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class IrisComposerEmojiCompletionTests: XCTestCase {
    private let entries = [
        IrisEmojiCompletionEntry(emoji: "😄", shortcode: "smile"),
        IrisEmojiCompletionEntry(emoji: "😃", shortcode: "smiley"),
        IrisEmojiCompletionEntry(emoji: "🔥", shortcode: "fire"),
        IrisEmojiCompletionEntry(emoji: "👍", shortcode: "thumbsup", aliases: ["+1", "thumbs_up"]),
    ]

    func testOnlyWhitespaceDelimitedTokensComplete() {
        for text in ["https://iris.to/:smile", "12:30", "name:smile", "::smile", "user@example.org:smile", "hello :sm ile"] {
            XCTAssertNil(IrisComposerCompletionToken.find(in: text, selection: NSRange(location: text.utf16.count, length: 0)), text)
        }
        for text in [":sm", "hello :sm", "hello\n:sm", "hello\t:sm"] {
            XCTAssertEqual(IrisComposerCompletionToken.find(in: text, selection: NSRange(location: text.utf16.count, length: 0))?.query, "sm")
        }
    }

    func testUTF16ReplacementPreservesEmojiPrefixAndSuffix() {
        let text = "👩🏽‍💻 :smile: after"
        let selection = NSRange(location: ("👩🏽‍💻 :sm" as NSString).length, length: 0)
        let token = IrisComposerCompletionToken.find(in: text, selection: selection)
        XCTAssertEqual(token?.query, "sm")
        XCTAssertEqual(token.map { (text as NSString).replacingCharacters(in: $0.range, with: "😄") }, "👩🏽‍💻 😄 after")
        XCTAssertNil(IrisComposerCompletionToken.find(in: text, selection: NSRange(location: 1, length: 0)))
        XCTAssertNil(IrisComposerCompletionToken.find(in: text, selection: NSRange(location: 9, length: 2)))
        XCTAssertNil(IrisComposerCompletionToken.find(in: text, selection: NSRange(location: 999, length: 0)))
    }

    func testSearchRequiresTwoCharactersAndRanksExactShortcodesFirst() {
        XCTAssertTrue(IrisEmojiCompletionEntry.search("s", in: entries).isEmpty)
        XCTAssertEqual(IrisEmojiCompletionEntry.search("sm", in: entries).map(\.emoji), ["😄", "😃"])
        XCTAssertEqual(IrisEmojiCompletionEntry.search("+1", in: entries).first?.emoji, "👍")
        XCTAssertEqual(IrisEmojiCompletionEntry.search("thumbs-up", in: entries).first?.emoji, "👍")
        XCTAssertTrue(IrisEmojiCompletionEntry.search("not-an-emoji", in: entries).isEmpty)
    }

    @MainActor
    func testKeyboardSelectionWrapsAndAcceptReplacesOnlyToken() {
        let model = IrisComposerEmojiCompletion(entries: entries)
        var replacement: (NSRange, String)?
        model.update(IrisComposerCompletionContext(text: "hi :sm", selection: NSRange(location: 6, length: 0), hasMarkedText: false) {
            replacement = ($0, $1); return true
        })
        XCTAssertTrue(model.handle(.previous))
        XCTAssertEqual(model.selectedIndex, 1)
        XCTAssertTrue(model.handle(.accept))
        XCTAssertEqual(replacement?.0, NSRange(location: 3, length: 3))
        XCTAssertEqual(replacement?.1, "😃 ")
        XCTAssertTrue(model.results.isEmpty)
        XCTAssertFalse(model.handle(.accept))
    }

    @MainActor
    func testClosedExactCodeConvertsButUnknownCodeAndMarkedTextDoNot() {
        let model = IrisComposerEmojiCompletion(entries: entries)
        var replacements: [String] = []
        for (text, marked) in [(":fire:", false), (":fir:", false), (":smile:", true)] {
            model.update(IrisComposerCompletionContext(text: text, selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: marked) {
                replacements.append($1); return true
            })
        }
        XCTAssertEqual(replacements, ["🔥"])
    }

    @MainActor
    func testEscapeAndCaretMovementDismissUntilAnotherEdit() {
        let model = IrisComposerEmojiCompletion(entries: entries)
        func update(_ text: String) {
            model.update(IrisComposerCompletionContext(text: text, selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: false) { _, _ in false })
        }
        update(":sm")
        model.selectionDidChange(text: ":sm", selection: NSRange(location: 3, length: 0))
        XCTAssertFalse(model.results.isEmpty, "Native selection echo after insertion must not close the menu")
        XCTAssertTrue(model.handle(.dismiss))
        update(":sm")
        XCTAssertTrue(model.results.isEmpty)
        update(":smi")
        XCTAssertFalse(model.results.isEmpty)
        model.selectionDidChange(text: ":smi", selection: NSRange(location: 2, length: 0))
        XCTAssertTrue(model.results.isEmpty)
    }

    @MainActor
    func testUndoDoesNotReapplyAClosedShortcode() {
        let model = IrisComposerEmojiCompletion(entries: entries)
        var replacements = 0
        model.update(IrisComposerCompletionContext(text: ":fire:", selection: NSRange(location: 6, length: 0), hasMarkedText: false, isUndoOrRedo: true) { _, _ in
            replacements += 1; return true
        })
        XCTAssertEqual(replacements, 0)
        XCTAssertTrue(model.results.isEmpty)
    }

    #if os(macOS)
    @MainActor
    func testNativeReplacementRejectsAStaleSelectionOrDraft() {
        var draft = ":sm"
        var context: IrisComposerCompletionContext?
        let parent = IrisAppKitComposerTextView(
            text: Binding(get: { draft }, set: { draft = $0 }), isFocused: .constant(true),
            onSubmit: { _ in .rejected }, onCompletionUpdate: { context = $0 }
        )
        let coordinator = parent.makeCoordinator()
        let editor = NSTextView()
        editor.string = draft
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        coordinator.textDidChange(Notification(name: NSText.didChangeNotification, object: editor))
        editor.setSelectedRange(NSRange(location: 0, length: 0))
        XCTAssertFalse(context?.replace(NSRange(location: 0, length: 3), "😄") ?? true)
        editor.string = "new draft"
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        XCTAssertFalse(context?.replace(NSRange(location: 0, length: 3), "😄") ?? true)
        XCTAssertEqual(editor.string, "new draft")
    }

    @MainActor
    func testNativeCompletionUsesUndoAndDoesNotSubmit() {
        let model = IrisComposerEmojiCompletion(entries: entries)
        var draft = ""
        var sends = 0
        let parent = IrisAppKitComposerTextView(
            text: Binding(get: { draft }, set: { draft = $0 }), isFocused: .constant(true),
            onSubmit: { _ in sends += 1; return .acceptedAndClear },
            onCompletionUpdate: model.update,
            onCompletionSelectionChange: model.selectionDidChange,
            onCompletionCommand: model.handle
        )
        let coordinator = parent.makeCoordinator()
        let editor = IrisComposerNSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 80))
        editor.allowsUndo = true
        editor.delegate = coordinator
        editor.composerCommandDelegate = coordinator
        editor.onCompletionCommand = model.handle
        let window = NSWindow(contentRect: editor.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = editor
        window.makeFirstResponder(editor)
        defer { window.orderOut(nil) }
        editor.insertText("hello :sm", replacementRange: NSRange(location: 0, length: 0))
        XCTAssertFalse(model.results.isEmpty)
        // The test inserts setup and selection in one run-loop turn. Discard
        // setup's undo entry so only the completion action is under test.
        editor.undoManager?.removeAllActions()
        editor.doCommand(by: #selector(NSResponder.moveDown(_:)))
        editor.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        XCTAssertEqual(editor.string, "hello 😃 ")
        XCTAssertEqual(draft, editor.string)
        XCTAssertEqual(sends, 0)
        XCTAssertTrue(editor.undoManager?.canUndo == true)
        editor.undoManager?.undo()
        XCTAssertEqual(editor.string, "hello :sm")
        XCTAssertTrue(model.results.isEmpty)
        editor.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        XCTAssertEqual(sends, 1)
    }
    #endif
}
