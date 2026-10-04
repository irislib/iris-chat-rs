#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class ChatMessageActionDockRealContentTests: XCTestCase {
    func testDockFollowsRealTextReplyAndFooterBubbles() throws {
        let fixtures = [
            Fixture(name: "short", body: "Hi"),
            Fixture(name: "link", body: "Read https://example.com before we meet."),
            Fixture(name: "truncated", body: String(repeating: "Warm blankets and a small map. ", count: 32)),
            Fixture(name: "reply", body: "Hi", hasReply: true),
            Fixture(name: "reply-footer", body: "Hi", hasReply: true, showsFooter: true),
            Fixture(name: "footer", body: "Hi", showsFooter: true)
        ]
        for fixture in fixtures {
            for kind in [ChatKind.direct, .group] {
                for outgoing in [false, true] {
                    for reacted in [false, true] {
                        try autoreleasepool {
                            _ = try qualify(fixture, kind: kind, outgoing: outgoing, reacted: reacted, width: 700)
                        }
                    }
                }
            }
        }
    }

    func testDockFollowsActualImageAndFileAttachmentBubbles() throws {
        let image = attachment(filename: "dock-fixture.png", isImage: true)
        let file = attachment(filename: "dock-fixture.txt", isImage: false)
        let fixtures = [
            Fixture(name: "image", body: "", attachments: [image]),
            Fixture(name: "image-caption", body: "A small picture", attachments: [image]),
            Fixture(name: "file", body: "", attachments: [file])
        ]
        for fixture in fixtures {
            for kind in [ChatKind.direct, .group] {
                for outgoing in [false, true] {
                    for reacted in [false, true] {
                        try autoreleasepool {
                            _ = try qualify(fixture, kind: kind, outgoing: outgoing, reacted: reacted, width: 700)
                        }
                    }
                }
            }
        }
    }

    func testRealParagraphReflowsInNarrowRowsWithoutSeparatingTheDock() throws {
        let fixture = Fixture(name: "wrapped", body: String(repeating: "Warm blankets and a small map for the woodland walk. ", count: 12))
        XCTAssertLessThan(fixture.body.count, 800, "Exercise full wrapping without the expansion toggle")
        for kind in [ChatKind.direct, .group] {
            for outgoing in [false, true] {
                for reacted in [false, true] {
                    let wide = try qualify(fixture, kind: kind, outgoing: outgoing, reacted: reacted, width: 700)
                    let narrow = try qualify(fixture, kind: kind, outgoing: outgoing, reacted: reacted, width: 420)
                    XCTAssertLessThan(narrow.width, wide.width)
                    XCTAssertGreaterThan(narrow.height, wide.height)
                }
            }
        }
    }

    private struct Fixture {
        let name: String
        let body: String
        var hasReply = false
        var showsFooter = false
        var attachments: [MessageAttachmentSnapshot] = []
    }

    private func attachment(filename: String, isImage: Bool) -> MessageAttachmentSnapshot {
        MessageAttachmentSnapshot(nhash: "dock-fixture", filename: filename,
            filenameEncoded: filename, htreeUrl: "htree://dock-fixture/\(filename)",
            isImage: isImage, isVideo: false, isAudio: false)
    }

    private func message(_ fixture: Fixture, outgoing: Bool, reacted: Bool) -> ChatMessageSnapshot {
        var item = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0,
                                         messagesInCurrentChat: 1).currentChat!.messages[0]
        item.id = "dock-\(fixture.name)"
        item.kind = .user
        item.author = "Lee"
        item.authorOwnerPubkeyHex = String(repeating: "1", count: 64)
        item.authorPictureUrl = nil
        item.body = fixture.body
        item.attachments = fixture.attachments
        item.isOutgoing = outgoing
        item.createdAtSecs = 1_790_157_600
        item.expiresAtSecs = nil
        item.delivery = .seen
        item.reactions = reacted ? [MessageReactionSnapshot(emoji: "👍", count: 2, reactedByMe: false)] : []
        if fixture.hasReply {
            var quoted = item
            quoted.body = "A quoted message"
            quoted.attachments = []
            item.body = replyEncodedMessage(reply: quoted, text: fixture.body)
        }
        return item
    }

    private func content(_ item: ChatMessageSnapshot, kind: ChatKind, footer: Bool, active: Bool,
                         width: CGFloat, onFrames: @escaping ([String: CGRect]) -> Void) -> some View {
        ChatMessageRow(message: item, chatKind: kind, showDayChip: false, hidesInlineDayChip: true,
            isFirstInCluster: true, isLastInCluster: true, showsFooter: footer,
            showsGroupSenderName: kind == .group && !item.isOutgoing,
            showsGroupSenderAvatar: kind == .group && !item.isOutgoing,
            reactions: item.reactions, swipeOffset: 0, isActionDockActive: active,
            onActionDockActiveChange: { _ in }, onReply: {}, onForward: {}, onForwardAttachment: { _ in },
            onReact: { _ in }, onInfo: {}, onDelete: {}, onScrollToQuote: { _ in }, onShowReactors: {},
            downloadAttachment: { _ in nil }, openAttachment: { _ in }, onOpenImage: { _, _ in })
            .padding(24)
            .frame(width: width, height: 1200, alignment: .topLeading)
            .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
            .onPreferenceChange(ChatMessageContentFramePreferenceKey.self) { onFrames($0.frames) }
            .environment(\.irisPalette, .dark)
            .environment(\.colorScheme, .dark)
            .background(Color.black)
    }

    @discardableResult
    private func qualify(_ fixture: Fixture, kind: ChatKind, outgoing: Bool, reacted: Bool,
                         width: CGFloat) throws -> CGRect {
        let item = message(fixture, outgoing: outgoing, reacted: reacted)
        var frames: [String: CGRect] = [:]
        func view(_ active: Bool) -> some View {
            content(item, kind: kind, footer: fixture.showsFooter, active: active, width: width) { frames = $0 }
        }
        let host = NSHostingView(rootView: view(false))
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: width, height: 1200),
                              styleMask: .borderless, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        host.frame = CGRect(x: 0, y: 0, width: width, height: 1200)
        defer { window.close() }
        let context = "\(fixture.name), \(kind), outgoing=\(outgoing), reacted=\(reacted), width=\(width)"
        try waitForLayout(host) { frames[item.id] != nil && self.actionFrames(in: host).isEmpty }
        let hidden = try XCTUnwrap(frames[item.id], context)

        host.rootView = view(true)
        try waitForLayout(host) { frames[item.id] != nil && self.actionFrames(in: host).count == 4 }
        let visible = try XCTUnwrap(frames[item.id], context)
        XCTAssertEqual(visible.minX, hidden.minX, accuracy: 0.5, context)
        XCTAssertEqual(visible.minY, hidden.minY, accuracy: 0.5, context)
        XCTAssertEqual(visible.width, hidden.width, accuracy: 0.5, context)
        XCTAssertEqual(visible.height, hidden.height, accuracy: 0.5, context)
        XCTAssertLessThanOrEqual(visible.width, IrisLayout.chatBubbleMaxWidth + 0.5, context)

        let actions = actionFrames(in: host).values.map { screenFrame -> CGRect in
            let local = host.convert(window.convertFromScreen(screenFrame), from: nil)
            return host.isFlipped ? local : CGRect(x: local.minX, y: host.bounds.maxY - local.maxY,
                                                  width: local.width, height: local.height)
        }
        let buttons = try XCTUnwrap(actions.reduce(nil as CGRect?) { $0?.union($1) ?? $1 }, context)
        // ChatMessageActionDock pads its actual buttons by five points. Measure
        // their native bounds rather than rendering a replacement dock/rectangle
        // or depending on the capsule's rasterized color and antialiasing.
        let dockInset: CGFloat = 5
        XCTAssertEqual(buttons.width, ChatMessageActionDock.dockWidth - 2 * dockInset,
                       accuracy: 0.5, "Accessibility must expose full button bounds: \(context)")
        for action in actions {
            XCTAssertEqual(action.maxY, buttons.maxY, accuracy: 0.5, context)
        }
        let gap = (outgoing ? visible.minX - buttons.maxX : buttons.minX - visible.maxX) - dockInset
        XCTAssertEqual(gap, SignalConversationLayout.messageStackSpacing, accuracy: 0.5,
                       "Dock must follow the actual bubble edge: \(context)")
        XCTAssertEqual(buttons.maxY + dockInset, visible.maxY, accuracy: 0.5,
                       "Reaction space must not lower the dock: \(context)")

        if ["short", "reply-footer", "image-caption", "file"].contains(fixture.name)
            && kind == .group && reacted && width == 700 {
            let renderer = ImageRenderer(content: host.rootView)
            renderer.scale = 2
            let image = try XCTUnwrap(renderer.cgImage, context)
            let attachment = XCTAttachment(image: NSImage(cgImage: image, size: .zero))
            attachment.name = "real-dock-\(fixture.name)-group-\(outgoing ? "outgoing" : "incoming")-reacted"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        host.rootView = view(false)
        try waitForLayout(host) { self.actionFrames(in: host).isEmpty }
        let hiddenAgain = try XCTUnwrap(frames[item.id], context)
        XCTAssertEqual(hiddenAgain.minX, visible.minX, accuracy: 0.5, context)
        XCTAssertEqual(hiddenAgain.minY, visible.minY, accuracy: 0.5, context)
        XCTAssertEqual(hiddenAgain.width, visible.width, accuracy: 0.5, context)
        XCTAssertEqual(hiddenAgain.height, visible.height, accuracy: 0.5, context)
        return visible
    }

    private func waitForLayout(_ host: NSView, ready: () -> Bool) throws {
        let deadline = Date().addingTimeInterval(2)
        var readyPasses = 0
        repeat {
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
            readyPasses = ready() ? readyPasses + 1 : 0
            if readyPasses >= 3 { return }
        } while Date() < deadline
        XCTFail("Production row did not expose stable bubble/action geometry")
        throw CaptureError.missingGeometry
    }

    private func actionFrames(in host: NSView) -> [String: CGRect] {
        let identifiers: Set<String> = ["messageReactButton", "messageReplyButton", "messageInfoButton", "messageMoreButton"]
        var frames: [String: CGRect] = [:]
        var visited: Set<ObjectIdentifier> = []
        func walk(_ object: NSObject) {
            guard visited.insert(ObjectIdentifier(object)).inserted else { return }
            if object.responds(to: NSSelectorFromString("accessibilityIdentifier")),
               let identifier = object.value(forKey: "accessibilityIdentifier") as? String,
               identifiers.contains(identifier), object.responds(to: NSSelectorFromString("accessibilityFrame")),
               let value = object.value(forKey: "accessibilityFrame") as? NSValue,
               value.rectValue.width > 0 && value.rectValue.height > 0 {
                frames[identifier] = value.rectValue
            }
            if object.responds(to: NSSelectorFromString("accessibilityChildren")),
               let children = object.value(forKey: "accessibilityChildren") as? [NSObject] {
                children.forEach { walk($0) }
            }
            if let view = object as? NSView { view.subviews.forEach { walk($0) } }
        }
        walk(host)
        return frames
    }

    private enum CaptureError: Error { case missingGeometry }
}
#endif
