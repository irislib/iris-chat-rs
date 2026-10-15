import XCTest
#if os(macOS)
import AppKit
import SwiftUI
@testable import IrisChatMac
#else
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
        draft.textDidChange(from: visible, to: "Hello " + visible)
        XCTAssertEqual(draft.wireText(from: "Hello " + visible), "Hello " + wire)
    }

    func testEditingMentionNeverRetargetsByName() {
        var draft = IrisMentionDraft()
        draft.insert(label: "@Alex", token: people[0].token, owner: "alice", at: 0)
        draft.textDidChange(from: "@Alex", to: "@Alexandra")
        XCTAssertEqual(draft.wireText(from: "@Alexandra"), "@Alexandra")
        draft.textDidChange(from: "@Alexandra", to: "@Alex")
        XCTAssertEqual(draft.wireText(from: "@Alex"), "@Alex")
    }

    #if os(macOS)
    @MainActor
    func testNativeAtMenuSelectionKeepsDraftIdentityAndDoesNotSend() {
        let model = IrisComposerMentionCompletion()
        var draft = ""
        var identityDraft = IrisMentionDraft()
        var sends = 0
        let parent = IrisAppKitComposerTextView(
            text: Binding(get: { draft }, set: { identityDraft.textDidChange(from: draft, to: $0); draft = $0 }), isFocused: .constant(true),
            onSubmit: { _ in sends += 1; return .acceptedAndClear },
            onCompletionUpdate: { model.update($0, candidates: self.people) },
            onCompletionSelectionChange: model.selectionDidChange,
            onCompletionCommand: model.handle
        )
        model.onInsert = { person, label, location in identityDraft.insert(label: label, token: person.token, owner: person.id, at: location) }
        let coordinator = parent.makeCoordinator()
        let editor = IrisComposerNSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 80))
        editor.allowsUndo = true; editor.delegate = coordinator; editor.composerCommandDelegate = coordinator
        editor.onCompletionCommand = model.handle
        editor.insertText("@", replacementRange: NSRange(location: 0, length: 0))
        XCTAssertEqual(model.results.count, 3)
        XCTAssertTrue(model.handle(.accept))
        XCTAssertEqual(editor.string, "@Alex ")
        XCTAssertEqual(identityDraft.wireText(from: draft), people[0].token + " ")
        XCTAssertEqual(sends, 0)
    }
    #endif
}
