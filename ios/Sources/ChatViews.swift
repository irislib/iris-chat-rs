import Foundation
import SwiftUI

struct ChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chatId: String

    @State private var draft = ""
    @State private var selectedAttachments: [StagedAttachment] = []
    @State private var isNearBottom = true
    @State private var shouldFollowLatest = true
    @State private var forceScrollToLatest = false
    @State private var timelineViewportMaxY: CGFloat = 0
    @State private var timelineBottomMaxY: CGFloat = .greatestFiniteMagnitude
    @State private var initialScrollPending = true
    @State private var renderedMessageCount = 0
    @State private var replyTarget: ChatMessageSnapshot?
    @State private var imageViewerItem: ImageViewerItem?
    @State private var lastTypingSentAt: Date?
    @State private var sentTypingIndicator = false
    @FocusState private var isComposerFocused: Bool

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var body: some View {
        VStack(spacing: 0) {
            Group {
                if let chat {
                    VStack(spacing: 0) {
                        ScrollViewReader { proxy in
                            ZStack(alignment: .bottomTrailing) {
                                ScrollView {
                                    let visibleMessages = chat.messages
                                    LazyVStack(spacing: 0) {
                                        ForEach(Array(visibleMessages.enumerated()), id: \.element.id) { index, message in
                                            let previous = index > 0 ? visibleMessages[index - 1] : nil
                                            let next = index + 1 < visibleMessages.count ? visibleMessages[index + 1] : nil
                                            let showDayChip = previous == nil || !irisSameTimelineDay(previous!.createdAtSecs, message.createdAtSecs)
                                            let isFirstInCluster = irisStartsMessageCluster(
                                                previous: previous,
                                                message: message,
                                                chatKind: chat.kind
                                            )
                                            let isLastInCluster = next.map {
                                                irisStartsMessageCluster(
                                                    previous: message,
                                                    message: $0,
                                                    chatKind: chat.kind
                                                )
                                            } ?? true

                                            ChatMessageRow(
                                                message: message,
                                                chatKind: chat.kind,
                                                showDayChip: showDayChip,
                                                isFirstInCluster: isFirstInCluster,
                                                isLastInCluster: isLastInCluster,
                                                reactions: message.reactions,
                                                onReply: {
                                                    replyTarget = message
                                                },
                                                onReact: { emoji in
                                                    manager.dispatch(
                                                        .toggleReaction(
                                                            chatId: chatId,
                                                            messageId: message.id,
                                                            emoji: emoji
                                                        )
                                                    )
                                                },
                                                onDelete: {
                                                    manager.dispatch(
                                                        .deleteLocalMessage(chatId: chatId, messageId: message.id)
                                                    )
                                                    if replyTarget?.id == message.id {
                                                        replyTarget = nil
                                                    }
                                                },
                                                downloadAttachment: { attachment in
                                                    await manager.downloadAttachment(attachment)
                                                },
                                                openAttachment: { attachment in
                                                    await manager.openAttachment(attachment)
                                                },
                                                onOpenImage: { data, filename in
                                                    imageViewerItem = ImageViewerItem(data: data, filename: filename)
                                                }
                                            )
                                            .id(message.id)
                                        }

                                        Color.clear
                                            .frame(height: 1)
                                            .id(ChatTimelineAnchor.bottom)
                                            .background(
                                                GeometryReader { geometry in
                                                    Color.clear.preference(
                                                        key: ChatTimelineBottomMaxYPreferenceKey.self,
                                                        value: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name)).maxY
                                                    )
                                                }
                                            )
                                            .accessibilityHidden(true)
                                    }
                                    .padding(.horizontal, IrisLayout.usesDesktopChrome ? 18 : 14)
                                    .padding(.vertical, 10)
                                    .accessibilityIdentifier("chatTimeline")
                                }
                                .simultaneousGesture(
                                    TapGesture().onEnded {
                                        isComposerFocused = false
                                    }
                                )
                                .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
                                .overlay {
                                    GeometryReader { geometry in
                                        Color.clear.preference(
                                            key: ChatTimelineViewportMaxYPreferenceKey.self,
                                            value: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name)).maxY
                                        )
                                    }
                                }
                                .irisInteractiveKeyboardDismiss()
                                .irisOnChange(of: chatId) { _ in
                                    initialScrollPending = true
                                    isNearBottom = true
                                    shouldFollowLatest = true
                                    forceScrollToLatest = false
                                    renderedMessageCount = 0
                                    lastTypingSentAt = nil
                                    sentTypingIndicator = false
                                }
                                .onPreferenceChange(ChatTimelineViewportMaxYPreferenceKey.self) { value in
                                    timelineViewportMaxY = value
                                    let nearBottom = chatTimelineIsNearBottom(
                                        viewportMaxY: value,
                                        bottomMaxY: timelineBottomMaxY
                                    )
                                    isNearBottom = nearBottom
                                    if chat.messages.count == renderedMessageCount {
                                        shouldFollowLatest = nearBottom
                                    }
                                }
                                .onPreferenceChange(ChatTimelineBottomMaxYPreferenceKey.self) { value in
                                    timelineBottomMaxY = value
                                    let nearBottom = chatTimelineIsNearBottom(
                                        viewportMaxY: timelineViewportMaxY,
                                        bottomMaxY: value
                                    )
                                    isNearBottom = nearBottom
                                    if chat.messages.count == renderedMessageCount {
                                        shouldFollowLatest = nearBottom
                                    }
                                }
                                .task(id: chat.messages.last?.id) {
                                    guard !chat.messages.isEmpty else {
                                        initialScrollPending = true
                                        shouldFollowLatest = true
                                        forceScrollToLatest = false
                                        renderedMessageCount = 0
                                        return
                                    }
                                    let messageCount = chat.messages.count
                                    let messageCountIncreased = messageCount > renderedMessageCount
                                    let shouldScroll = initialScrollPending
                                        || forceScrollToLatest
                                        || (messageCountIncreased && shouldFollowLatest)
                                    renderedMessageCount = messageCount
                                    if shouldScroll {
                                        scrollToBottom(proxy: proxy, animated: !initialScrollPending)
                                        initialScrollPending = false
                                        shouldFollowLatest = true
                                    }
                                    if forceScrollToLatest {
                                        forceScrollToLatest = false
                                    }
                                }
                                .task(id: forceScrollToLatest) {
                                    guard forceScrollToLatest, !chat.messages.isEmpty else {
                                        return
                                    }
                                    scrollToBottom(proxy: proxy, animated: true)
                                }

                                if !isNearBottom && !chat.messages.isEmpty {
                                    Button {
                                        isComposerFocused = false
                                        shouldFollowLatest = true
                                        scrollToBottom(proxy: proxy, animated: true)
                                    } label: {
                                        Image(systemName: "arrow.down")
                                            .font(.system(size: 18, weight: .bold))
                                            .foregroundStyle(palette.onAccent)
                                            .frame(width: 48, height: 48)
                                            .background(
                                                Circle()
                                                    .fill(palette.accent)
                                            )
                                    }
                                    .padding(.trailing, 18)
                                    .padding(.bottom, 18)
                                    .buttonStyle(.plain)
                                    .shadow(color: .black.opacity(0.16), radius: 16, y: 10)
                                    .accessibilityIdentifier("chatJumpToBottom")
                                }

                                if !chat.typingIndicators.isEmpty {
                                    IrisTypingIndicatorRow(indicators: chat.typingIndicators)
                                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                                        .padding(.leading, IrisLayout.usesDesktopChrome ? 22 : 16)
                                        .padding(.trailing, 76)
                                        .padding(.bottom, 16)
                                        .allowsHitTesting(false)
                                }
                            }
                        }

                        if let replyTarget {
                            IrisReplyComposerStrip(message: replyTarget) {
                                self.replyTarget = nil
                            }
                        }

                        IrisComposerBar(
                            draft: $draft,
                            attachments: $selectedAttachments,
                            placeholder: "Message",
                            isSending: manager.state.busy.sendingMessage,
                            isUploading: manager.state.busy.uploadingAttachment,
                            isFocused: $isComposerFocused,
                            onDraftChange: {
                                sendTypingIfNeeded()
                            },
                            onAttach: { urls in
                                do {
                                    selectedAttachments.append(
                                        contentsOf: try manager.stageOutgoingAttachments(urls)
                                    )
                                } catch {
                                    manager.showAttachmentOpenError()
                                }
                            }
                        ) {
                            let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !text.isEmpty || !selectedAttachments.isEmpty else { return }
                            stopTypingIfNeeded()
                            shouldFollowLatest = true
                            forceScrollToLatest = true
                            let outgoingText = replyEncodedMessage(reply: replyTarget, text: text)
                            replyTarget = nil
                            if selectedAttachments.isEmpty {
                                draft = ""
                                manager.dispatch(.sendMessage(chatId: chatId, text: outgoingText))
                            } else {
                                let attachments = selectedAttachments
                                selectedAttachments = []
                                draft = ""
                                manager.sendAttachments(chatId: chatId, attachments: attachments, caption: outgoingText)
                            }
                        }
                    }
                } else {
                    VStack(spacing: 0) {
                        Spacer()
                        IrisSectionCard {
                            Text("Loading chat…")
                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                .foregroundStyle(palette.textPrimary)
                        }
                        .padding(.horizontal, 16)
                        Spacer()
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay {
            if let imageViewerItem {
                IrisImageViewer(item: imageViewerItem) {
                    self.imageViewerItem = nil
                }
            }
        }
        .onDisappear {
            stopTypingIfNeeded()
        }
        .task(id: seenReceiptToken(for: chat)) {
            guard let chat else { return }
            let incomingIds = chat.messages
                .filter { !$0.isOutgoing && $0.delivery != .seen }
                .map(\.id)
            guard !incomingIds.isEmpty else { return }
            manager.dispatch(.markMessagesSeen(chatId: chat.chatId, messageIds: incomingIds))
        }
    }

    private func sendTypingIfNeeded() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            stopTypingIfNeeded()
            return
        }
        let now = Date()
        if let lastTypingSentAt, now.timeIntervalSince(lastTypingSentAt) < 3 {
            return
        }
        lastTypingSentAt = now
        sentTypingIndicator = true
        manager.dispatch(.sendTyping(chatId: chatId))
    }

    private func stopTypingIfNeeded() {
        guard sentTypingIndicator else { return }
        sentTypingIndicator = false
        lastTypingSentAt = nil
        manager.dispatch(.stopTyping(chatId: chatId))
    }

    private func seenReceiptToken(for chat: CurrentChatSnapshot?) -> String {
        guard let chat else { return "" }
        return chat.messages
            .filter { !$0.isOutgoing && $0.delivery != .seen }
            .map(\.id)
            .joined(separator: ",")
    }

    private func scrollToBottom(proxy: ScrollViewProxy, animated: Bool) {
        DispatchQueue.main.async {
            if animated {
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo(ChatTimelineAnchor.bottom, anchor: .bottom)
                }
            } else {
                proxy.scrollTo(ChatTimelineAnchor.bottom, anchor: .bottom)
            }
        }
    }

}

