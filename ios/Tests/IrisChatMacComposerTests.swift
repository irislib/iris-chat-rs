import XCTest

#if os(macOS)
import AppKit
@testable import IrisChatMac

final class IrisChatMacComposerTests: XCTestCase {
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
