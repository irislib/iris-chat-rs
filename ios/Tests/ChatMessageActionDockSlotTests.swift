#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class ChatMessageActionDockSlotTests: XCTestCase {
    private let size = CGSize(width: 136, height: 38)

    func testHiddenDockDoesNotConstructControlsOrMoveTheBubble() {
        var constructions = 0
        let slot = ChatMessageActionDockSlot(isVisible: false, width: size.width) {
            constructions += 1
            return Text("Actions").frame(width: 136, height: 38)
        }
        let host = NSHostingView(rootView: slot)

        XCTAssertEqual(host.fittingSize, CGSize(width: size.width, height: 0))
        XCTAssertEqual(constructions, 0, "Invisible rows must not build their hover controls")
    }

    func testHoverBuildsControlsWithoutChangingTheSlotSizeAndStopsWhenHidden() {
        var constructions = 0
        func slot(_ visible: Bool) -> some View {
            ChatMessageActionDockSlot(isVisible: visible, width: size.width) {
                constructions += 1
                return Button("Reply") {}.frame(width: 136, height: 38)
            }
        }
        let host = NSHostingView(rootView: slot(false))
        XCTAssertEqual(host.fittingSize, CGSize(width: size.width, height: 0))
        XCTAssertEqual(constructions, 0)

        host.rootView = slot(true)
        XCTAssertEqual(host.fittingSize, CGSize(width: size.width, height: 0))
        XCTAssertGreaterThan(constructions, 0)

        let beforeHiding = constructions
        host.rootView = slot(false)
        XCTAssertEqual(host.fittingSize, CGSize(width: size.width, height: 0))
        XCTAssertEqual(constructions, beforeHiding)
    }

    func testShortMessageRowsKeepCompactSpacingWithOrWithoutHoverControls() {
        let size = self.size
        for visible in [false, true] {
            for bubbleHeight: CGFloat in [28, 46, 90] {
                let timeline = VStack(spacing: 2) {
                    ForEach(0..<3) { _ in
                        HStack(alignment: .bottom, spacing: 8) {
                            ChatMessageActionDockSlot(isVisible: visible, width: size.width) {
                                Button("Reply") {}.frame(width: size.width, height: size.height)
                            }
                            Color.purple.frame(width: 48, height: bubbleHeight)
                        }
                    }
                }
                let host = NSHostingView(rootView: timeline)
                XCTAssertEqual(host.fittingSize.height, 3 * bubbleHeight + 4,
                               "Hover actions must never spread short messages apart")
            }
        }
    }

    func testBubbleWidthLimitKeepsShortMessagesIntrinsic() {
        let bubble = Text("Hi")
            .padding(12)
            .modifier(ChatMessageBubbleWidthLimit(maxWidth: 480))
        let host = NSHostingView(rootView: bubble)
        XCTAssertLessThan(host.fittingSize.width, 80)
        XCTAssertGreaterThan(host.fittingSize.width, 24)
    }

    func testBubbleWidthLimitWrapsLongTextAndAdaptsToNarrowRows() {
        let text = String(repeating: "A longer message that should wrap. ", count: 12)
        func bubble(_ width: CGFloat) -> some View {
            Text(text)
                .fixedSize(horizontal: false, vertical: true)
                .padding(12)
                .modifier(ChatMessageBubbleWidthLimit(maxWidth: width))
        }
        let wide = NSHostingView(rootView: bubble(480)).fittingSize
        let narrow = NSHostingView(rootView: bubble(240)).fittingSize
        XCTAssertLessThanOrEqual(wide.width, 480)
        XCTAssertLessThanOrEqual(narrow.width, 240)
        XCTAssertGreaterThan(narrow.height, wide.height)
    }
}
#endif