private let irisMessageClusterGapSecs: UInt64 = 60

private func irisStartsMessageCluster(
    previous: ChatMessageSnapshot?,
    message: ChatMessageSnapshot,
    chatKind: ChatKind
) -> Bool {
    guard let previous else {
        return true
    }
    if !irisSameTimelineDay(previous.createdAtSecs, message.createdAtSecs) {
        return true
    }
    if previous.isOutgoing != message.isOutgoing {
        return true
    }
    if chatKind == .group && !message.isOutgoing && previous.author != message.author {
        return true
    }
    let gap = message.createdAtSecs >= previous.createdAtSecs
        ? message.createdAtSecs - previous.createdAtSecs
        : 0
    if gap <= irisMessageClusterGapSecs {
        return false
    }
    if chatKind == .direct {
        let previousMinute = previous.createdAtSecs / 60
        let messageMinute = message.createdAtSecs / 60
        if messageMinute >= previousMinute && messageMinute - previousMinute <= 1 {
            return false
        }
    }
    return true
}

private enum ChatTimelineCoordinateSpace {
    static let name = "chatTimelineCoordinateSpace"
}

private enum ChatTimelineAnchor {
    static let bottom = "chatTimelineBottom"
}

private struct ChatTimelineViewportMaxYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private struct ChatTimelineBottomMaxYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = .greatestFiniteMagnitude

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private func chatTimelineIsNearBottom(viewportMaxY: CGFloat, bottomMaxY: CGFloat) -> Bool {
    guard viewportMaxY > 0, bottomMaxY.isFinite else {
        return true
    }
    return bottomMaxY <= viewportMaxY + 24
}

