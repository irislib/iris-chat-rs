import SwiftUI
import XCTest

#if os(iOS)
@testable import IrisChat
#elseif os(macOS)
@testable import IrisChatMac
#endif

final class IrisComposerTypingTests: XCTestCase {
    @MainActor
    func testDraftSaveDebouncesEditsAndDoesNotRepeatOnFlush() async {
        let composer = IrisComposerState()
        composer.restore("saved", replaceExisting: true)
        var saved: [String] = []
        let didSave = expectation(description: "latest draft saved")
        composer.text = "first edit"
        composer.scheduleSave { saved.append($0) }
        composer.text = "latest edit"
        composer.scheduleSave {
            saved.append($0)
            didSave.fulfill()
        }

        await fulfillment(of: [didSave], timeout: 2)
        composer.flush { saved.append($0) }

        XCTAssertEqual(saved, ["latest edit"])
    }

    @MainActor
    func testLeavingChatFlushesDraftAndCancelsDelayedSave() async {
        let composer = IrisComposerState()
        composer.restore("saved", replaceExisting: true)
        let delayedSave = expectation(description: "cancelled save")
        delayedSave.isInverted = true
        composer.text = "unsaved edit"
        composer.scheduleSave { _ in delayedSave.fulfill() }
        var saved: [String] = []

        composer.flush { saved.append($0) }
        await fulfillment(of: [delayedSave], timeout: 0.7)

        XCTAssertEqual(saved, ["unsaved edit"])
    }

    @MainActor
    func testRestoringAnotherChatCancelsPreviousDraftSave() async {
        let composer = IrisComposerState()
        composer.restore("first chat", replaceExisting: true)
        composer.text = "local edit"
        let delayedSave = expectation(description: "cancelled previous chat save")
        delayedSave.isInverted = true
        composer.scheduleSave { _ in delayedSave.fulfill() }
        composer.restore("older server draft", replaceExisting: false)
        XCTAssertEqual(composer.text, "local edit")

        composer.restore("second chat", replaceExisting: true)
        await fulfillment(of: [delayedSave], timeout: 0.7)

        XCTAssertEqual(composer.text, "second chat")
        composer.flush { _ in XCTFail("Restored draft should not be saved again") }
    }

    func testParentDraftRestoreDoesNotReportUserTyping() {
        var draft = ""
        var userEdits: [String] = []
        let parentBinding = Binding(
            get: { draft },
            set: { draft = $0 }
        )
        let editorBinding = irisComposerUserEditingBinding(parentBinding) { value in
            userEdits.append(value)
        }

        parentBinding.wrappedValue = "restored draft"

        XCTAssertEqual(draft, "restored draft")
        XCTAssertTrue(userEdits.isEmpty)

        editorBinding.wrappedValue = "restored draft!"

        XCTAssertEqual(draft, "restored draft!")
        XCTAssertEqual(userEdits, ["restored draft!"])
    }
}
