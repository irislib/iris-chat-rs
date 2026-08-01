import XCTest

#if os(iOS)
@testable import IrisChat
#elseif os(macOS)
@testable import IrisChatMac
#endif

final class IrisMessageActionHoverTests: XCTestCase {
    func testOnlyOneMessageOwnsTheActionDock() {
        var activeId = irisNextActiveMessageActionDockId(
            current: nil,
            messageId: "first",
            isActive: true
        )
        XCTAssertEqual(activeId, "first")

        activeId = irisNextActiveMessageActionDockId(
            current: activeId,
            messageId: "second",
            isActive: true
        )
        XCTAssertEqual(activeId, "second")

        activeId = irisNextActiveMessageActionDockId(
            current: activeId,
            messageId: "first",
            isActive: false
        )
        XCTAssertEqual(activeId, "second")

        activeId = irisNextActiveMessageActionDockId(
            current: activeId,
            messageId: "second",
            isActive: false
        )
        XCTAssertNil(activeId)
    }
}