private struct ChatMessageRow: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let chatKind: ChatKind
    let showDayChip: Bool
    let isFirstInCluster: Bool
    let isLastInCluster: Bool
    let reactions: [MessageReactionSnapshot]
    let onReply: () -> Void
    let onReact: (String) -> Void
    let onDelete: () -> Void
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var isHovering = false
    @State private var isMobileActionDockOpen = false

    private var bodyParts: ReplyParsedMessage {
        parseReplyEncodedMessage(message.body)
    }

    private var showActionDock: Bool {
        IrisLayout.usesDesktopChrome ? isHovering : isMobileActionDockOpen
    }

    private var bubbleShape: ChatMessageBubbleShape {
        ChatMessageBubbleShape(
            isOutgoing: message.isOutgoing,
            isFirstInCluster: isFirstInCluster,
            isLastInCluster: isLastInCluster
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            if showDayChip {
                HStack {
                    Spacer()
                    IrisDayChip(text: irisTimelineDay(message.createdAtSecs))
                    Spacer()
                }
                .padding(.vertical, 14)
            }

            if message.kind == .system {
                HStack {
                    Spacer(minLength: 24)
                    Text(message.body)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 7)
                        .background(Capsule(style: .continuous).fill(palette.panel.opacity(0.74)))
                    Spacer(minLength: 24)
                }
                .padding(.vertical, 8)
                .accessibilityIdentifier("chatSystemMessage-\(message.id)")
            } else {
            VStack(
                alignment: message.isOutgoing ? .trailing : .leading,
                spacing: 6
            ) {
                if chatKind == .group && !message.isOutgoing && isFirstInCluster {
                    Text(message.author)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }

                HStack(alignment: .center, spacing: 6) {
                    if showActionDock && message.isOutgoing {
                        ChatMessageActionDock(
                            onReact: onReact,
                            onReply: onReply,
                            onCopyInfo: {
                                PlatformClipboard.setString("Message \(message.id) · \(irisMessageClock(message.createdAtSecs))")
                            },
                            onDelete: onDelete
                        )
                    }

                    VStack(alignment: .trailing, spacing: 8) {
                        if let reply = bodyParts.reply {
                            ReplyPreviewView(reply: reply, isOutgoing: message.isOutgoing)
                        }
                        if !bodyParts.body.isEmpty {
                            Text(
                                linkedMessageAttributedString(
                                    bodyParts.body,
                                    linkColor: message.isOutgoing ? palette.onBubbleMine : palette.accentAlt
                                )
                            )
                            .font(.system(.body, design: .rounded))
                            .multilineTextAlignment(message.isOutgoing ? .trailing : .leading)
                        }
                        ForEach(Array(message.attachments.enumerated()), id: \.offset) { _, attachment in
                            ChatAttachmentView(
                                attachment: attachment,
                                isOutgoing: message.isOutgoing,
                                downloadAttachment: downloadAttachment,
                                openAttachment: openAttachment,
                                onOpenImage: onOpenImage
                            )
                        }
                        if isLastInCluster {
                            HStack(spacing: 6) {
                                if message.expiresAtSecs != nil {
                                    Image(systemName: "timer")
                                        .font(.system(.caption2, design: .rounded, weight: .semibold))
                                        .accessibilityLabel("Disappearing message")
                                        .accessibilityIdentifier("chatMessageDisappearing-\(message.id)")
                                }
                                Text(irisMessageClock(message.createdAtSecs))
                                    .font(.system(.caption2, design: .rounded, weight: .medium))
                                if message.isOutgoing {
                                    IrisDeliveryGlyph(delivery: message.delivery)
                                }
                            }
                            .foregroundStyle(
                                (message.isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs)
                                    .opacity(0.72)
                            )
                        }
                    }
                    .foregroundStyle(message.isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 11)
                    .background(
                        bubbleShape
                            .fill(message.isOutgoing ? palette.bubbleMine : palette.bubbleTheirs)
                    )
                    .clipShape(bubbleShape)
                    .contentShape(bubbleShape)
                    .onTapGesture {
                        if !IrisLayout.usesDesktopChrome {
                            isMobileActionDockOpen.toggle()
                        }
                    }
                    .contextMenu {
                        Button("Reply", action: onReply)
                        Button("React 👍") { onReact("👍") }
                        Button("React ❤️") { onReact("❤️") }
                        Button("Copy") {
                            PlatformClipboard.setString(copyableMessageText(message))
                        }
                        Button("Delete locally", role: .destructive, action: onDelete)
                    }
                    .accessibilityIdentifier("chatMessage-\(message.id)")

                    if showActionDock && !message.isOutgoing {
                        ChatMessageActionDock(
                            onReact: onReact,
                            onReply: onReply,
                            onCopyInfo: {
                                PlatformClipboard.setString("Message \(message.id) · \(irisMessageClock(message.createdAtSecs))")
                            },
                            onDelete: onDelete
                        )
                    }
                }

                if !reactions.isEmpty {
                    ReactionRow(reactions: reactions, isOutgoing: message.isOutgoing)
                }
            }
            .frame(maxWidth: .infinity, alignment: message.isOutgoing ? .trailing : .leading)
            .contentShape(Rectangle())
            .onHover { isHovering = $0 }
            .padding(.top, isFirstInCluster ? 10 : 4)
            .padding(.bottom, isLastInCluster ? 10 : 0)
            }
        }
    }
}

