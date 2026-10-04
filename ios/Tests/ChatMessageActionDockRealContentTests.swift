#if os(macOS)
import AppKit
import Darwin
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

    func testWideLazyTimelineKeepsTheSelectedDockBesideItsRealBubble() throws {
        var long = message(Fixture(name: "wide-paragraph",
            body: String(repeating: "Warm blankets and a small map for the woodland walk. ", count: 10)),
            outgoing: false, reacted: false)
        long.author = "Lee"
        long.authorOwnerPubkeyHex = String(repeating: "2", count: 64)
        var incoming = message(Fixture(name: "wide-short", body: "Hi"), outgoing: false, reacted: true)
        incoming.author = "Tim"
        var outgoing = message(Fixture(name: "wide-outgoing", body: "Hi"), outgoing: true, reacted: true)
        outgoing.author = "You"
        let items = [long, incoming, outgoing]
        let sink = FrameSink()
        var activeID: String? = nil
        let renderer = ImageRenderer(content: timeline(items, activeID: activeID, sink: sink))
        renderer.scale = 2
        var snapshots: [(String, Snapshot)] = []
        var captureFailed = false
        let failuresBefore = testRun?.failureCount ?? 0
        defer {
            if captureFailed || (testRun?.failureCount ?? 0) > failuresBefore {
                for (stage, snapshot) in snapshots {
                    retain(snapshot.image, named: "real-dock-failure-wide-timeline-\(stage)")
                }
            }
        }
        func captureStage(_ stage: String) throws -> Snapshot {
            let snapshot = try captureRendered(stage: stage, name: "wide-timeline", message: incoming) {
                let image = renderer.cgImage
                return (image, sink.frames)
            }
            snapshots.append((stage, snapshot))
            return snapshot
        }
        do {
            let hidden = try captureStage("initial-hidden")
            for selected in [incoming, outgoing] {
                activeID = irisNextActiveMessageActionDockId(
                    current: activeID, messageId: selected.id, isActive: true)
                XCTAssertEqual(activeID, selected.id)
                // Keep the renderer's view graph: the production EquatableView
                // must update when only the selected message ID changes.
                renderer.content = timeline(items, activeID: activeID, sink: sink)
                let visible = try captureStage(selected.isOutgoing ? "outgoing-visible" : "incoming-visible")
                try assertTimelineFrames(visible, match: hidden, items: items)
                guard let bubble = visible.frames[selected.id] else {
                    throw CaptureError(reason: "Wide timeline selected bubble preference missing: \(selected.id)")
                }
                try assertDock(visible, from: hidden, bubble: bubble, outgoing: selected.isOutgoing,
                    context: "927 pt lazy timeline, selected=\(selected.id)")
                retain(visible.image, named: "real-dock-wide-timeline-\(selected.isOutgoing ? "outgoing" : "incoming")-reacted")
            }
            activeID = irisNextActiveMessageActionDockId(current: activeID, messageId: incoming.id, isActive: false)
            XCTAssertEqual(activeID, outgoing.id, "A late leave from the prior row must keep the selected dock")
            renderer.content = timeline(items, activeID: activeID, sink: sink)
            let stillOutgoing = try captureStage("outgoing-after-prior-leave")
            try assertTimelineFrames(stillOutgoing, match: hidden, items: items)
            guard let outgoingBubble = stillOutgoing.frames[outgoing.id] else {
                throw CaptureError(reason: "Wide timeline outgoing bubble preference missing after prior leave")
            }
            try assertDock(stillOutgoing, from: hidden, bubble: outgoingBubble, outgoing: true,
                context: "927 pt lazy timeline, prior incoming row left")
            activeID = irisNextActiveMessageActionDockId(current: activeID, messageId: outgoing.id, isActive: false)
            XCTAssertNil(activeID)
            renderer.content = timeline(items, activeID: activeID, sink: sink)
            let hiddenAgain = try captureStage("hidden-again")
            try assertTimelineFrames(hiddenAgain, match: hidden, items: items)
            XCTAssertTrue(hidden.raster.sameOutsideBubble(as: hiddenAgain.raster, bubble: hidden.bubble),
                          "Wide timeline hidden baselines must agree outside the actual bubble")
        } catch {
            captureFailed = true
            XCTFail("Rendered production timeline capture failed: \(error)")
            throw error
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
        row(item, kind: kind, footer: footer, active: active)
            .padding(24)
            .frame(width: width, height: 1200, alignment: .topLeading)
            .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
            .overlayPreferenceValue(ChatMessageContentFramePreferenceKey.self) { value in
                self.readFrames(value.frames, into: onFrames)
            }
            .environment(\.irisPalette, .dark)
            .environment(\.colorScheme, .dark)
            .background(Color.black)
    }

    private func row(_ item: ChatMessageSnapshot, kind: ChatKind, footer: Bool, active: Bool) -> ChatMessageRow {
        ChatMessageRow(message: item, chatKind: kind, showDayChip: false, hidesInlineDayChip: true,
            isFirstInCluster: true, isLastInCluster: true, showsFooter: footer,
            showsGroupSenderName: kind == .group && !item.isOutgoing,
            showsGroupSenderAvatar: kind == .group && !item.isOutgoing,
            reactions: item.reactions, swipeOffset: 0, isActionDockActive: active,
            onActionDockActiveChange: { _ in }, onReply: {}, onForward: {}, onForwardAttachment: { _ in },
            onReact: { _ in }, onInfo: {}, onDelete: {}, onScrollToQuote: { _ in }, onShowReactors: {},
            downloadAttachment: { _ in nil }, openAttachment: { _ in }, onOpenImage: { _, _ in })
    }

    private func timeline(_ items: [ChatMessageSnapshot], activeID: String?, sink: FrameSink) -> some View {
        GeometryReader { viewport in
            ScrollView {
                ChatTimelineContentLayout {
                    ChatTimelineMessageLayout {
                        ForEach(items, id: \.id) { item in
                            EquatableView(content: row(item, kind: .group, footer: false, active: activeID == item.id))
                                .id(item.id)
                        }
                    }
                    .padding(.horizontal, 18)
                    .padding(.top, SignalConversationLayout.contentTopMargin)
                    .padding(.bottom, SignalConversationLayout.contentBottomMargin)
                    .frame(width: viewport.size.width)
                    .frame(minHeight: viewport.size.height, alignment: .bottom)
                    Color.clear.frame(height: 1).accessibilityHidden(true)
                }
            }
            .irisDefaultScrollAnchorBottom()
            .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
        }
            .frame(width: 927, height: 600)
            .overlayPreferenceValue(ChatMessageContentFramePreferenceKey.self) { value in
                self.readFrames(value.frames) { sink.frames = $0 }
            }
            .environment(\.irisPalette, .dark)
            .environment(\.colorScheme, .dark)
            .background(Color.black)
    }

    // Read the production preference during this renderer's layout. This sink
    // is not SwiftUI state and adds no geometry probe to the production row.
    private func readFrames(_ frames: [String: CGRect], into receive: ([String: CGRect]) -> Void) -> some View {
        receive(frames)
        return Color.clear.allowsHitTesting(false).accessibilityHidden(true)
    }

    private final class FrameSink {
        var frames: [String: CGRect] = [:]
    }

    private struct Snapshot {
        let image: CGImage
        let bubble: CGRect
        let raster: Raster
        let frames: [String: CGRect]
    }

    @discardableResult
    private func qualify(_ fixture: Fixture, kind: ChatKind, outgoing: Bool, reacted: Bool,
                         width: CGFloat) throws -> CGRect {
        let item = message(fixture, outgoing: outgoing, reacted: reacted)
        let context = "\(fixture.name), \(kind), outgoing=\(outgoing), reacted=\(reacted), width=\(width)"
        let name = "\(fixture.name)-\(kind)-\(outgoing ? "outgoing" : "incoming")-\(reacted ? "reacted" : "plain")-\(Int(width))"
        var snapshots: [(String, Snapshot)] = []
        var captureFailed = false
        let failuresBefore = testRun?.failureCount ?? 0
        defer {
            if captureFailed || (testRun?.failureCount ?? 0) > failuresBefore {
                for (stage, snapshot) in snapshots {
                    retain(snapshot.image, named: "real-dock-failure-\(name)-\(stage)")
                }
            }
        }
        func view(_ active: Bool, sink: FrameSink) -> some View {
            content(item, kind: kind, footer: fixture.showsFooter, active: active, width: width) { sink.frames = $0 }
        }
        do {
            let hidden = try capture(stage: "initial-hidden", name: name,
                                     message: item) { view(false, sink: $0) }
            snapshots.append(("initial-hidden", hidden))
            let visible = try capture(stage: "visible", name: name,
                                      message: item) { view(true, sink: $0) }
            snapshots.append(("visible", visible))
            let hiddenAgain = try capture(stage: "hidden-again", name: name,
                                          message: item) { view(false, sink: $0) }
            snapshots.append(("hidden-again", hiddenAgain))
            for snapshot in [hidden, hiddenAgain] {
                XCTAssertEqual(snapshot.bubble.minX, visible.bubble.minX, accuracy: 0.5, context)
                XCTAssertEqual(snapshot.bubble.minY, visible.bubble.minY, accuracy: 0.5, context)
                XCTAssertEqual(snapshot.bubble.width, visible.bubble.width, accuracy: 0.5, context)
                XCTAssertEqual(snapshot.bubble.height, visible.bubble.height, accuracy: 0.5, context)
            }
            XCTAssertLessThanOrEqual(visible.bubble.width, IrisLayout.chatBubbleMaxWidth + 0.5, context)

            // Image loads/placeholders can change within the bubble. The real
            // bubble frame is the only excluded region: avatar, reactions, and
            // every other pixel must agree in the hidden baselines. Both the
            // preference and capsule pixels belong to this same renderer.
            guard hidden.raster.sameOutsideBubble(as: hiddenAgain.raster, bubble: visible.bubble) else {
                throw CaptureError(reason: "Hidden baselines differ outside the actual bubble")
            }
            try assertDock(visible, from: hidden, bubble: visible.bubble, outgoing: outgoing, context: context)

            if ["short", "reply-footer", "image-caption", "file"].contains(fixture.name)
                && kind == .group && reacted && width == 700 {
                retain(visible.image, named: "real-dock-\(fixture.name)-group-\(outgoing ? "outgoing" : "incoming")-reacted")
            }
            return visible.bubble
        } catch {
            captureFailed = true
            XCTFail("Rendered production row capture failed [\(context)]: \(error)")
            throw error
        }
    }

    private func assertTimelineFrames(_ snapshot: Snapshot, match baseline: Snapshot,
                                      items: [ChatMessageSnapshot]) throws {
        for item in items {
            guard let bubble = snapshot.frames[item.id], let before = baseline.frames[item.id],
                  bubble.width > 0, bubble.height > 0, before.width > 0, before.height > 0,
                  bubble.minX.isFinite, bubble.minY.isFinite, bubble.maxX.isFinite, bubble.maxY.isFinite,
                  before.minX.isFinite, before.minY.isFinite, before.maxX.isFinite, before.maxY.isFinite else {
                throw CaptureError(reason: "Wide timeline bubble preference missing/empty/nonfinite: \(item.id)")
            }
            let context = "927 pt lazy timeline, bubble=\(item.id)"
            XCTAssertEqual(bubble.minX, before.minX, accuracy: 0.5, context)
            XCTAssertEqual(bubble.minY, before.minY, accuracy: 0.5, context)
            XCTAssertEqual(bubble.width, before.width, accuracy: 0.5, context)
            XCTAssertEqual(bubble.height, before.height, accuracy: 0.5, context)
            XCTAssertLessThanOrEqual(bubble.width, IrisLayout.chatBubbleMaxWidth + 0.5, context)
        }
    }

    private func assertDock(_ visible: Snapshot, from hidden: Snapshot, bubble: CGRect,
                            outgoing: Bool, context: String) throws {
        let components = try visible.raster.changedComponents(from: hidden.raster,
            excluding: bubble, toolbar: IrisPalette.dark.toolbar)
        guard components.count == 1 else {
            throw CaptureError(reason: "Expected one real dock pixel component; found \(components.count), firstBounds=\(components.prefix(8).map { $0.bounds })")
        }
        let component = components[0]
        guard component.toolbarPixels >= 4 else {
            throw CaptureError(reason: "Changed component has no unambiguous actual toolbar fill")
        }
        let dock = component.bounds
        XCTAssertEqual(dock.width, 136, accuracy: 0.5, "Actual rendered dock width: \(context)")
        XCTAssertEqual(dock.height, 38, accuracy: 0.5, "Actual rendered dock height: \(context)")
        let gap = outgoing ? bubble.minX - dock.maxX : dock.minX - bubble.maxX
        XCTAssertEqual(gap, 8, accuracy: 0.5, "Dock must follow the actual bubble edge: \(context)")
        XCTAssertEqual(dock.maxY, bubble.maxY, accuracy: 0.5,
                       "Reaction space must not lower the dock: \(context)")
    }

    private func capture<Content: View>(stage: String, name: String,
                                       message: ChatMessageSnapshot,
                                       view: (FrameSink) -> Content) throws -> Snapshot {
        try captureRendered(stage: stage, name: name, message: message) {
            let sink = FrameSink()
            let renderer = ImageRenderer(content: view(sink))
            renderer.scale = 2
            let image = renderer.cgImage
            return (image, sink.frames)
        }
    }

    private func captureRendered(stage: String, name: String, message: ChatMessageSnapshot,
                                 render: () -> (CGImage?, [String: CGRect])) throws -> Snapshot {
        let deadline = Date().addingTimeInterval(2)
        var readyPasses = 0
        var previous: Snapshot?
        var latestImage: CGImage?
        var diagnostic = "No rendered image"
        repeat {
            let (renderedImage, frames) = render()
            if let image = renderedImage {
                latestImage = image
                if let bubble = frames[message.id], bubble.width > 0, bubble.height > 0,
                   bubble.minX.isFinite, bubble.minY.isFinite, bubble.maxX.isFinite, bubble.maxY.isFinite {
                    do {
                        let raster = try Raster(image: image, bubble: bubble,
                            bubbleColor: message.isOutgoing ? IrisPalette.dark.bubbleMine : IrisPalette.dark.bubbleTheirs)
                        let snapshot = Snapshot(image: image, bubble: bubble, raster: raster, frames: frames)
                        if let previous, previous.bubble == bubble,
                           raster.sameOutsideBubble(as: previous.raster, bubble: bubble) {
                            readyPasses += 1
                        } else {
                            readyPasses = 1
                        }
                        previous = snapshot
                        if readyPasses >= 3 { return snapshot }
                        diagnostic = "bubble=\(bubble), raster=\(image.width)x\(image.height), stablePasses=\(readyPasses)/3"
                    } catch {
                        readyPasses = 0
                        previous = nil
                        diagnostic = "bubble=\(bubble), raster=\(image.width)x\(image.height), \(error)"
                    }
                } else {
                    readyPasses = 0
                    previous = nil
                    diagnostic = "Production bubble preference missing/empty/nonfinite, raster=\(image.width)x\(image.height)"
                }
            } else {
                readyPasses = 0
                previous = nil
            }
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        } while Date() < deadline
        if let latestImage { retain(latestImage, named: "real-dock-capture-failure-\(name)-\(stage)") }
        throw CaptureError(reason: "stage=\(stage), stablePasses=\(readyPasses)/3, \(diagnostic)")
    }

    private func retain(_ image: CGImage, named name: String) {
        let attachment = XCTAttachment(image: NSImage(cgImage: image, size: .zero))
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private struct CaptureError: Error, CustomStringConvertible {
        let reason: String
        var description: String { reason }
    }

    private struct Component {
        let bounds: CGRect
        let toolbarPixels: Int
    }

    private struct Raster {
        static let scale: CGFloat = 2
        let width: Int
        let height: Int
        let bytes: [UInt8]
        let flipped: Bool

        init(image: CGImage, bubble: CGRect, bubbleColor: Color) throws {
            let width = image.width
            let height = image.height
            var rgba = [UInt8](repeating: 0, count: width * height * 4)
            let drew = rgba.withUnsafeMutableBytes { storage -> Bool in
                guard let space = CGColorSpace(name: CGColorSpace.sRGB),
                      let context = CGContext(data: storage.baseAddress, width: width, height: height,
                        bitsPerComponent: 8, bytesPerRow: width * 4, space: space,
                        bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
                context.draw(image, in: CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)))
                return true
            }
            guard drew else { throw CaptureError(reason: "Cannot normalize actual rendered pixels to sRGB RGBA") }
            let fill = try Self.rgb(bubbleColor)
            // Determine bitmap orientation from real bubble padding paint, not
            // from an assumed dock location. Reject missing/ambiguous evidence.
            let points = [CGPoint(x: bubble.midX, y: bubble.minY + 2),
                          CGPoint(x: bubble.minX + 2, y: bubble.midY),
                          CGPoint(x: bubble.maxX - 2, y: bubble.midY),
                          CGPoint(x: bubble.midX, y: bubble.maxY - 2)]
            let orientations = [false, true].filter { flip in
                points.filter { point in
                    let x = Int((point.x * Self.scale).rounded(.down))
                    let topY = Int((point.y * Self.scale).rounded(.down))
                    let y = flip ? height - 1 - topY : topY
                    guard x >= 0, x < width, y >= 0, y < height else { return false }
                    let offset = (y * width + x) * 4
                    return abs(Int(rgba[offset]) - fill.0) <= 2 &&
                        abs(Int(rgba[offset + 1]) - fill.1) <= 2 &&
                        abs(Int(rgba[offset + 2]) - fill.2) <= 2
                }.count >= 2
            }
            guard orientations.count == 1 else {
                throw CaptureError(reason: "Actual bubble paint cannot establish a unique raster coordinate orientation (\(orientations.count) candidates)")
            }
            self.width = width
            self.height = height
            self.bytes = rgba
            self.flipped = orientations[0]
        }

        private static func rgb(_ color: Color, extraOpacity: CGFloat = 1) throws -> (Int, Int, Int) {
            guard let resolved = NSColor(color).usingColorSpace(.sRGB) else {
                throw CaptureError(reason: "Cannot resolve actual production palette fill")
            }
            let alpha = resolved.alphaComponent * extraOpacity
            return (Int((resolved.redComponent * alpha * 255).rounded()),
                    Int((resolved.greenComponent * alpha * 255).rounded()),
                    Int((resolved.blueComponent * alpha * 255).rounded()))
        }

        private func row(_ y: Int) -> Int { (flipped ? height - 1 - y : y) * width * 4 }

        private func excludedPixels(_ bubble: CGRect) -> (x: Range<Int>, y: Range<Int>) {
            // Exclude only pixel centers inside the actual preference frame.
            let x0 = max(0, min(width, Int(ceil(bubble.minX * Self.scale - 0.5))))
            let x1 = max(x0, min(width, Int(ceil(bubble.maxX * Self.scale - 0.5))))
            let y0 = max(0, min(height, Int(ceil(bubble.minY * Self.scale - 0.5))))
            let y1 = max(y0, min(height, Int(ceil(bubble.maxY * Self.scale - 0.5))))
            return (x0..<x1, y0..<y1)
        }

        func sameOutsideBubble(as other: Raster, bubble: CGRect) -> Bool {
            guard width == other.width, height == other.height else { return false }
            let excluded = excludedPixels(bubble)
            return bytes.withUnsafeBufferPointer { first in
                other.bytes.withUnsafeBufferPointer { second in
                    for y in 0..<height {
                        let ranges = excluded.y.contains(y) ? [0..<excluded.x.lowerBound, excluded.x.upperBound..<width] : [0..<width]
                        for range in ranges where !range.isEmpty {
                            if memcmp(first.baseAddress! + row(y) + range.lowerBound * 4,
                                      second.baseAddress! + other.row(y) + range.lowerBound * 4,
                                      range.count * 4) != 0 { return false }
                        }
                    }
                    return true
                }
            }
        }

        func changedComponents(from hidden: Raster, excluding bubble: CGRect, toolbar: Color) throws -> [Component] {
            guard width == hidden.width, height == hidden.height else {
                throw CaptureError(reason: "Hidden/visible renderer dimensions disagree")
            }
            let fill = try Self.rgb(toolbar, extraOpacity: 0.96)
            // Half the actual capsule/background contrast identifies its
            // rasterized boundary to one scale-2 pixel (0.5 pt). This is a color
            // threshold, never a dock size/position used to fabricate bounds.
            let threshold = max(1, Int(ceil(Double(max(fill.0, max(fill.1, fill.2))) / 2)))
            let excluded = excludedPixels(bubble)
            var mask = [UInt8](repeating: 0, count: width * height)
            var changed: [Int] = []
            bytes.withUnsafeBufferPointer { visible in
                hidden.bytes.withUnsafeBufferPointer { before in
                    for y in 0..<height {
                        let vr = row(y)
                        let hr = hidden.row(y)
                        if memcmp(visible.baseAddress! + vr, before.baseAddress! + hr, width * 4) == 0 { continue }
                        for x in 0..<width {
                            if excluded.y.contains(y) && excluded.x.contains(x) { continue }
                            let v = vr + x * 4
                            let h = hr + x * 4
                            let delta = max(abs(Int(visible[v]) - Int(before[h])),
                                max(abs(Int(visible[v + 1]) - Int(before[h + 1])), abs(Int(visible[v + 2]) - Int(before[h + 2]))))
                            if delta >= threshold {
                                let index = y * width + x
                                mask[index] = 1
                                changed.append(index)
                            }
                        }
                    }
                }
            }
            var components: [Component] = []
            for origin in changed where mask[origin] != 0 {
                var queue = [origin]
                mask[origin] = 0
                var cursor = 0
                var minX = width, minY = height, maxX = 0, maxY = 0, toolbarPixels = 0
                while cursor < queue.count {
                    let index = queue[cursor]
                    cursor += 1
                    let x = index % width, y = index / width
                    minX = min(minX, x)
                    maxX = max(maxX, x)
                    minY = min(minY, y)
                    maxY = max(maxY, y)
                    let pixel = row(y) + x * 4
                    if abs(Int(bytes[pixel]) - fill.0) <= 1 &&
                        abs(Int(bytes[pixel + 1]) - fill.1) <= 1 &&
                        abs(Int(bytes[pixel + 2]) - fill.2) <= 1 { toolbarPixels += 1 }
                    for dy in -1...1 {
                        for dx in -1...1 where dx != 0 || dy != 0 {
                            let nx = x + dx, ny = y + dy
                            guard nx >= 0, nx < width, ny >= 0, ny < height else { continue }
                            let neighbor = ny * width + nx
                            if mask[neighbor] != 0 {
                                mask[neighbor] = 0
                                queue.append(neighbor)
                            }
                        }
                    }
                }
                components.append(Component(bounds: CGRect(x: CGFloat(minX) / Self.scale, y: CGFloat(minY) / Self.scale,
                    width: CGFloat(maxX - minX + 1) / Self.scale, height: CGFloat(maxY - minY + 1) / Self.scale), toolbarPixels: toolbarPixels))
            }
            return components
        }
    }
}
#endif
