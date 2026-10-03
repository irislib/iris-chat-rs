import XCTest
#if os(macOS)
import AppKit
import SwiftUI
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ChatMessageGroupingTests: XCTestCase {
    func testSameSenderClustersForLessThanThreeMinutesOnBothChatKinds() {
        let first = message(at: 1_790_157_600)
        for kind in [ChatKind.direct, .group] {
            for gap: UInt64 in [0, 1, 60, 119, 179] {
                XCTAssertFalse(irisStartsMessageCluster(previous: first,
                    message: message(at: first.createdAtSecs + gap), chatKind: kind))
            }
            XCTAssertTrue(irisStartsMessageCluster(previous: first,
                message: message(at: first.createdAtSecs + 180), chatKind: kind))
        }
    }

    func testReactionsBreakOnlyTheFollowingBoundary() {
        let first = message(at: 1_790_157_600)
        var reacted = message(at: first.createdAtSecs + 1)
        reacted.reactions = [MessageReactionSnapshot(emoji: "👍", count: 1, reactedByMe: false)]
        XCTAssertFalse(irisStartsMessageCluster(previous: first, message: reacted, chatKind: .direct))
        XCTAssertTrue(irisStartsMessageCluster(previous: reacted,
            message: message(at: first.createdAtSecs + 2), chatKind: .direct))
    }

    func testSenderIdentitySurvivesNameChangesAndSeparatesIdenticalNames() {
        var first = message(at: 1_790_157_600)
        first.isOutgoing = false
        first.author = "Robin"
        first.authorOwnerPubkeyHex = "person-a"
        var next = first
        next.createdAtSecs += 1
        next.author = "Robin renamed"
        XCTAssertFalse(irisStartsMessageCluster(previous: first, message: next, chatKind: .group))
        XCTAssertFalse(irisShowsGroupSenderName(previous: first, message: next, chatKind: .group))
        next.author = first.author
        next.authorOwnerPubkeyHex = "person-b"
        XCTAssertTrue(irisStartsMessageCluster(previous: first, message: next, chatKind: .group))
        XCTAssertTrue(irisShowsGroupSenderAvatar(message: first, next: next, chatKind: .group))
    }

    func testDirectionNoticesDayAndReverseTimeSeparateClusters() {
        let first = message(at: 1_790_157_600)
        var next = message(at: first.createdAtSecs + 1)
        next.isOutgoing = false
        XCTAssertTrue(irisStartsMessageCluster(previous: first, message: next, chatKind: .direct))
        next = first
        next.kind = .system
        XCTAssertTrue(irisStartsMessageCluster(previous: first, message: next, chatKind: .direct))
        XCTAssertTrue(irisStartsMessageCluster(previous: next, message: first, chatKind: .direct))
        next = message(at: first.createdAtSecs - 1)
        XCTAssertTrue(irisStartsMessageCluster(previous: first, message: next, chatKind: .direct))
        let midnight = Calendar.current.startOfDay(for: Date(timeIntervalSince1970: Double(first.createdAtSecs)))
        let boundary = UInt64(midnight.timeIntervalSince1970)
        XCTAssertTrue(irisStartsMessageCluster(previous: message(at: boundary - 1),
            message: message(at: boundary + 1), chatKind: .direct))
    }

    func testFootersHideOnlyRedundantTimeAndDeliveryWithinACluster() {
        var first = message(at: 1_790_157_600)
        first.delivery = .seen
        var next = first
        next.createdAtSecs += 1
        XCTAssertFalse(irisShowsMessageFooter(message: first, next: next, chatKind: .direct))
        for delivery in [DeliveryState.queued, .pending, .failed, .sent, .received] {
            first.delivery = delivery
            XCTAssertTrue(irisShowsMessageFooter(message: first, next: next, chatKind: .direct))
        }
        first.delivery = .seen
        first.expiresAtSecs = first.createdAtSecs + 600
        XCTAssertTrue(irisShowsMessageFooter(message: first, next: next, chatKind: .direct))
        first.expiresAtSecs = nil
        next.createdAtSecs = first.createdAtSecs + 60
        XCTAssertFalse(irisStartsMessageCluster(previous: first, message: next, chatKind: .direct))
        XCTAssertTrue(irisShowsMessageFooter(message: first, next: next, chatKind: .direct))
        XCTAssertTrue(irisShowsMessageFooter(message: first, next: nil, chatKind: .direct))
    }

#if os(macOS)
    @MainActor
    func testProductionShortBubblesKeepTwoPointGapsIncludingTheFooterRow() throws {
        for outgoing in [false, true] {
            let messages = (0..<4).map { index in
                var item = message(at: 1_790_157_600 + UInt64(index))
                item.id = "cluster-\(index)"
                item.body = ["Hello", "Yes", "Great", "See you soon"][index]
                item.isOutgoing = outgoing
                item.delivery = .seen
                return item
            }
            var frames: [String: CGRect] = [:]
            let content = VStack(spacing: 0) {
                ForEach(Array(messages.enumerated()), id: \.element.id) { index, item in
                    self.clusterRow(item, previous: index > 0 ? messages[index - 1] : nil,
                               next: index + 1 < messages.count ? messages[index + 1] : nil)
                }
            }
            .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
            .onPreferenceChange(ChatMessageContentFramePreferenceKey.self) { frames = $0 }
            .padding(24).frame(width: 700)
            .background(Color.black)
            .environment(\.colorScheme, .dark)
            .environment(\.irisPalette, .dark)
            let host = NSHostingView(rootView: content)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            host.layoutSubtreeIfNeeded()
            for index in 1..<messages.count {
                let previous = try XCTUnwrap(frames[messages[index - 1].id])
                let current = try XCTUnwrap(frames[messages[index].id])
                XCTAssertEqual(current.minY - previous.maxY, 2, accuracy: 0.5)
            }
            let renderer = ImageRenderer(content: content)
            renderer.scale = 2
            let image = try XCTUnwrap(renderer.cgImage)
            let attachment = XCTAttachment(image: NSImage(cgImage: image, size: .zero))
            attachment.name = "cluster-\(outgoing ? "outgoing" : "incoming")"
            attachment.lifetime = .keepAlways
            add(attachment)
            if let directory = ProcessInfo.processInfo.environment["IRIS_CLUSTER_LAYOUT_ARTIFACT_DIR"] {
                let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
                try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("cluster-\(outgoing ? "outgoing" : "incoming").png"))
            }
        }
    }

    @MainActor
    private func clusterRow(_ item: ChatMessageSnapshot, previous: ChatMessageSnapshot?,
                            next: ChatMessageSnapshot?) -> some View {
        ChatMessageRow(message: item, chatKind: .direct, showDayChip: false,
            hidesInlineDayChip: true,
            isFirstInCluster: irisStartsMessageCluster(previous: previous, message: item, chatKind: .direct),
            isLastInCluster: next.map { irisStartsMessageCluster(previous: item, message: $0, chatKind: .direct) } ?? true,
            showsFooter: irisShowsMessageFooter(message: item, next: next, chatKind: .direct),
            showsGroupSenderName: false, showsGroupSenderAvatar: false, reactions: [],
            swipeOffset: 0, isActionDockActive: false, onActionDockActiveChange: { _ in },
            onReply: {}, onForward: {}, onForwardAttachment: { _ in }, onReact: { _ in },
            onInfo: {}, onDelete: {}, onScrollToQuote: { _ in }, onShowReactors: {},
            downloadAttachment: { _ in nil }, openAttachment: { _ in }, onOpenImage: { _, _ in })
    }
#endif

    private func message(at seconds: UInt64) -> ChatMessageSnapshot {
        var message = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0,
            messagesInCurrentChat: 1).currentChat!.messages[0]
        message.kind = .user
        message.isOutgoing = true
        message.createdAtSecs = seconds
        message.reactions = []
        message.expiresAtSecs = nil
        return message
    }
}