private struct ChatMessageBubbleShape: Shape {
    let isOutgoing: Bool
    let isFirstInCluster: Bool
    let isLastInCluster: Bool

    func path(in rect: CGRect) -> Path {
        let large: CGFloat = 22
        let tail: CGFloat = 6
        let radii: (topLeft: CGFloat, topRight: CGFloat, bottomRight: CGFloat, bottomLeft: CGFloat)

        if isFirstInCluster && isLastInCluster {
            radii = (large, large, large, large)
        } else if isOutgoing && isFirstInCluster {
            radii = (large, large, tail, large)
        } else if isOutgoing && isLastInCluster {
            radii = (large, tail, large, large)
        } else if isOutgoing {
            radii = (large, tail, tail, large)
        } else if isFirstInCluster {
            radii = (large, large, large, tail)
        } else if isLastInCluster {
            radii = (tail, large, large, large)
        } else {
            radii = (tail, large, large, tail)
        }

        let maxRadius = min(rect.width, rect.height) / 2
        let topLeft = min(radii.topLeft, maxRadius)
        let topRight = min(radii.topRight, maxRadius)
        let bottomRight = min(radii.bottomRight, maxRadius)
        let bottomLeft = min(radii.bottomLeft, maxRadius)

        var path = Path()
        path.move(to: CGPoint(x: rect.minX + topLeft, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX - topRight, y: rect.minY))
        path.addQuadCurve(
            to: CGPoint(x: rect.maxX, y: rect.minY + topRight),
            control: CGPoint(x: rect.maxX, y: rect.minY)
        )
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY - bottomRight))
        path.addQuadCurve(
            to: CGPoint(x: rect.maxX - bottomRight, y: rect.maxY),
            control: CGPoint(x: rect.maxX, y: rect.maxY)
        )
        path.addLine(to: CGPoint(x: rect.minX + bottomLeft, y: rect.maxY))
        path.addQuadCurve(
            to: CGPoint(x: rect.minX, y: rect.maxY - bottomLeft),
            control: CGPoint(x: rect.minX, y: rect.maxY)
        )
        path.addLine(to: CGPoint(x: rect.minX, y: rect.minY + topLeft))
        path.addQuadCurve(
            to: CGPoint(x: rect.minX + topLeft, y: rect.minY),
            control: CGPoint(x: rect.minX, y: rect.minY)
        )
        path.closeSubpath()
        return path
    }
}

