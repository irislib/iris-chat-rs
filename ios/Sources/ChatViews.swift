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
    @State private var messageInfoSelection: MessageInfoSelection?
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
                                GeometryReader { viewport in
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
                                                    onInfo: {
                                                        messageInfoSelection = MessageInfoSelection(
                                                            chatId: chat.chatId,
                                                            messageId: message.id,
                                                            snapshot: message
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
                                        .frame(minHeight: viewport.size.height, alignment: .bottom)
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
                                }
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
                                .task(id: chatId) {
                                    if IrisLayout.usesDesktopChrome {
                                        isComposerFocused = true
                                    }
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
        .sheet(item: $messageInfoSelection) { selection in
            let context = messageInfoContext(for: selection)
            MessageInfoSheet(message: context.message, chat: context.chat) {
                messageInfoSelection = nil
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .irisDismissOnMacOutsideClick {
                messageInfoSelection = nil
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

    private func messageInfoContext(for selection: MessageInfoSelection) -> (message: ChatMessageSnapshot, chat: CurrentChatSnapshot?) {
        let currentChat = manager.state.currentChat?.chatId == selection.chatId ? manager.state.currentChat : nil
        let message = currentChat?.messages.first { $0.id == selection.messageId } ?? selection.snapshot
        return (message, currentChat)
    }

}

private struct MessageInfoSelection: Identifiable {
    let chatId: String
    let messageId: String
    let snapshot: ChatMessageSnapshot

    var id: String {
        "\(chatId):\(messageId)"
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
    let onInfo: () -> Void
    let onDelete: () -> Void
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var isHovering = false
    @State private var showReactionPicker = false
    @State private var showActionsSheet = false

    private var bodyParts: ReplyParsedMessage {
        parseReplyEncodedMessage(message.body)
    }

    private var showActionDock: Bool {
        IrisLayout.usesDesktopChrome && isHovering
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
                            onShowReactionPicker: { showReactionPicker = true },
                            onReply: onReply,
                            onInfo: onInfo,
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
                    .onLongPressGesture(minimumDuration: 0.4) {
                        if !IrisLayout.usesDesktopChrome {
                            showActionsSheet = true
                        }
                    }
                    .sheet(isPresented: $showActionsSheet) {
                        ChatMessageActionsSheet(
                            message: message,
                            bodyText: bodyParts.body,
                            onReact: { emoji in
                                showActionsSheet = false
                                onReact(emoji)
                            },
                            onShowFullReactionPicker: {
                                showActionsSheet = false
                                showReactionPicker = true
                            },
                            onReply: {
                                showActionsSheet = false
                                onReply()
                            },
                            onCopy: {
                                showActionsSheet = false
                                PlatformClipboard.setString(copyableMessageText(message))
                            },
                            onInfo: {
                                showActionsSheet = false
                                onInfo()
                            },
                            onDelete: {
                                showActionsSheet = false
                                onDelete()
                            }
                        )
                        .presentationDetents([.medium])
                        .presentationDragIndicator(.visible)
                    }
                    .sheet(isPresented: $showReactionPicker) {
                        IrisEmojiPicker(
                            onClose: { showReactionPicker = false }
                        ) { emoji in
                            showReactionPicker = false
                            onReact(emoji)
                        }
                        .presentationDetents([.medium, .large])
                        .presentationDragIndicator(.visible)
                        .irisDismissOnMacOutsideClick { showReactionPicker = false }
                    }
                    .accessibilityIdentifier("chatMessage-\(message.id)")

                    if showActionDock && !message.isOutgoing {
                        ChatMessageActionDock(
                            onShowReactionPicker: { showReactionPicker = true },
                            onReply: onReply,
                            onInfo: onInfo,
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

private struct MessageInfoSheet: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let chat: CurrentChatSnapshot?
    let onClose: () -> Void

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    header
                    statusSection
                    peopleSection
                    transportSection
                    idsSection
                    attachmentSection
                    reactionSection
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 16)
                .frame(maxWidth: IrisLayout.scrollMaxWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
            }
            .background(palette.background)
            .navigationTitle("Message info")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    IrisModalCloseButton(action: onClose)
                        .accessibilityIdentifier("messageInfoCloseButton")
                }
            }
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
        }
        .accessibilityIdentifier("messageInfoSheet")
    }

    private var header: some View {
        IrisSectionCard(accent: true) {
            HStack(alignment: .center, spacing: 12) {
                IrisDeliveryGlyph(delivery: message.delivery)
                    .frame(width: 26, height: 26)
                Text(irisDeliveryLabel(message.delivery))
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
                    .accessibilityIdentifier("messageInfoStatus")
                Spacer(minLength: 12)
                IrisCopyButton(
                    label: "Copy info",
                    value: messageInfoText(message, chat: chat),
                    compact: true
                )
            }
        }
    }

    private var statusSection: some View {
        MessageInfoSection(title: "Status") {
            MessageInfoValueRow(label: "Time", value: messageInfoDateTime(message.createdAtSecs))
            if let expiresAtSecs = message.expiresAtSecs {
                MessageInfoValueRow(label: "Deletes", value: messageInfoDateTime(expiresAtSecs))
            }
            MessageInfoValueRow(label: "Type", value: messageInfoKind(message))
        }
    }

    @ViewBuilder
    private var peopleSection: some View {
        MessageInfoSection(title: "People") {
            if message.isOutgoing {
                if message.recipientDeliveries.isEmpty {
                    MessageInfoValueRow(label: "Recipients", value: "No receipts")
                } else {
                    ForEach(message.recipientDeliveries, id: \.ownerPubkeyHex) { recipient in
                        MessageInfoRecipientRow(
                            title: messageInfoRecipientName(recipient.ownerPubkeyHex, chat: chat),
                            subtitle: messageInfoDateTime(recipient.updatedAtSecs),
                            delivery: recipient.delivery
                        )
                    }
                }
            } else {
                MessageInfoValueRow(label: "From", value: message.author)
                MessageInfoRecipientRow(
                    title: "You",
                    subtitle: messageInfoDateTime(message.createdAtSecs),
                    delivery: message.delivery
                )
            }
        }
    }

    @ViewBuilder
    private var transportSection: some View {
        let trace = message.deliveryTrace
        let channels = trace.transportChannels.map(prettyTransportChannel)
        let queuedDeviceNpubs = trace.queuedProtocolTargets.map(shortNpub)
        if !channels.isEmpty ||
            !queuedDeviceNpubs.isEmpty ||
            trace.lastTransportError?.isEmpty == false {
            MessageInfoSection(title: "Transport") {
                if !channels.isEmpty {
                    MessageInfoMultiValueRow(
                        label: message.isOutgoing ? "Sent over" : "Received over",
                        values: channels
                    )
                }
                if !queuedDeviceNpubs.isEmpty {
                    MessageInfoMultiValueRow(
                        label: "Queued devices",
                        values: queuedDeviceNpubs,
                        monospaced: true
                    )
                }
                if let error = trace.lastTransportError, !error.isEmpty {
                    MessageInfoValueRow(label: "Last error", value: error)
                }
            }
        }
    }

    @ViewBuilder
    private var idsSection: some View {
        let trace = message.deliveryTrace
        MessageInfoSection(title: "IDs") {
            MessageInfoValueRow(label: "Message", value: message.id, monospaced: true, copyValue: message.id)
            if let sourceEventId = message.sourceEventId, !sourceEventId.isEmpty {
                MessageInfoValueRow(
                    label: "Received event",
                    value: shortMessageIdentifier(sourceEventId),
                    monospaced: true,
                    copyValue: sourceEventId
                )
            }
            if !trace.outerEventIds.isEmpty {
                MessageInfoCopyListRow(label: "Network events", values: trace.outerEventIds)
            }
            if !trace.targetDeviceIds.isEmpty {
                MessageInfoCopyListRow(
                    label: "Target devices",
                    values: trace.targetDeviceIds.map { peerInputToNpub(input: $0) }
                )
            }
        }
    }

    @ViewBuilder
    private var attachmentSection: some View {
        if !message.attachments.isEmpty {
            MessageInfoSection(title: "Attachments") {
                ForEach(message.attachments, id: \.htreeUrl) { attachment in
                    MessageInfoValueRow(
                        label: attachment.filename.isEmpty ? "File" : attachment.filename,
                        value: attachment.htreeUrl,
                        monospaced: true,
                        copyValue: attachment.htreeUrl
                    )
                }
            }
        }
    }

    @ViewBuilder
    private var reactionSection: some View {
        if !message.reactions.isEmpty || !message.reactors.isEmpty {
            MessageInfoSection(title: "Reactions") {
                ForEach(message.reactions, id: \.emoji) { reaction in
                    MessageInfoValueRow(
                        label: reaction.emoji,
                        value: "\(reaction.count)"
                    )
                }
                ForEach(message.reactors, id: \.author) { reactor in
                    MessageInfoReactorRow(
                        name: messageInfoRecipientName(reactor.author, chat: chat),
                        emoji: reactor.emoji
                    )
                }
            }
        }
    }
}

private struct MessageInfoReactorRow: View {
    @Environment(\.irisPalette) private var palette
    let name: String
    let emoji: String

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            IrisAvatar(label: name, size: 28)
            Text(name)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .lineLimit(1)
            Spacer(minLength: 8)
            Text(emoji.isEmpty ? "Removed" : emoji)
                .font(emoji.isEmpty ? .system(.caption, design: .rounded, weight: .medium) : .system(size: 22))
                .foregroundStyle(emoji.isEmpty ? palette.muted : palette.textPrimary)
        }
        .padding(.vertical, 6)
    }
}

private struct MessageInfoSection<Content: View>: View {
    let title: String
    let content: () -> Content

    init(title: String, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.content = content
    }

    var body: some View {
        IrisSectionCard {
            Text(title)
                .font(.system(.headline, design: .rounded, weight: .bold))
            VStack(spacing: 0, content: content)
        }
    }
}

private struct MessageInfoValueRow: View {
    @Environment(\.irisPalette) private var palette
    let label: String
    let value: String
    var monospaced: Bool = false
    var copyValue: String?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Text(label)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.muted)
                .frame(width: 96, alignment: .leading)
            Text(value)
                .font(monospaced ? .system(.footnote, design: .monospaced, weight: .medium) : .system(.subheadline, design: .rounded))
                .foregroundStyle(palette.textPrimary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let copyValue {
                Button {
                    PlatformClipboard.setString(copyValue)
                } label: {
                    Image(systemName: "doc.on.doc")
                        .font(.system(size: 13, weight: .semibold))
                        .frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .foregroundStyle(palette.muted)
                .accessibilityLabel("Copy")
            }
        }
        .padding(.vertical, 8)
    }
}

private struct MessageInfoMultiValueRow: View {
    @Environment(\.irisPalette) private var palette
    let label: String
    let values: [String]
    var monospaced: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.muted)
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(values.enumerated()), id: \.offset) { _, value in
                    Text(value)
                        .font(monospaced ? .system(.footnote, design: .monospaced, weight: .medium) : .system(.subheadline, design: .rounded))
                        .foregroundStyle(palette.textPrimary)
                        .textSelection(.enabled)
                }
            }
        }
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct MessageInfoCopyListRow: View {
    let label: String
    let values: [String]

    var body: some View {
        ForEach(Array(values.enumerated()), id: \.offset) { index, value in
            MessageInfoValueRow(
                label: index == 0 ? label : "",
                value: shortMessageIdentifier(value),
                monospaced: true,
                copyValue: value
            )
        }
    }
}

