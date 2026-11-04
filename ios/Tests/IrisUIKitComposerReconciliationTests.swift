#if os(iOS)
import SwiftUI
import UIKit
import XCTest
@testable import IrisChat

@MainActor
final class IrisUIKitComposerReconciliationTests: XCTestCase {
    @MainActor
    final class Harness {
        var rendered = ""
        let state = IrisComposerState()
        let editor: IrisComposerUITextView = {
            let storage = NSTextStorage()
            let layout = NSLayoutManager()
            let container = NSTextContainer()
            storage.addLayoutManager(layout)
            layout.addTextContainer(container)
            return IrisComposerUITextView(frame: .zero, textContainer: container)
        }()
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        lazy var parent = IrisUIKitComposerTextView(
            text: Binding(get: { [unowned self] in rendered }, set: { [unowned self] in state.text = $0 }),
            isFocused: .constant(true),
            onNativeEdit: state.prepareNativeEdit
        )
        lazy var coordinator = parent.makeCoordinator()

        @MainActor
        func start() {
            let controller = UIViewController()
            editor.frame = CGRect(x: 0, y: 0, width: 360, height: 60)
            controller.view.addSubview(editor)
            window.rootViewController = controller
            window.makeKeyAndVisible()
            editor.delegate = coordinator
            editor.textStorage.delegate = coordinator
            coordinator.nativeTextView = editor
            XCTAssertTrue(editor.becomeFirstResponder())
            render("")
        }

        @MainActor
        func render(_ text: String) {
            rendered = text
            parent.draftRevision = state.draftRevision
            coordinator.parent = parent
            coordinator.reconcile(editor)
        }

        @MainActor
        func type(_ text: String) {
            editor.insertText(text)
            coordinator.textViewDidChange(editor)
        }
    }

    func testNativePrefixAndCaretSurviveDelayedEmptyAndIntermediateRenders() {
        let fixture = Harness()
        fixture.start()
        defer { fixture.window.isHidden = true }
        fixture.type("Ir")
        fixture.type("i")
        XCTAssertEqual(fixture.state.text, "Iri")
        fixture.render("")
        XCTAssertEqual(fixture.editor.text, "Iri")
        fixture.render("Ir")
        XCTAssertEqual(fixture.editor.selectedRange, NSRange(location: 3, length: 0))
        fixture.render("Iri")
        fixture.type("s quality check")
        XCTAssertEqual(fixture.editor.text, "Iris quality check")
        XCTAssertEqual(fixture.state.text, "Iris quality check")
    }

    func testStaleRenderPreservesSelectionInsideTheDraft() {
        let fixture = Harness()
        fixture.start()
        defer { fixture.window.isHidden = true }
        fixture.type("hello")
        fixture.editor.selectedRange = NSRange(location: 2, length: 0)
        fixture.type("X")
        fixture.render("hello")
        XCTAssertEqual(fixture.editor.text, "heXllo")
        XCTAssertEqual(fixture.editor.selectedRange, NSRange(location: 3, length: 0))
    }
    func testIntentionalClearAndDifferentChatReplaceAnUnacknowledgedEdit() {
        let fixture = Harness()
        fixture.start()
        defer { fixture.window.isHidden = true }
        fixture.state.restore("", forChat: "self")
        fixture.type("Iri")
        fixture.state.clearForSend { _ in }
        fixture.render("")
        XCTAssertEqual(fixture.editor.text, "")
        XCTAssertEqual(fixture.editor.selectedRange, NSRange(location: 0, length: 0))

        fixture.type("new local note")
        fixture.state.restore("restored elsewhere", forChat: "other")
        fixture.render(fixture.state.text)
        XCTAssertEqual(fixture.editor.text, "restored elsewhere")
        XCTAssertEqual(fixture.editor.selectedRange.location, "restored elsewhere".utf16.count)
    }

    func testDelayedInitialSelfChatLoadPreservesLocalTextAndCaret() {
        let fixture = Harness()
        fixture.start()
        defer { fixture.window.isHidden = true }
        fixture.type("Iri")
        fixture.state.restore("", forChat: "self")
        fixture.render(fixture.state.text)
        fixture.state.restore("stale saved text", forChat: "self")
        fixture.render(fixture.state.text)
        fixture.type("s")
        XCTAssertEqual(fixture.editor.text, "Iris")
        XCTAssertEqual(fixture.editor.selectedRange, NSRange(location: 4, length: 0))
    }

    func testFirstSnapshotBetweenNativeStorageChangeAndBindingCannotApplyAnOldDraft() {
        let state = IrisComposerState()
        let revision = state.draftRevision
        state.prepareNativeEdit(IrisComposerNativeEdit(text: "I", range: NSRange(location: 0, length: 0),
                                                       replacementLength: 1, undoManager: nil))
        state.restore("old draft", forChat: "self")
        XCTAssertEqual(state.text, "")
        XCTAssertEqual(state.draftRevision, revision)
        state.text = "I"
        state.restore("old draft", forChat: "self")
        XCTAssertEqual(state.text, "I")
    }

}
#endif