private struct IrisDeliveryGlyph: View {
    @Environment(\.irisPalette) private var palette
    let delivery: DeliveryState

    var body: some View {
        glyph
            .font(.system(size: 11, weight: .bold))
            .foregroundStyle(tint)
            .accessibilityLabel(irisDeliveryLabel(delivery))
    }

    @ViewBuilder
    private var glyph: some View {
        switch delivery {
        case .queued, .pending:
            Image(systemName: "clock")
        case .sent:
            Image(systemName: "checkmark")
        case .received, .seen:
            HStack(spacing: 1) {
                Image(systemName: "checkmark")
                Image(systemName: "checkmark")
            }
        case .failed:
            Image(systemName: "exclamationmark.circle.fill")
        }
    }

    private var tint: Color {
        switch delivery {
        case .received:
            return palette.accentAlt
        case .seen:
            return Color(.sRGB, red: 0.055, green: 0.647, blue: 0.914, opacity: 1)
        case .failed:
            return .red
        default:
            return palette.muted
        }
    }
}

private struct ChatMessageActionDock: View {
    @Environment(\.irisPalette) private var palette
    let onReact: (String) -> Void
    let onReply: () -> Void
    let onCopyInfo: () -> Void
    let onDelete: () -> Void

    var body: some View {
        HStack(spacing: 2) {
            Menu {
                ForEach(["👍", "❤️", "😂", "😮", "😢", "🙏"], id: \.self) { emoji in
                    Button(emoji) { onReact(emoji) }
                }
            } label: {
                Image(systemName: "face.smiling.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
            dockButton("arrowshape.turn.up.left.fill", action: onReply)
            Menu {
                Button("Message info", action: onCopyInfo)
                Button("Delete message", role: .destructive, action: onDelete)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .bold))
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
        }
        .padding(5)
        .background(
            Capsule(style: .continuous)
                .fill(palette.toolbar.opacity(0.96))
        )
    }

    private func dockButton(_ systemName: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: 26, height: 24)
        }
        .buttonStyle(.plain)
    }
}

