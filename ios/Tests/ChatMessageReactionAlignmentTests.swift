import SwiftUI
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
import UIKit
@testable import IrisChat
#endif

@MainActor
final class ChatMessageReactionAlignmentTests: XCTestCase {
    func testAvatarAndDockFollowTheBubbleBottomOnBothSidesWithAndWithoutReactions() throws {
        for outgoing in [false, true] {
            for reacted in [false, true] {
                var frames: [String: CGRect] = [:]
                let content = row(outgoing: outgoing, reacted: reacted)
                    .coordinateSpace(name: "reaction-alignment")
                    .onPreferenceChange(ReactionAlignmentFrames.self) { frames = $0 }
                    .padding(24)
                    .frame(width: 500, height: 140, alignment: .topLeading)
                #if os(macOS)
                let host = NSHostingView(rootView: content)
                host.frame = CGRect(x: 0, y: 0, width: 500, height: 140)
                host.layoutSubtreeIfNeeded()
                #else
                let host = UIHostingController(rootView: content)
                let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 500, height: 140))
                window.rootViewController = host
                window.makeKeyAndVisible()
                host.view.setNeedsLayout()
                host.view.layoutIfNeeded()
                defer { window.isHidden = true }
                #endif
                let deadline = Date().addingTimeInterval(1)
                while frames.count < (outgoing ? 3 : 4) && Date() < deadline {
                    RunLoop.main.run(until: Date().addingTimeInterval(0.01))
                }
                let bubble = try XCTUnwrap(frames["bubble"])
                let dock = try XCTUnwrap(frames["dock"])
                let extent = try XCTUnwrap(frames["extent"])
                XCTAssertEqual(dock.maxY, bubble.maxY, accuracy: 0.5,
                               "Hover actions follow the visible bubble, including outgoing rows")
                XCTAssertEqual(outgoing ? bubble.minX - dock.maxX : dock.minX - bubble.maxX,
                               SignalConversationLayout.messageStackSpacing, accuracy: 0.5)
                if !outgoing {
                    let avatar = try XCTUnwrap(frames["avatar"])
                    XCTAssertEqual(avatar.maxY, bubble.maxY, accuracy: 0.5)
                    XCTAssertEqual(bubble.minX - avatar.maxX,
                                   SignalConversationLayout.messageStackSpacing, accuracy: 0.5)
                }
                XCTAssertEqual(extent.maxY - bubble.maxY,
                               reacted ? SignalConversationLayout.reactionPillProtrusion : 0,
                               accuracy: 0.5, "Reaction space remains allocated below the bubble")
                let renderer = ImageRenderer(content: content)
                renderer.scale = 2
                #if os(macOS)
                let image = try XCTUnwrap(renderer.nsImage)
                #else
                let image = try XCTUnwrap(renderer.uiImage)
                #endif
                let attachment = XCTAttachment(image: image)
                attachment.name = "reaction-alignment-\(outgoing ? "outgoing" : "incoming")-\(reacted ? "reacted" : "plain")"
                attachment.lifetime = .keepAlways
                add(attachment)
            }
        }
    }

    private func row(outgoing: Bool, reacted: Bool) -> some View {
        HStack(alignment: .bottom, spacing: SignalConversationLayout.messageStackSpacing) {
            if outgoing { dock }
            if !outgoing {
                Circle().fill(Color.orange)
                    .frame(width: SignalConversationLayout.groupMessageAvatarSize,
                           height: SignalConversationLayout.groupMessageAvatarSize)
                    .background(measure("avatar"))
            }
            Text("Hi").padding(.horizontal, 12).padding(.vertical, 7)
                .background(Color.purple, in: RoundedRectangle(cornerRadius: 12))
                .background(measure("bubble"))
                .padding(.bottom, reacted ? SignalConversationLayout.reactionPillProtrusion : 0)
                .overlay(alignment: outgoing ? .bottomLeading : .bottomTrailing) {
                    if reacted {
                        Text("👍 1").font(.caption).frame(height: SignalConversationLayout.reactionPillHeight)
                            .padding(.horizontal, 6).background(Color.gray, in: Capsule())
                    }
                }
                .modifier(ChatMessageBubbleWidthLimit(maxWidth: 480))
                .modifier(ChatMessageBubbleBottomAlignment(
                    reactionProtrusion: reacted ? SignalConversationLayout.reactionPillProtrusion : 0))
            if !outgoing { dock }
        }
        .background(measure("extent"))
    }

    private var dock: some View {
        ChatMessageActionDockSlot(isVisible: true, width: 136) {
            HStack { Image(systemName: "arrowshape.turn.up.left"); Image(systemName: "face.smiling") }
                .frame(width: 136, height: 38)
                .background(Color.blue.opacity(0.2), in: Capsule())
                .background(self.measure("dock"))
        }
    }

    private func measure(_ id: String) -> some View {
        GeometryReader { geometry in
            Color.clear.preference(key: ReactionAlignmentFrames.self,
                                   value: [id: geometry.frame(in: .named("reaction-alignment"))])
        }
    }
}

private struct ReactionAlignmentFrames: PreferenceKey {
    static var defaultValue: [String: CGRect] = [:]
    static func reduce(value: inout [String: CGRect], nextValue: () -> [String: CGRect]) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}
