import SwiftUI
import XCTest

#if os(iOS)
@testable import IrisChat
#elseif os(macOS)
@testable import IrisChatMac
#endif

final class IrisComposerTypingTests: XCTestCase {
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