private struct IrisTypingIndicatorRow: View {
    @Environment(\.irisPalette) private var palette
    let indicators: [TypingIndicatorSnapshot]

    private var label: String {
        guard let first = indicators.first else { return "" }
        if indicators.count == 1 {
            return "\(first.displayName) is typing"
        }
        return "\(first.displayName) and \(indicators.count - 1) more are typing"
    }

    var body: some View {
        HStack(spacing: 8) {
            HStack(spacing: 4) {
                Circle().frame(width: 5, height: 5)
                Circle().frame(width: 5, height: 5)
                Circle().frame(width: 5, height: 5)
            }
            .foregroundStyle(palette.muted)

            Text(label)
                .font(.system(.caption, design: .rounded, weight: .medium))
                .foregroundStyle(palette.muted)
                .lineLimit(1)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Capsule(style: .continuous).fill(palette.toolbar.opacity(0.9)))
        .frame(maxWidth: 260, alignment: .leading)
        .accessibilityIdentifier("chatTypingIndicator")
    }
}

private struct ReactionRow: View {
    @Environment(\.irisPalette) private var palette
    let reactions: [MessageReactionSnapshot]
    let isOutgoing: Bool

    var body: some View {
        HStack(spacing: 5) {
            ForEach(reactions, id: \.emoji) { reaction in
                Text("\(reaction.emoji) \(reaction.count)")
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 4)
                    .background(
                        Capsule(style: .continuous)
                            .fill(reaction.reactedByMe ? palette.accent.opacity(0.18) : palette.panel)
                    )
            }
        }
        .frame(maxWidth: .infinity, alignment: isOutgoing ? .trailing : .leading)
    }
}

private struct IrisReplyComposerStrip: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Rectangle()
                .fill(palette.accent)
                .frame(width: 3)
                .clipShape(Capsule())
            VStack(alignment: .leading, spacing: 2) {
                Text(message.author)
                    .font(.system(.caption, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
                Text(replySnippet(for: message))
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
            }
            Spacer()
            Button(action: onCancel) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.muted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 18 : 16)
        .padding(.vertical, 8)
        .background(palette.toolbar)
        .accessibilityIdentifier("chatReplyComposer")
    }
}

private struct ReplyPreviewView: View {
    @Environment(\.irisPalette) private var palette
    let reply: ReplyPreview
    let isOutgoing: Bool

    var body: some View {
        HStack(spacing: 8) {
            Rectangle()
                .fill(isOutgoing ? palette.onBubbleMine.opacity(0.6) : palette.accent)
                .frame(width: 3)
                .clipShape(Capsule())
            VStack(alignment: .leading, spacing: 2) {
                Text(reply.author)
                    .font(.system(.caption, design: .rounded, weight: .bold))
                Text(reply.body)
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .lineLimit(2)
                    .opacity(0.82)
            }
        }
        .frame(maxWidth: 280, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
        )
    }
}

private enum ChatAttachmentCategory: String {
    case image = "Image"
    case video = "Video"
    case audio = "Audio"
    case archive = "Archive"
    case document = "Document"
    case file = "File"

    var systemIcon: String {
        switch self {
        case .image:
            return "photo.fill"
        case .video:
            return "play.rectangle.fill"
        case .audio:
            return "waveform"
        case .archive:
            return "archivebox.fill"
        case .document:
            return "doc.text.fill"
        case .file:
            return "doc.fill"
        }
    }
}

private let chatImageExtensions: Set<String> = ["gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif"]
private let chatVideoExtensions: Set<String> = ["avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts"]
private let chatAudioExtensions: Set<String> = ["aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"]
private let chatArchiveExtensions: Set<String> = ["7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip"]
private let chatDocumentExtensions: Set<String> = ["csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml"]

private func chatAttachmentCategory(from filename: String) -> ChatAttachmentCategory {
    let ext = filename
        .split(separator: ".")
        .last
        .map { String($0).lowercased() }

    guard let extensionValue = ext, !extensionValue.isEmpty else {
        return .file
    }

    if chatImageExtensions.contains(extensionValue) {
        return .image
    }
    if chatVideoExtensions.contains(extensionValue) {
        return .video
    }
    if chatAudioExtensions.contains(extensionValue) {
        return .audio
    }
    if chatArchiveExtensions.contains(extensionValue) {
        return .archive
    }
    if chatDocumentExtensions.contains(extensionValue) {
        return .document
    }
    return .file
}