private struct MessageInfoRecipientRow: View {
    @Environment(\.irisPalette) private var palette
    let title: String
    let subtitle: String
    let delivery: DeliveryState

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            IrisDeliveryGlyph(delivery: delivery)
                .frame(width: 22, height: 22)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                Text("\(irisDeliveryLabel(delivery)) - \(subtitle)")
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
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
        Group {
            if let tint {
                glyph.foregroundStyle(tint)
            } else {
                glyph
            }
        }
        .font(.system(size: 9, weight: .bold))
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
            HStack(spacing: -7) {
                Image(systemName: "checkmark")
                Image(systemName: "checkmark")
            }
        case .failed:
            Image(systemName: "exclamationmark.circle.fill")
        }
    }

    private var tint: Color? {
        switch delivery {
        case .seen:
            return Color(.sRGB, red: 0.055, green: 0.647, blue: 0.914, opacity: 1)
        case .failed:
            return .red
        default:
            return nil
        }
    }
}

private struct ChatMessageActionDock: View {
    @Environment(\.irisPalette) private var palette
    let onShowReactionPicker: () -> Void
    let onReply: () -> Void
    let onInfo: () -> Void
    let onDelete: () -> Void

    var body: some View {
        HStack(spacing: 2) {
            Button {
                // The picker is owned by the parent row so it survives the
                // hover-state collapse that tears this dock down when the
                // cursor leaves the message row.
                onShowReactionPicker()
            } label: {
                Image(systemName: "face.smiling")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("messageReactButton")
            dockButton("arrowshape.turn.up.left", identifier: "messageReplyButton", action: onReply)
            Menu {
                Button("Message info", action: onInfo)
                Button("Delete message", role: .destructive, action: onDelete)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .bold))
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("messageMoreButton")
        }
        .foregroundStyle(palette.muted)
        .padding(5)
        .background(
            Capsule(style: .continuous)
                .fill(palette.toolbar.opacity(0.96))
        )
    }

