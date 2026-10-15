import XCTest
import SwiftUI
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
import UIKit
@testable import IrisChat
#endif

final class IrisComposerMentionTests: XCTestCase {
    private let people = [
        IrisMentionCandidate(id: "alice", name: "Alex", token: "[@Alex](nostr:alice)"),
        IrisMentionCandidate(id: "bob", name: "Alex", token: "[@Alex](nostr:bob)"),
        IrisMentionCandidate(id: "carol", name: "Carol Smith", token: "[@Carol Smith](nostr:carol)")
    ]

    func testBareAtOpensAndNamesCanIncludeSpaces() {
        XCTAssertEqual(IrisMentionQuery.find(in: "@", selection: NSRange(location: 1, length: 0))?.query, "")
        XCTAssertEqual(IrisMentionQuery.find(in: "👋 @Carol S", selection: NSRange(location: 11, length: 0))?.query, "Carol S")
        for text in ["a@b", "@@", "https://example/@", "@" + String(repeating: "x", count: 65)] {
            XCTAssertNil(IrisMentionQuery.find(in: text, selection: NSRange(location: text.utf16.count, length: 0)))
        }
    }

    @MainActor
    func testSelectionUsesIdentityWithDuplicateNames() {
        let model = IrisComposerMentionCompletion()
        var text = "@"
        var chosen: IrisMentionCandidate?
        model.onInsert = { person, _, _ in chosen = person }
        model.update(IrisComposerCompletionContext(text: text, selection: NSRange(location: 1, length: 0), hasMarkedText: false) {
            text = (text as NSString).replacingCharacters(in: $0, with: $1); return true
        }, candidates: people)
        XCTAssertEqual(model.results.count, 3)
        XCTAssertTrue(model.handle(.next))
        XCTAssertTrue(model.handle(.accept))
        XCTAssertEqual(chosen?.id, "bob")
        XCTAssertEqual(text, "@Alex ")
        XCTAssertFalse(model.handle(.accept))
    }

    @MainActor
    func testDismissMarkedTextAndStaleEditorDoNotInsert() {
        let model = IrisComposerMentionCompletion()
        var inserted = false
        model.onInsert = { _, _, _ in inserted = true }
        func update(_ text: String, marked: Bool = false) {
            model.update(IrisComposerCompletionContext(text: text, selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: marked) { _, _ in false }, candidates: people)
        }
        update("@"); XCTAssertTrue(model.handle(.dismiss))
        update("@"); XCTAssertTrue(model.results.isEmpty)
        update("@Al"); XCTAssertEqual(model.results.count, 2)
        model.insert(people[0]); XCTAssertFalse(inserted)
        update("@Ale", marked: true); XCTAssertTrue(model.results.isEmpty)
    }

    func testDraftPreservesIdentityAcrossRestorePrefixEditsAndRename() {
        let person = people[0]
        var draft = IrisMentionDraft()
        let wire = "👋 " + person.token + "!"
        let span = IrisMentionDraft.Span(range: NSRange(location: 3, length: person.token.utf16.count), label: "@Renamed", token: person.token, owner: person.id)
        let visible = draft.restore(wireText: wire, encodedSpans: [span])
        XCTAssertEqual(visible, "👋 @Renamed!")
        draft.replace(range: NSRange(location: 0, length: 0), replacementLength: 6)
        XCTAssertEqual(draft.wireText(from: "Hello " + visible), "Hello " + wire)
    }

    func testEditingMentionNeverRetargetsByName() {
        var draft = IrisMentionDraft()
        draft.insert(label: "@Alex", token: people[0].token, owner: "alice", at: 0)
        draft.replace(range: NSRange(location: 5, length: 0), replacementLength: 5, insertingWord: true)
        XCTAssertEqual(draft.wireText(from: "@Alexandra"), "@Alexandra")
        draft.replace(range: NSRange(location: 5, length: 5), replacementLength: 0)
        XCTAssertEqual(draft.wireText(from: "@Alex"), "@Alex")
    }

