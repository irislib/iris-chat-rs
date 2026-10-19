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
    func testLargeGroupCanSelectBeyondTheFirstSixResultsAndFilterAgain() {
        let model = IrisComposerMentionCompletion()
        let candidates = (0..<12).map {
            IrisMentionCandidate(id: "person-\($0)", name: "Person \($0)", token: "person-\($0)")
        }
        var chosen: IrisMentionCandidate?
        model.onInsert = { person, _, _ in chosen = person }
        func update(_ text: String) {
            model.update(IrisComposerCompletionContext(text: text,
                selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: false) { _, _ in true },
                candidates: candidates)
        }
        update("@")
        XCTAssertEqual(model.results.count, 12)
        for _ in 0..<11 { XCTAssertTrue(model.handle(.next)) }
        XCTAssertTrue(model.handle(.accept))
        XCTAssertEqual(chosen?.id, "person-11")
        update("@Person 2")
        XCTAssertEqual(model.results.map(\.id), ["person-2"])
        XCTAssertEqual(model.selectedIndex, 0)
    }

    @MainActor
    func testNarrowingAndBackspacingKeepsTheMatchingIdentity() {
        let model = IrisComposerMentionCompletion()
        let candidates = [
            IrisMentionCandidate(id: "andrew", name: "Andrew", token: "andrew"),
            IrisMentionCandidate(id: "ada", name: "Ada", token: "ada")
        ]
        for text in ["@", "@a", "@ad", "@ada", "@ad", "@ada"] {
            model.update(IrisComposerCompletionContext(text: text,
                selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: false) { _, _ in true },
                candidates: candidates)
            XCTAssertEqual(model.results.map(\.id), text.count <= 2 ? ["andrew", "ada"] : ["ada"])
            XCTAssertEqual(model.selectedCandidateID, text.count <= 2 ? "andrew" : "ada")
        }
    }

    @MainActor
    func testFilteredOutRowCannotInsertAnotherPerson() {
        let model = IrisComposerMentionCompletion()
        var inserted: String?
        model.onInsert = { person, _, _ in inserted = person.id }
        model.update(IrisComposerCompletionContext(text: "@Car", selection: NSRange(location: 4, length: 0),
            hasMarkedText: false) { _, _ in true }, candidates: people)
        model.insert(people[0])
        XCTAssertNil(inserted, "A stale row must never insert someone excluded by the current query")
        model.insert(people[2])
        XCTAssertEqual(inserted, "carol")
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
    func testRenderedRowsFollowIdentityWhileNarrowingAndBackspacing() throws {
        let model = IrisComposerMentionCompletion()
        let preferences = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0,
            messagesInCurrentChat: 0).preferences
        let pictureUrl = "https://example.invalid/mention-ada-avatar.png"
        let source = try XCTUnwrap(irisAvatarImageSource(pictureUrl: pictureUrl, preferences: preferences, pixelSize: 64))
        let avatar = NSImage(size: NSSize(width: 32, height: 32))
        avatar.lockFocus()
        NSColor.red.setFill()
        NSRect(x: 0, y: 0, width: 32, height: 32).fill()
        avatar.unlockFocus()
        IrisAvatarImageCache.store(avatar, for: source.cacheKey)
        let candidates = [
            IrisMentionCandidate(id: "andrew", name: "Andrew", token: "andrew"),
            IrisMentionCandidate(id: "ada", name: "Ada", token: "ada"),
            IrisMentionCandidate(id: "alice", name: "Alice", token: "alice")
        ]
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        state.currentChat?.participants = [ChatParticipantSnapshot(socialConnection: nil, ownerPubkeyHex: "ada",
            displayName: "Ada", pictureUrl: pictureUrl, isLocalOwner: false)]
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: MockRustApp(state: state), secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        func content(_ model: IrisComposerMentionCompletion) -> some View {
            IrisComposerMentionSuggestions(completion: model)
                .environment(\.irisAvatarContext, IrisAvatarContext(manager: manager))
        }
        let host = NSHostingView(rootView: content(model))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }

        func capture(_ view: NSView) throws -> NSBitmapImageRep {
            let image = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: image)
            return image
        }
        func difference(_ a: NSBitmapImageRep, _ b: NSBitmapImageRep) throws -> Double {
            XCTAssertEqual(a.pixelsWide, b.pixelsWide)
            XCTAssertEqual(a.pixelsHigh, b.pixelsHigh)
            guard a.pixelsWide == b.pixelsWide, a.pixelsHigh == b.pixelsHigh else { return 1 }
            var changed = 0
            for y in 0..<a.pixelsHigh { for x in 0..<a.pixelsWide {
                let p = try XCTUnwrap(a.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB))
                let q = try XCTUnwrap(b.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB))
                if max(abs(p.redComponent - q.redComponent), abs(p.greenComponent - q.greenComponent),
                       abs(p.blueComponent - q.blueComponent)) > 0.04 { changed += 1 }
            } }
            return Double(changed) / Double(a.pixelsWide * a.pixelsHigh)
        }

        for text in ["@", "@a", "@ad", "@ada", "@ad", "@a", "@ada"] {
            model.update(IrisComposerCompletionContext(text: text,
                selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: false) { _, _ in true },
                candidates: candidates)
            RunLoop.main.run(until: Date().addingTimeInterval(0.2))
            host.layoutSubtreeIfNeeded()
            if text.hasPrefix("@ad") { XCTAssertEqual(model.results.map(\.id), ["ada"]) }
            let reference = IrisComposerMentionCompletion()
            reference.update(IrisComposerCompletionContext(text: text,
                selection: NSRange(location: text.utf16.count, length: 0), hasMarkedText: false) { _, _ in true },
                candidates: candidates)
            let referenceHost = NSHostingView(rootView: content(reference))
            let referenceWindow = NSWindow(contentRect: window.contentRect(forFrameRect: window.frame),
                styleMask: [.titled], backing: .buffered, defer: false)
            referenceWindow.isReleasedWhenClosed = false
            referenceWindow.contentView = referenceHost
            referenceWindow.makeKeyAndOrderFront(nil)
            RunLoop.main.run(until: Date().addingTimeInterval(0.1))
            referenceHost.layoutSubtreeIfNeeded()
            let delta = try difference(capture(host), capture(referenceHost))
            referenceWindow.close()
            XCTAssertLessThan(delta, 0.005,
                "The filtered picker must match a fresh rendering of the current people for \(text)")
        }
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let redPixels = (0..<bitmap.pixelsHigh).reduce(0) { count, y in
            count + (0..<bitmap.pixelsWide).filter { x in
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { return false }
                return color.redComponent > 0.8 && color.greenComponent < 0.2 && color.blueComponent < 0.2
            }.count
        }
        XCTAssertGreaterThan(redPixels, 400, "A matching member's cached profile picture must be rendered")
        let image = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])),
            uniformTypeIdentifier: "public.png")
        image.name = "mention-ada-after-filtering"
        image.lifetime = .keepAlways
        add(image)
    }

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