private func chatAttachmentCategory(for attachment: MessageAttachmentSnapshot) -> ChatAttachmentCategory {
    if attachment.isImage {
        return .image
    }
    if attachment.isVideo {
        return .video
    }
    if attachment.isAudio {
        return .audio
    }
    return chatAttachmentCategory(from: attachment.filename)
}

private struct ChatAttachmentView: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var localImageData: Data?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false
    @State private var isOpeningAttachment = false

    private var localImage: PlatformImage? {
        guard let localImageData, !isAnimatedImage(data: localImageData, filename: attachment.filename) else {
            return nil
        }
        return PlatformImage(data: localImageData)
    }

    var body: some View {
        if attachment.isImage {
            Button {
                if let localImageData {
                    onOpenImage(localImageData, attachment.filename)
                } else {
                    Task {
                        await loadImageIfNeeded()
                        if let localImageData {
                            onOpenImage(localImageData, attachment.filename)
                        }
                    }
                }
            } label: {
                VStack(alignment: .leading, spacing: 7) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 16, style: .continuous)
                            .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                            .frame(width: 220, height: 150)
                        if let localImage {
                            Image(platformImage: localImage)
                                .resizable()
                                .scaledToFill()
                                .frame(width: 220, height: 150)
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                        } else if let localImageData, isAnimatedImage(data: localImageData, filename: attachment.filename) {
                            IrisAnimatedImageDataView(data: localImageData)
                                .frame(width: 220, height: 150)
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                                .allowsHitTesting(false)
                        } else if isLoadingImage {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Image(systemName: failedImageLoad ? "exclamationmark.triangle.fill" : "photo.fill")
                                .font(.system(size: 28, weight: .semibold))
                                .opacity(0.72)
                        }
                    }
                    Text(attachment.filename)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .lineLimit(1)
                        .frame(maxWidth: 220, alignment: .leading)
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel(attachment.filename)
            .task(id: attachment.htreeUrl) {
                await loadImageIfNeeded()
            }
        } else {
            let category = chatAttachmentCategory(for: attachment)

            Button {
                Task {
                    guard !isOpeningAttachment else { return }
                    isOpeningAttachment = true
                    await openAttachment(attachment)
                    isOpeningAttachment = false
                }
            } label: {
                HStack(spacing: 8) {
                    if isOpeningAttachment {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 20, height: 20)
                    } else {
                        Image(systemName: category.systemIcon)
                            .font(.system(size: 15, weight: .semibold))
                            .frame(width: 20, height: 20)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(attachment.filename)
                            .font(.system(.subheadline, design: .rounded, weight: .semibold))
                            .lineLimit(1)
                        Text(category.rawValue)
                            .font(.system(.caption, design: .rounded, weight: .medium))
                            .foregroundStyle(isOutgoing ? palette.onBubbleMine.opacity(0.6) : palette.onBubbleTheirs.opacity(0.6))
                            .lineLimit(1)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                )
            }
            .buttonStyle(.plain)
            .disabled(isOpeningAttachment)
            .contextMenu {
                Button("Copy link") {
                    PlatformClipboard.setString(attachment.htreeUrl)
                }
            }
            .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
        }
    }

    @MainActor
    private func loadImageIfNeeded() async {
        guard localImageData == nil, !isLoadingImage else {
            return
        }
        isLoadingImage = true
        failedImageLoad = false
        guard let data = await downloadAttachment(attachment) else {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        if !isAnimatedImage(data: data, filename: attachment.filename), PlatformImage(data: data) == nil {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        localImageData = data
        isLoadingImage = false
    }

}

private struct ImageViewerItem: Identifiable, Equatable {
    let id = UUID()
    let data: Data
    let filename: String

    var isAnimated: Bool {
        isAnimatedImage(data: data, filename: filename)
    }

    var image: PlatformImage? {
        guard !isAnimated else {
            return nil
        }
        return PlatformImage(data: data)
    }

    static func == (lhs: ImageViewerItem, rhs: ImageViewerItem) -> Bool {
        lhs.id == rhs.id
    }
}

private struct IrisImageViewer: View {
    let item: ImageViewerItem
    let onClose: () -> Void

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.opacity(0.92)
                .ignoresSafeArea()
                .onTapGesture(perform: onClose)
            if item.isAnimated {
                IrisAnimatedImageDataView(data: item.data)
                    .padding(22)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .allowsHitTesting(false)
            } else if let image = item.image {
                Image(platformImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(22)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .tint(.white)
            }

            Button(action: onClose) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 30, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.9))
                    .padding(18)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close image")
        }
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .zIndex(10)
    }
}