    #if os(iOS)
    @MainActor
    func testUIKitNativeMentionEditsPreserveExactIdentityThroughUndo() {
        let state = IrisComposerState()
        let model = IrisComposerMentionCompletion()
        let parent = IrisUIKitComposerTextView(
            text: Binding(get: { state.text }, set: { state.text = $0 }), isFocused: .constant(true),
            onNativeEdit: state.prepareNativeEdit,
            onCompletionUpdate: { model.update($0, candidates: self.people) },
            onCompletionSelectionChange: model.selectionDidChange,
            onCompletionCommand: model.handle
        )
        model.onInsert = { person, label, location in
            state.insertMention(label: label, token: person.token, owner: person.id, at: location)
        }
        let coordinator = parent.makeCoordinator()
        let storage = NSTextStorage()
        let layout = NSLayoutManager()
        let container = NSTextContainer()
        storage.addLayoutManager(layout); layout.addTextContainer(container)
        let editor = IrisComposerUITextView(frame: CGRect(x: 0, y: 0, width: 400, height: 100), textContainer: container)
        editor.delegate = coordinator; storage.delegate = coordinator; coordinator.nativeTextView = editor
        let controller = UIViewController(); controller.view.addSubview(editor)
        let window = UIWindow(frame: editor.bounds); window.rootViewController = controller
        window.makeKeyAndVisible(); editor.becomeFirstResponder()
        defer { editor.resignFirstResponder(); window.isHidden = true }
        guard let undo = editor.undoManager else { return XCTFail("Native undo manager is required") }
        undo.groupsByEvent = false
        func change(_ action: () -> Void) {
            undo.beginUndoGrouping(); action(); coordinator.textViewDidChange(editor); undo.endUndoGrouping()
        }
        change { editor.insertText("@") }
        XCTAssertEqual(model.results.count, 3)
        change { XCTAssertTrue(model.handle(.accept)) }
        XCTAssertEqual(state.wireText, people[0].token + " ")
        undo.undo(); XCTAssertEqual(state.wireText, "@")
        undo.redo(); XCTAssertEqual(state.wireText, people[0].token + " ")
        change { editor.insertText("@") }
        change { XCTAssertTrue(model.handle(.next)); XCTAssertTrue(model.handle(.accept)) }
        let both = people[0].token + " " + people[1].token + " "
        XCTAssertEqual(state.wireText, both)
        change { editor.selectedRange = NSRange(location: 0, length: 6); editor.insertText("") }
        XCTAssertEqual(state.wireText, people[1].token + " ")
        undo.undo(); XCTAssertEqual(state.wireText, both)
        undo.redo(); XCTAssertEqual(state.wireText, people[1].token + " ")
        undo.undo()
        change { editor.selectedRange = NSRange(location: 6, length: 6); editor.insertText("") }
        XCTAssertEqual(state.wireText, people[0].token + " ")
        undo.undo(); XCTAssertEqual(state.wireText, both)
        change { editor.selectedRange = NSRange(location: 0, length: 5); editor.insertText("@Alex") }
        XCTAssertEqual(state.wireText, "@Alex " + people[1].token + " ")
        undo.undo(); XCTAssertEqual(state.wireText, both)
        undo.redo(); XCTAssertEqual(state.wireText, "@Alex " + people[1].token + " ")
        undo.undo()
        change { editor.selectedRange = NSRange(location: 1, length: 1); editor.insertText("Z") }
        XCTAssertEqual(state.wireText, "@Zlex " + people[1].token + " ")
        undo.undo(); XCTAssertEqual(state.wireText, both)
    }
    #endif