    private func dockButton(_ systemName: String, identifier: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: 26, height: 24)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(identifier)
    }
}

private let quickReactionEmojis: [String] = ["❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"]

private struct ChatMessageActionsSheet: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let bodyText: String
    let onReact: (String) -> Void
    let onShowFullReactionPicker: () -> Void
    let onReply: () -> Void
    let onCopy: () -> Void
    let onInfo: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            quickReactionRow
            previewCard
            VStack(spacing: 0) {
                actionRow(icon: "arrowshape.turn.up.left", label: "Reply", action: onReply)
                actionRow(icon: "doc.on.doc", label: "Copy", action: onCopy)
                actionRow(icon: "info.circle", label: "Message info", action: onInfo)
                actionRow(icon: "trash", label: "Delete locally", destructive: true, action: onDelete)
            }
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(palette.panel)
            )
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.top, 14)
        .padding(.bottom, 6)
        .background(palette.background)
        .accessibilityIdentifier("messageActionsSheet")
    }

    private var quickReactionRow: some View {
        HStack(spacing: 4) {
            ForEach(quickReactionEmojis, id: \.self) { emoji in
                Button {
                    onReact(emoji)
                } label: {
                    Text(emoji)
                        .font(.system(size: 26))
                        .frame(maxWidth: .infinity)
                        .frame(height: 40)
                }
                .buttonStyle(.plain)
            }
            Button(action: onShowFullReactionPicker) {
                Image(systemName: "plus.circle")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .frame(height: 40)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("messageReactButton")
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 6)
        .background(
            Capsule(style: .continuous)
                .fill(palette.panel)
        )
    }

    private var previewText: String {
        if !bodyText.isEmpty { return bodyText }
        if let first = message.attachments.first {
            return first.filename.isEmpty ? "Attachment" : first.filename
        }
        return ""
    }

    @ViewBuilder
    private var previewCard: some View {
        if !previewText.isEmpty || !message.attachments.isEmpty || !message.reactions.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text(message.author)
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                if !previewText.isEmpty {
                    Text(previewText)
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                }
                if !message.attachments.isEmpty,
                   previewText != message.attachments.first?.filename {
                    Text(message.attachments.count == 1 ? "1 attachment" : "\(message.attachments.count) attachments")
                        .font(.system(.caption2, design: .rounded, weight: .medium))
                        .foregroundStyle(palette.muted)
                }
                if !message.reactions.isEmpty {
                    ReactionRow(reactions: message.reactions, isOutgoing: false)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(palette.panel)
            )
        }
    }

    private func actionRow(
        icon: String,
        label: String,
        destructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: icon)
                    .font(.system(size: 18, weight: .semibold))
                    .frame(width: 22)
                Text(label)
                    .font(.system(.body, design: .rounded, weight: .medium))
                Spacer()
            }
            .foregroundStyle(destructive ? Color.red : palette.textPrimary)
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct IrisEmojiPicker: View {
    @Environment(\.irisPalette) private var palette
    let onPick: (String) -> Void
    let onClose: (() -> Void)?

    @State private var query: String = ""

    init(
        onClose: (() -> Void)? = nil,
        onPick: @escaping (String) -> Void
    ) {
        self.onClose = onClose
        self.onPick = onPick
    }

    private static let categories: [(String, String, [String])] = [
        ("Smileys", "face.smiling",
         ["😀","😃","😄","😁","😆","😅","😂","🤣","😊","🙂","🙃","😉","😍","🥰","😘","😎","🤩","🥳","😏","😌","😴","😪","🤤","😋","😜","🤪","😝","🤔","🤨","😐","😑","😶","🙄","😬","🤐","🤧","🤒","🤕","😇","🤠","🥺","😢","😭","😠","🤬","🤯","🥶","🥵","😱","😨","😰","😳","🤗"]),
        ("Hearts", "heart.fill",
         ["❤️","🧡","💛","💚","💙","💜","🖤","🤍","🤎","💖","💗","💓","💞","💕","💘","💝","💟","♥️","💔","❣️","❤️‍🔥","❤️‍🩹"]),
        ("Hands", "hand.thumbsup.fill",
         ["👍","👎","👌","✌️","🤞","🤟","🤘","🤙","👈","👉","👆","👇","☝️","✋","🤚","🖐","🖖","👋","🤝","🙏","👏","🙌","💪","🫶","🫰","🫵","🫱","🫲"]),
        ("Animals", "pawprint.fill",
         ["🐶","🐱","🐭","🐹","🐰","🦊","🐻","🐼","🐨","🐯","🦁","🐮","🐷","🐸","🐵","🙈","🙉","🙊","🐔","🐧","🐦","🦅","🦉","🦄","🐝","🦋","🐞","🐢","🐍","🦖","🐙","🦀","🐬","🐳","🦈"]),
        ("Food", "fork.knife",
         ["🍏","🍎","🍐","🍊","🍋","🍌","🍉","🍇","🍓","🫐","🍒","🍑","🥭","🍍","🥥","🥝","🍅","🥑","🥕","🌽","🍆","🥔","🍕","🍔","🍟","🌭","🍿","🥪","🌮","🌯","🍣","🍜","🍝","🍦","🍩","🍪","🎂","🍰","☕","🍵","🍺","🥂","🍷","🥃"]),
        ("Activities", "sportscourt",
         ["⚽","🏀","🏈","⚾","🥎","🎾","🏐","🏉","🎱","🪀","🏓","🏸","🥅","🏒","🏑","🥍","🏏","🪃","🥊","🥋","🎽","⛸","🥌","🛷","🪂","🏋️","🤸","🤺","🏇","⛷","🏂","🏌️","🏄","🚣","🏊","🤽","🚴","🚵","🎯","🎮","🎲","🎼","🎤","🎧","🎷","🎸","🥁"]),
        ("Travel", "airplane",
         ["🚗","🚕","🚙","🚌","🚎","🏎","🚓","🚑","🚒","🚐","🛻","🚚","🚛","🚜","🛵","🏍","🛺","🚲","🛴","🛹","🚂","✈️","🚀","🛸","🛶","⛵","🚢","🚁","🗺","🗽","🗼","🏰","🎡","🎢","🎠","🏖","🏝","🏔","🌋","🏕","🌄","🌅","🌌"]),
        ("Objects", "lightbulb.fill",
         ["📱","💻","⌨️","🖥","🖨","🖱","💾","💿","📷","📸","📹","🎥","📺","📻","📞","☎️","🔌","🔋","💡","🔦","🕯","🧯","🛢","💵","💰","💳","💎","⚖️","🔧","🔨","🛠","⛏","🪛","🪚","🔩","⚙️","🧱","⛓","🧲","🔫","💣","🧨"]),
        ("Symbols", "sparkles",
         ["✅","❎","✔️","❌","⭕","🚫","⚠️","🔱","☑️","💯","🔥","✨","🌟","⭐","🌈","☀️","🌙","⚡","☄️","💥","🌊","💧","💦","🎉","🎊","🎁","🎀","🎈","🪅","🎂","🍾","🥇","🥈","🥉","🏆","🎖","🏅","💤","💭","🗯","💬","🆗","🆕","🆒","🆓","🆙","🔝","♻️","✅","❤️","💔","☮️","✝️","☪️","🕉","☸️","✡️","☯️","☦️"]),
    ]

    private var filteredCategories: [(String, String, [String])] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return Self.categories }
        return Self.categories.compactMap { name, icon, list in
            let hits = list.filter { $0.contains(q) || name.localizedCaseInsensitiveContains(q) }
            return hits.isEmpty ? nil : (name, icon, hits)
        }
    }

    private let columns = [GridItem(.adaptive(minimum: 40), spacing: 4)]

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                HStack {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(palette.muted)
                    TextField("Search", text: $query)
                        .textFieldStyle(.plain)
                        .autocorrectionDisabled()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(palette.panel)
                )
                .frame(maxWidth: .infinity)

                if let onClose {
                    IrisModalCloseButton(action: onClose)
                        .accessibilityIdentifier("reactionPickerCloseButton")
                }
            }
            .padding(10)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12, pinnedViews: [.sectionHeaders]) {
                    ForEach(filteredCategories, id: \.0) { name, icon, list in
                        Section {
                            LazyVGrid(columns: columns, spacing: 4) {
                                ForEach(list, id: \.self) { emoji in
                                    Button {
                                        onPick(emoji)
                                    } label: {
                                        Text(emoji)
                                            .font(.system(size: 26))
                                            .frame(width: 36, height: 36)
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                            .padding(.horizontal, 10)
                        } header: {
                            HStack(spacing: 6) {
                                Image(systemName: icon)
                                    .font(.system(size: 11, weight: .semibold))
                                Text(name)
                                    .font(.system(.caption, weight: .semibold))
                            }
                            .foregroundStyle(palette.muted)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(palette.background)
                        }
                    }
                }
                .padding(.bottom, 10)
            }
        }
        .frame(minWidth: 280, idealWidth: 320, minHeight: 320, idealHeight: 420)
        .background(palette.background)
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
    @State private var sharedFileURL: URL?

    var body: some View {
        ZStack(alignment: .top) {
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

            HStack {
                if let sharedFileURL {
                    ShareLink(item: sharedFileURL) {
                        Image(systemName: "square.and.arrow.up")
                            .font(.system(size: 24, weight: .semibold))
                            .foregroundStyle(.white.opacity(0.9))
                            .padding(18)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Share image")
                }
                Spacer()
                IrisModalCloseButton(
                    accessibilityLabel: "Close image",
                    tone: .light,
                    iconSize: 30,
                    hitSize: 66,
                    action: onClose
                )
            }
        }
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .zIndex(10)
        .task(id: item.id) {
            sharedFileURL = writeTempImage(data: item.data, filename: item.filename)
        }
    }
}

private func writeTempImage(data: Data, filename: String) -> URL? {
    let safeName = filename.isEmpty ? "image" : filename
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString)
        .appendingPathComponent(safeName)
    do {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: .atomic)
        return url
    } catch {
        return nil
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

private func messageInfoText(_ message: ChatMessageSnapshot, chat: CurrentChatSnapshot? = nil) -> String {
    var lines: [String] = [
        "Message \(message.id)",
        "Time \(messageInfoDateTime(message.createdAtSecs))",
        "Type \(messageInfoKind(message))",
        "Status \(irisDeliveryLabel(message.delivery))",
    ]
    if let expiresAtSecs = message.expiresAtSecs {
        lines.append("Deletes \(messageInfoDateTime(expiresAtSecs))")
    }
    let trace = message.deliveryTrace
    let channels = trace.transportChannels.map(prettyTransportChannel)
    if !channels.isEmpty {
        lines.append("\(message.isOutgoing ? "Sent over" : "Received over") \(channels.joined(separator: ", "))")
    }
    if !message.recipientDeliveries.isEmpty {
        lines.append("Recipients")
        lines.append(contentsOf: message.recipientDeliveries.map { recipient in
            "- \(messageInfoRecipientName(recipient.ownerPubkeyHex, chat: chat)) \(irisDeliveryLabel(recipient.delivery)) \(messageInfoDateTime(recipient.updatedAtSecs))"
        })
    } else if !message.isOutgoing {
        lines.append("From \(message.author)")
        lines.append("You \(irisDeliveryLabel(message.delivery))")
    }
    if !trace.outerEventIds.isEmpty {
        lines.append("Network IDs \(shortMessageIdentifierList(trace.outerEventIds))")
    }
    if !trace.queuedProtocolTargets.isEmpty {
        lines.append("Queued devices \(trace.queuedProtocolTargets.map(shortNpub).joined(separator: ", "))")
    }
    if !trace.targetDeviceIds.isEmpty {
        lines.append("Devices \(trace.targetDeviceIds.map(shortNpub).joined(separator: ", "))")
    }
    if let lastError = trace.lastTransportError, !lastError.isEmpty {
        lines.append("Last send error \(lastError)")
    }
    if let sourceEventId = message.sourceEventId, !sourceEventId.isEmpty {
        lines.append("Received as \(shortMessageIdentifier(sourceEventId))")
    }
    if !message.attachments.isEmpty {
        lines.append("Attachments")
        lines.append(contentsOf: message.attachments.map { attachment in
            "- \((attachment.filename.isEmpty ? "File" : attachment.filename)) \(attachment.htreeUrl)"
        })
    }
    if !message.reactions.isEmpty {
        lines.append("Reactions")
        lines.append(contentsOf: message.reactions.map { reaction in
            "- \(reaction.emoji) \(reaction.count)"
        })
    }
    return lines.joined(separator: "\n")
}

private func messageInfoDirection(_ message: ChatMessageSnapshot) -> String {
    if message.kind == .system {
        return "System message"
    }
    return message.isOutgoing ? "Sent message" : "Received message"
}

private func messageInfoKind(_ message: ChatMessageSnapshot) -> String {
    switch message.kind {
    case .system:
        return "System"
    case .user:
        return message.isOutgoing ? "Sent" : "Received"
    }
}

private func messageInfoRecipientName(_ ownerPubkeyHex: String, chat: CurrentChatSnapshot?) -> String {
    if let chat, chat.kind == .direct && chat.chatId == ownerPubkeyHex {
        return chat.displayName
    }
    return shortNpub(ownerPubkeyHex)
}

private func shortNpub(_ pubkeyInput: String) -> String {
    let npub = peerInputToNpub(input: pubkeyInput)
    let value = npub.isEmpty ? pubkeyInput : npub
    return shortMessageIdentifier(value)
}

private func prettyTransportChannel(_ channel: String) -> String {
    let prefix = "message server: "
    if channel.hasPrefix(prefix) {
        return String(channel.dropFirst(prefix.count))
    }
    if channel == "message servers" {
        return "Message server"
    }
    return channel
}

private func messageInfoDateTime(_ secs: UInt64) -> String {
    messageInfoDateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(secs)))
}

private let messageInfoDateFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .medium
    formatter.timeStyle = .short
    return formatter
}()

private func shortMessageIdentifierList(_ values: [String]) -> String {
    values.map(shortMessageIdentifier).joined(separator: ", ")
}

private func shortMessageIdentifier(_ value: String) -> String {
    guard value.count > 16 else {
        return value
    }
    return "\(value.prefix(8))...\(value.suffix(8))"
}