private func isAnimatedImage(data: Data, filename: String) -> Bool {
    if filename.lowercased().hasSuffix(".gif") {
        return true
    }
    let gifHeader = [UInt8](data.prefix(6))
    return gifHeader == Array("GIF87a".utf8) || gifHeader == Array("GIF89a".utf8)
}

private struct ReplyPreview {
    let author: String
    let body: String
}

private struct ReplyParsedMessage {
    let reply: ReplyPreview?
    let body: String
}

private func replyEncodedMessage(reply: ChatMessageSnapshot?, text: String) -> String {
    guard let reply else {
        return text
    }
    let snippet = replySnippet(for: reply)
    return "\(replyMessagePrefix)\(reply.author): \(snippet)\n\n\(text)"
}

private func parseReplyEncodedMessage(_ text: String) -> ReplyParsedMessage {
    guard text.hasPrefix(replyMessagePrefix) else {
        return ReplyParsedMessage(reply: nil, body: text)
    }
    let remaining = text.dropFirst(replyMessagePrefix.count)
    guard let separator = remaining.range(of: "\n\n") else {
        return ReplyParsedMessage(reply: nil, body: text)
    }
    let header = String(remaining[..<separator.lowerBound])
    let body = String(remaining[separator.upperBound...])
    let pieces = header.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
    guard pieces.count == 2 else {
        return ReplyParsedMessage(reply: nil, body: text)
    }
    return ReplyParsedMessage(
        reply: ReplyPreview(
            author: String(pieces[0]).trimmingCharacters(in: .whitespacesAndNewlines),
            body: String(pieces[1]).trimmingCharacters(in: .whitespacesAndNewlines)
        ),
        body: body
    )
}

private func replySnippet(for message: ChatMessageSnapshot) -> String {
    let parsed = parseReplyEncodedMessage(message.body)
    let source = parsed.body.isEmpty ? copyableMessageText(message) : parsed.body
    let normalized = source
        .replacingOccurrences(of: "\n", with: " ")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if normalized.isEmpty {
        return message.attachments.first?.filename ?? "Attachment"
    }
    return String(normalized.prefix(96))
}

private let replyMessagePrefix = "↩ "

private func linkedMessageAttributedString(_ text: String, linkColor: Color) -> AttributedString {
    var attributed = AttributedString()
    var cursor = text.startIndex
    for match in messageURLMatches(in: text) {
        if cursor < match.range.lowerBound {
            attributed.append(AttributedString(String(text[cursor..<match.range.lowerBound])))
        }
        var linked = AttributedString(String(text[match.range]))
        linked.link = match.url
        linked.foregroundColor = linkColor
        linked.underlineStyle = .single
        attributed.append(linked)
        cursor = match.range.upperBound
    }
    if cursor < text.endIndex {
        attributed.append(AttributedString(String(text[cursor...])))
    }
    return attributed
}

private func messageURLMatches(in text: String) -> [(range: Range<String.Index>, url: URL)] {
    var matches: [(Range<String.Index>, URL)] = []
    let pattern = #"\b((https?://|www\.)[^\s<]+)"#
    guard let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) else {
        return matches
    }
    let nsRange = NSRange(text.startIndex..<text.endIndex, in: text)
    regex.enumerateMatches(in: text, range: nsRange) { result, _, _ in
        guard
            let result,
            let range = Range(result.range(at: 1), in: text)
        else {
            return
        }
        let visible = String(text[range]).trimmingCharacters(in: messageURLTrailingPunctuation)
        guard !visible.isEmpty else {
            return
        }
        let end = text.index(range.lowerBound, offsetBy: visible.count)
        let lowercase = visible.lowercased()
        let normalized = lowercase.hasPrefix("http://") || lowercase.hasPrefix("https://")
            ? visible
            : "https://\(visible)"
        guard let url = URL(string: normalized) else {
            return
        }
        matches.append((range.lowerBound..<end, url))
    }
    return matches
}

private let messageURLTrailingPunctuation = CharacterSet(charactersIn: ".,;:!?)]")

private func copyableMessageText(_ message: ChatMessageSnapshot) -> String {
    var pieces: [String] = []
    if !message.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        pieces.append(message.body)
    }
    pieces.append(contentsOf: message.attachments.map(\.htreeUrl))
    return pieces.joined(separator: "\n")
}