    #if os(macOS)
    @MainActor
    func testNativeAtMenuSelectionKeepsDraftIdentityAndDoesNotSend() {
        let model = IrisComposerMentionCompletion()
        let state = IrisComposerState()
        var sends = 0
        let parent = IrisAppKitComposerTextView(
            text: Binding(get: { state.text }, set: { state.text = $0 }), isFocused: .constant(true),
            onSubmit: { _ in sends += 1; return .acceptedAndClear },
            onNativeEdit: state.prepareNativeEdit,
            onCompletionUpdate: { model.update($0, candidates: self.people) },
            onCompletionSelectionChange: model.selectionDidChange,
            onCompletionCommand: model.handle
        )
        model.onInsert = { person, label, location in state.insertMention(label: label, token: person.token, owner: person.id, at: location) }
        let coordinator = parent.makeCoordinator()
        let editor = IrisComposerNSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 80))
        editor.allowsUndo = true; editor.delegate = coordinator; editor.composerCommandDelegate = coordinator
        editor.textStorage?.delegate = coordinator; coordinator.nativeTextView = editor
        editor.onCompletionCommand = model.handle
        let window = NSWindow(contentRect: editor.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = editor
        guard let undo = editor.undoManager else { return XCTFail("Native undo manager is required") }
        undo.groupsByEvent = false
        func change(_ action: () -> Void) { undo.beginUndoGrouping(); editor.breakUndoCoalescing(); action(); undo.endUndoGrouping() }
        change { editor.insertText("@", replacementRange: NSRange(location: 0, length: 0)) }
        XCTAssertEqual(model.results.count, 3)
        change { XCTAssertTrue(model.handle(.accept)) }
        XCTAssertEqual(editor.string, "@Alex ")
        XCTAssertEqual(state.wireText, people[0].token + " ")
        XCTAssertEqual(sends, 0)
        undo.undo()
        XCTAssertEqual(editor.string, "@")
        XCTAssertEqual(state.wireText, "@")
        undo.redo()
        XCTAssertEqual(state.wireText, people[0].token + " ")
        change { editor.insertText("@", replacementRange: editor.selectedRange()) }
        change { XCTAssertTrue(model.handle(.next)); XCTAssertTrue(model.handle(.accept)) }
        XCTAssertEqual(editor.string, "@Alex @Alex ")
        XCTAssertEqual(state.wireText, people[0].token + " " + people[1].token + " ")
        change { editor.insertText("", replacementRange: NSRange(location: 0, length: 6)) }
        XCTAssertEqual(state.wireText, people[1].token + " ", "Deleting the first label must retain the second identity")
        undo.undo()
        XCTAssertEqual(state.wireText, people[0].token + " " + people[1].token + " ")
        undo.redo()
        XCTAssertEqual(state.wireText, people[1].token + " ")
        undo.undo()
        change { editor.insertText("", replacementRange: NSRange(location: 6, length: 6)) }
        XCTAssertEqual(state.wireText, people[0].token + " ", "Deleting the second label must retain the first identity")
        undo.undo()
        change { editor.insertText("@Alex", replacementRange: NSRange(location: 0, length: 5)) }
        XCTAssertEqual(state.wireText, "@Alex " + people[1].token + " ", "Replacing a label with identical plain text removes identity")
        undo.undo()
        XCTAssertEqual(state.wireText, people[0].token + " " + people[1].token + " ")
        undo.redo()
        XCTAssertEqual(state.wireText, "@Alex " + people[1].token + " ")
        undo.undo()
        change { editor.insertText("Z", replacementRange: NSRange(location: 1, length: 1)) }
        XCTAssertEqual(state.wireText, "@Zlex " + people[1].token + " ")
        undo.undo()
        XCTAssertEqual(state.wireText, people[0].token + " " + people[1].token + " ", "Undo restores mention metadata after a label edit")
        undo.redo()
        XCTAssertEqual(state.wireText, "@Zlex " + people[1].token + " ")
        state.restore("New draft", replaceExisting: true)
        coordinator.lastPublishedNativeText = nil
        coordinator.reconcile(editor)
        XCTAssertEqual(editor.string, "New draft")
        XCTAssertFalse(undo.canUndo, "Restored drafts reset native and identity undo history together")
        window.close()
    }
    #endif
}
