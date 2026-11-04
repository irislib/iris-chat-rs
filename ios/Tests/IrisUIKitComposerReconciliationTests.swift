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
            text: Binding(get: { self.rendered }, set: { self.state.text = $0 }),
            isFocused: .constant(true)
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
}
#endif
