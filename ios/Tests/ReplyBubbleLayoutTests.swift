#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

final class ReplyBubbleLayoutTests: XCTestCase {
    @MainActor
    func testQuotesFillReplyWidthWithoutExpandingShortBubbles() {
        for width in [CGFloat(280), CGFloat(480)] {
            for body in ["Yes", String(repeating: "A longer reply with several words. ", count: 8)] {
                var measured: [String: CGFloat] = [:]
                let view = ReplyBubbleLayout(isOutgoing: true) {
                    ReplyPreviewView(reply: ReplyPreview(author: "Friend", body: "A short quote"), isOutgoing: true, onTap: {})
                        .background(widthReader("quote"))
                    TruncatableMessageBody(attributed: AttributedString(body), isOutgoing: true, bodyFont: .body)
                }
                .background(widthReader("bubble"))
                .onPreferenceChange(ReplyWidthPreference.self) { measured = $0 }
                .frame(width: width, height: 400, alignment: .topTrailing)
                let host = NSHostingView(rootView: view)
                host.frame = NSRect(x: 0, y: 0, width: width, height: 400)
                host.layoutSubtreeIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
                host.layoutSubtreeIfNeeded()
                XCTAssertNotNil(measured["quote"])
                XCTAssertEqual(measured["quote"] ?? -1, measured["bubble"] ?? -2, accuracy: 0.5)
                if body == "Yes" {
                    XCTAssertLessThan(measured["bubble"] ?? width, width - 20)
                }
            }
        }
    }

    @MainActor
    func testRenderProductionReplyRows() throws {
        let view = VStack(spacing: 20) {
            fixtureRow(body: "Bitcoin conferences are the right place to fix that. There's also an afterparty and more details at https://example.com/events/afterparty", outgoing: true)
            fixtureRow(body: "Yes", outgoing: true)
            fixtureRow(body: "A longer incoming reply should use the same full-width quote layout while keeping the text readable.", outgoing: false)
        }
        .padding(24)
        .frame(width: 900)
        .background(Color.black)
        .environment(\.colorScheme, .dark)
        .environment(\.irisPalette, .dark)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        let rendered = try XCTUnwrap(renderer.cgImage)
        XCTAssertEqual(rendered.width, 1800)
        guard let directory = ProcessInfo.processInfo.environment["IRIS_REPLY_LAYOUT_ARTIFACT_DIR"] else { return }
        let bitmap = NSBitmapImageRep(cgImage: rendered)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("reply-bubbles.png"))
    }

    @MainActor
    private func fixtureRow(body: String, outgoing: Bool) -> some View {
        var message = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 1).currentChat!.messages[0]
        message.body = "↩ Friend: Anyway, I don't know anybody there yet :)\n\n\(body)"
        message.kind = .user
        message.isOutgoing = outgoing
        return ChatMessageRow(message: message, chatKind: .direct, showDayChip: false,
            hidesInlineDayChip: true, isFirstInCluster: true, isLastInCluster: true,
            showsGroupSenderName: false, showsGroupSenderAvatar: false, reactions: [],
            swipeOffset: 0, isActionDockActive: false, onActionDockActiveChange: { _ in },
            onReply: {}, onForward: {}, onForwardAttachment: { _ in }, onReact: { _ in },
            onInfo: {}, onDelete: {}, onScrollToQuote: { _ in }, onShowReactors: {},
            downloadAttachment: { _ in nil }, openAttachment: { _ in }, onOpenImage: { _, _ in })
    }

    private func widthReader(_ name: String) -> some View {
        GeometryReader { geometry in
            Color.clear.preference(key: ReplyWidthPreference.self, value: [name: geometry.size.width])
        }
    }
}

private struct ReplyWidthPreference: PreferenceKey {
    static var defaultValue: [String: CGFloat] = [:]
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) {
        value.merge(nextValue(), uniquingKeysWith: { _, next in next })
    }
}
#endif
