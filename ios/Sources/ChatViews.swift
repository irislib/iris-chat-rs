import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

struct ChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chatId: String

    @State private var draft = ""
    @State private var selectedAttachments: [StagedAttachment] = []
    @State private var isNearBottom = true
    @State private var shouldFollowLatest = true
    @State private var forceScrollToLatest = false
    @State private var pendingScrollSettle: DispatchWorkItem?
    @State private var timelineViewportMaxY: CGFloat = 0
    @State private var timelineBottomMaxY: CGFloat = .greatestFiniteMagnitude
    @State private var timelineContentHeight: CGFloat = 0
    @State private var initialScrollPending = true
    @State private var renderedMessageCount = 0
    @State private var replyTarget: ChatMessageSnapshot?
    @State private var imageViewerItem: ImageViewerItem?
    @State private var lastTypingSentAt: Date?
    @State private var sentTypingIndicator = false
    @State private var messageInfoSelection: MessageInfoSelection?
    @State private var reactorsSelection: MessageReactorsSelection?
    @State private var lastPersistedDraft: String?
    @State private var draftFlushWork: DispatchWorkItem?
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
                                        // Eager VStack instead of LazyVStack:
                                        // SwiftUI's `LazyVStack` only realises
                                        // & measures rows once they're scrolled
                                        // into view, so on a freshly opened
                                        // long chat `proxy.scrollTo(.bottom)`
                                        // landed mid-timeline (the trailing
                                        // anchor's resolved position was wrong
                                        // because the rows above hadn't been
                                        // measured yet). The chat already pages
                                        // to ≤ `OPEN_CHAT_MESSAGES_PER_PAGE`
                                        // (80) messages per open in the Rust
                                        // core, so eager layout is fine —
                                        // matches Signal-iOS, which also
                                        // pre-measures every visible cell
                                        // before the scroll lands.
                                        VStack(spacing: 0) {
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

                                                EquatableView(content: ChatMessageRow(
                                                    message: message,
                                                    chatKind: chat.kind,
                                                    showDayChip: showDayChip,
                                                    isFirstInCluster: isFirstInCluster,
                                                    isLastInCluster: isLastInCluster,
                                                    reactions: message.reactions,
                                                    onReply: {
                                                        replyTarget = message
                                                        isComposerFocused = true
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
                                                    onScrollToQuote: { reply in
                                                        scrollToQuotedMessage(
                                                            from: message,
                                                            reply: reply,
                                                            in: chat.messages,
                                                            proxy: proxy
                                                        )
                                                    },
                                                    onShowReactors: {
                                                        reactorsSelection = MessageReactorsSelection(messageId: message.id)
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
                                                ))
                                                .id(message.id)
                                            }
                                        }
                                        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 18 : 14)
                                        .padding(.vertical, 10)
                                        .background(
                                            // Publishes the timeline's
                                            // intrinsic content height —
                                            // this background sits on the
                                            // padded LazyVStack, so its
                                            // size reflects "how tall do
                                            // all the bubbles want to be"
                                            // and changes only when bubbles
                                            // are added/removed/resized,
                                            // not when the user scrolls.
                                            GeometryReader { geo in
                                                Color.clear.preference(
                                                    key: ChatTimelineContentHeightPreferenceKey.self,
                                                    value: geo.size.height
                                                )
                                            }
                                        )
                                        .frame(minHeight: viewport.size.height, alignment: .bottom)
                                        .accessibilityIdentifier("chatTimeline")

                                        // Trailing bottom anchor sits OUTSIDE
                                        // the LazyVStack so SwiftUI always
                                        // realises it. When it was a child of
                                        // the LazyVStack, SwiftUI would only
                                        // lay it out once it scrolled into
                                        // view — so on a freshly opened long
                                        // chat its frame.maxY stayed at the
                                        // default `.greatestFiniteMagnitude`,
                                        // `chatTimelineIsNearBottom` returned
                                        // false, and the jump-to-bottom button
                                        // flashed up even though we were at
                                        // the latest message.
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
                                    .irisDefaultScrollAnchorBottom()
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
                                    timelineContentHeight = 0
                                    lastTypingSentAt = nil
                                    sentTypingIndicator = false
                                }
                                .onPreferenceChange(ChatTimelineViewportMaxYPreferenceKey.self) { value in
                                    let nearBottom = chatTimelineIsNearBottom(
                                        viewportMaxY: value,
                                        bottomMaxY: timelineBottomMaxY
                                    )
                                    if !chatTimelineGeometryMatches(timelineViewportMaxY, value) {
                                        timelineViewportMaxY = value
                                    }
                                    updateTimelineFollowState(
                                        nearBottom: nearBottom,
                                        messageCount: chat.messages.count
                                    )
                                }
                                .onPreferenceChange(ChatTimelineBottomMaxYPreferenceKey.self) { value in
                                    let nearBottom = chatTimelineIsNearBottom(
                                        viewportMaxY: timelineViewportMaxY,
                                        bottomMaxY: value
                                    )
                                    if !chatTimelineGeometryMatches(timelineBottomMaxY, value) {
                                        timelineBottomMaxY = value
                                    }
                                    updateTimelineFollowState(
                                        nearBottom: nearBottom,
                                        messageCount: chat.messages.count
                                    )
                                }
                                .onPreferenceChange(ChatTimelineContentHeightPreferenceKey.self) { value in
                                    // Repin to the bottom when the timeline's
                                    // intrinsic content grew (reaction landed,
                                    // attachment finished loading, quote
                                    // preview rendered, etc.) and we were
                                    // already following. Crucially, this
                                    // preference does NOT change while the
                                    // user is scrolling — only when bubbles
                                    // actually resize — so scrolling up is
                                    // never misread as growth.
                                    let previous = timelineContentHeight
                                    if !chatTimelineGeometryMatches(timelineContentHeight, value) {
                                        timelineContentHeight = value
                                    }
                                    let grew = previous > 0 && value > previous + 1
                                    if !initialScrollPending, shouldFollowLatest, grew {
                                        scrollToBottom(proxy: proxy, animated: false)
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
                                    // Search hits ask us to land on a
                                    // specific bubble instead of the
                                    // bottom of the timeline. Consume
                                    // the manager-side flag here so a
                                    // tap on a "Messages" row scrolls
                                    // straight to that message; falls
                                    // through to the regular bottom
                                    // scroll for normal opens.
                                    if let targetId = manager.pendingScrollMessageId,
                                       chat.messages.contains(where: { $0.id == targetId }) {
                                        renderedMessageCount = messageCount
                                        initialScrollPending = false
                                        shouldFollowLatest = false
                                        forceScrollToLatest = false
                                        scrollToMessage(proxy: proxy, messageId: targetId)
                                        manager.consumePendingScrollMessage()
                                        return
                                    }
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
                                // The `forceScrollToLatest` flag used to drive a
                                // dedicated scroll task here, but it always fired
                                // *before* the optimistic message landed — adding
                                // a redundant animated scroll to the OLD bottom on
                                // top of the scrolls already coming from
                                // `.task(id: chat.messages.last?.id)` and the
                                // content-height preference. We now just leave
                                // the flag for the messages task to consume in
                                // `shouldScroll`, so each send fires exactly one
                                // animated scroll once the new bubble has been
                                // laid out.
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
                                        // Signal-style glass capsule:
                                        // translucent pane that adapts to
                                        // the bubbles below, the arrow
                                        // itself carries the accent
                                        // colour for visibility. The
                                        // outer 60×60 contentShape lets
                                        // off-center thumb taps still
                                        // land on the button instead of
                                        // slipping through to a bubble
                                        // underneath.
                                        Image(systemName: "chevron.down")
                                            .font(.system(size: 17, weight: .bold))
                                            .foregroundStyle(palette.textPrimary)
                                            .frame(width: 44, height: 44)
                                            .irisGlassSurface(in: Circle())
                                            .overlay(
                                                Circle()
                                                    .strokeBorder(
                                                        palette.border.opacity(0.42),
                                                        lineWidth: 0.5
                                                    )
                                            )
                                            .frame(width: 60, height: 60)
                                            .contentShape(Rectangle())
                                    }
                                    .padding(.trailing, 8)
                                    .padding(.bottom, 8)
                                    .buttonStyle(.irisPlain)
                                    .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
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
                            // Float the reply strip + composer over the
                            // chat timeline via .safeAreaInset so the
                            // bubbles actually scroll *under* the
                            // composer's glass surface — that's what
                            // makes the translucent material visible.
                            // Without this the composer was a separate
                            // band below the ScrollView, with no content
                            // behind it for the blur to reveal.
                            .safeAreaInset(edge: .bottom, spacing: 0) {
                                VStack(spacing: 0) {
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
        .modifier(EscDismissesReply(replyTarget: $replyTarget))
        .overlay {
            if let imageViewerItem {
                IrisImageViewer(item: imageViewerItem) {
                    self.imageViewerItem = nil
                }
            }
        }
        .sheet(item: $messageInfoSelection) { selection in
            let context = messageInfoContext(for: selection)
            MessageInfoSheet(message: context.message, chat: context.chat, manager: manager) {
                messageInfoSelection = nil
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .irisDismissOnMacOutsideClick {
                messageInfoSelection = nil
            }
        }
        .sheet(item: $reactorsSelection) { selection in
            let context = reactorsContext(for: selection)
            MessageReactorsSheet(reactors: context.reactors, chat: context.chat, manager: manager) {
                reactorsSelection = nil
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .irisDismissOnMacOutsideClick {
                reactorsSelection = nil
            }
        }
        .onDisappear {
            stopTypingIfNeeded()
            flushDraftImmediately()
        }
        .task(id: chatId) {
            // First arrival at this chat (or chatId rebind in a split
            // shell): seed the composer from the persisted thread
            // draft so unsent text survives navigation + relaunch.
            // We only paste it once per chat appearance; subsequent
            // user keystrokes own the buffer.
            let initial = manager.state.currentChat?.draft ?? ""
            if !initial.isEmpty {
                draft = initial
            }
            lastPersistedDraft = initial
        }
        .irisOnChange(of: draft) { newValue in
            scheduleDraftFlush(text: newValue)
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

    /// Centre the targeted bubble in the viewport for search-hit
    /// taps. Reuses the multi-tick re-scroll pattern from
    /// `scrollToBottom` so quoted-reply previews / images that
    /// resolve a moment after layout don't end up just off-screen.
    private func scrollToMessage(proxy: ScrollViewProxy, messageId: String) {
        let scroll = {
            withAnimation(.easeOut(duration: 0.25)) {
                proxy.scrollTo(messageId, anchor: .center)
            }
        }
        DispatchQueue.main.async { scroll() }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { scroll() }
    }

    /// Debounce composer writes so a fast typist generates one
    /// SQLite-row update every ~500ms instead of one per keystroke.
    /// On disappear / send we flush eagerly so the latest text always
    /// hits disk before the view goes away.
    private func scheduleDraftFlush(text: String) {
        draftFlushWork?.cancel()
        if lastPersistedDraft == text {
            return
        }
        let work = DispatchWorkItem {
            manager.dispatch(.setChatDraft(chatId: chatId, text: text))
            lastPersistedDraft = text
        }
        draftFlushWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }

    private func flushDraftImmediately() {
        draftFlushWork?.cancel()
        draftFlushWork = nil
        guard lastPersistedDraft != draft else { return }
        manager.dispatch(.setChatDraft(chatId: chatId, text: draft))
        lastPersistedDraft = draft
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
        // Prefer the last message's own id over the trailing 1pt
        // anchor: SwiftUI must realise & measure the targeted row, so
        // a scroll to the actual final bubble forces SwiftUI to lay
        // it out and lands the bottom of that bubble at the
        // viewport's bottom. Scrolling to the empty anchor view
        // doesn't have that effect — SwiftUI happily resolves it to
        // its current (wrong) position when sibling rows haven't
        // been measured yet.
        //
        // We previously queued four scrolls (immediate + async + 100ms
        // + 300ms) to catch images/quotes settling. With the chat
        // re-scrolling on every state push (send → queued, queued →
        // pending, pending → sent), those overlapping batches stacked
        // up to ~12 scrollTo calls per send, which iOS rendered as a
        // visible flicker. We now keep a single deferred follow-up
        // and cancel any earlier pending one — so a fresh send
        // collapses cleanly to one immediate scroll + one short
        // settle, with no leftover scrolls fighting the next state
        // push.
        let target = chat?.messages.last?.id ?? ChatTimelineAnchor.bottom
        let scroll = {
            proxy.scrollTo(target, anchor: .bottom)
        }
        if animated {
            withAnimation(.easeOut(duration: 0.2)) { scroll() }
        } else {
            scroll()
        }
        pendingScrollSettle?.cancel()
        let item = DispatchWorkItem { scroll() }
        pendingScrollSettle = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: item)
    }

    // The wire format for a quoted reply only carries author + a snippet
    // (max 96 chars, newlines flattened). To find the message the snippet
    // came from, walk backwards from the replying message and match the
    // same author whose own snippet matches. Disambiguates on the most
    // recent matching message, which is the natural reading order.
    private func scrollToQuotedMessage(
        from replyingMessage: ChatMessageSnapshot,
        reply: ReplyPreview,
        in messages: [ChatMessageSnapshot],
        proxy: ScrollViewProxy
    ) {
        guard let currentIdx = messages.firstIndex(where: { $0.id == replyingMessage.id }) else { return }
        let target = reply.body
        for i in stride(from: currentIdx - 1, through: 0, by: -1) {
            let candidate = messages[i]
            guard candidate.author == reply.author else { continue }
            let candidateSnippet = replySnippet(for: candidate)
            if candidateSnippet == target {
                withAnimation(.easeOut(duration: 0.25)) {
                    proxy.scrollTo(candidate.id, anchor: .center)
                }
                #if os(iOS)
                PlatformHaptics.messageMenuOpened()
                #endif
                return
            }
        }
    }

    private func updateTimelineFollowState(nearBottom: Bool, messageCount: Int) {
        if isNearBottom != nearBottom {
            isNearBottom = nearBottom
        }
        if messageCount == renderedMessageCount, shouldFollowLatest != nearBottom {
            shouldFollowLatest = nearBottom
        }
    }

    private func messageInfoContext(for selection: MessageInfoSelection) -> (message: ChatMessageSnapshot, chat: CurrentChatSnapshot?) {
        let currentChat = manager.state.currentChat?.chatId == selection.chatId ? manager.state.currentChat : nil
        let message = currentChat?.messages.first { $0.id == selection.messageId } ?? selection.snapshot
        return (message, currentChat)
    }

    private func reactorsContext(for selection: MessageReactorsSelection) -> (reactors: [MessageReactor], chat: CurrentChatSnapshot?) {
        let currentChat = chat
        let message = currentChat?.messages.first { $0.id == selection.messageId }
        return (message?.reactors ?? [], currentChat)
    }

}

private struct MessageReactorsSelection: Identifiable {
    let messageId: String
    var id: String { messageId }
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

// Reports the timeline's intrinsic content height (sum of all bubbles +
// padding), independent of scroll position. Used as a stable "did the
// content grow?" signal for the auto-stick-to-bottom logic — bottomMaxY
// alone changes when the user scrolls, which made the old check
// repeatedly drag the user back to the bottom every time they scrolled
// up.
private struct ChatTimelineContentHeightPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

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

private func chatTimelineGeometryMatches(_ lhs: CGFloat, _ rhs: CGFloat) -> Bool {
    if lhs == rhs {
        return true
    }
    guard lhs.isFinite, rhs.isFinite else {
        return false
    }
    return abs(lhs - rhs) < 0.5
}

private struct ChatMessageRow: View, Equatable {
    // Only compare the data SwiftUI actually renders from. Closures captured
    // by the parent are recreated on every parent body call (relay events,
    // typing pings, scene phase, …) and would otherwise force this row's
    // body to re-evaluate every time. With Equatable + .equatable() in the
    // ForEach, SwiftUI skips body when nothing visible changed, so a single
    // message update only re-renders that one row instead of all 50.
    static func == (lhs: ChatMessageRow, rhs: ChatMessageRow) -> Bool {
        lhs.message == rhs.message
            && lhs.reactions == rhs.reactions
            && lhs.chatKind == rhs.chatKind
            && lhs.showDayChip == rhs.showDayChip
            && lhs.isFirstInCluster == rhs.isFirstInCluster
            && lhs.isLastInCluster == rhs.isLastInCluster
    }

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
    let onScrollToQuote: (ReplyPreview) -> Void
    let onShowReactors: () -> Void
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var isHovering = false
    @State private var showReactionPicker = false
    @State private var showActionsSheet = false

    private var showActionDock: Bool {
        IrisLayout.usesDesktopChrome && isHovering
    }

    private var postReactionSuggestions: [String] {
        irisPostReactionSuggestionEmojis(reactions)
    }

    var body: some View {
        // Hoist a couple of computed values that are read 3-4 times in this
        // body so we don't pay for parsing/struct construction on every
        // access. SwiftUI re-evaluates body whenever the parent ChatScreen
        // re-runs, which happens on any AppManager publish.
        let parsed = parseReplyEncodedMessage(message.body)
        let bubble = ChatMessageBubbleShape(
            isOutgoing: message.isOutgoing,
            isFirstInCluster: isFirstInCluster,
            isLastInCluster: isLastInCluster
        )
        return VStack(spacing: 0) {
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
                    if message.isOutgoing {
                        Spacer(minLength: 56)
                    }
                    if showActionDock && message.isOutgoing {
                        ChatMessageActionDock(
                            onShowReactionPicker: { showReactionPicker = true },
                            onReply: onReply,
                            onCopy: {
                                PlatformClipboard.setString(copyableMessageText(message))
                            },
                            onInfo: onInfo,
                            onDelete: onDelete
                        )
                    }

                    VStack(alignment: message.isOutgoing ? .trailing : .leading, spacing: 8) {
                        if let reply = parsed.reply {
                            ReplyPreviewView(
                                reply: reply,
                                isOutgoing: message.isOutgoing,
                                onTap: { onScrollToQuote(reply) }
                            )
                        }
                        if !parsed.body.isEmpty {
                            TruncatableMessageBody(
                                attributed: linkedMessageAttributedString(
                                    parsed.body,
                                    linkColor: message.isOutgoing ? palette.onBubbleMine : palette.accentAlt
                                ),
                                isOutgoing: message.isOutgoing
                            )
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
                        bubble
                            .fill(message.isOutgoing ? palette.bubbleMine : palette.bubbleTheirs)
                    )
                    .clipShape(bubble)
                    .contentShape(bubble)
                    .onLongPressGesture(minimumDuration: 0.4) {
                        if !IrisLayout.usesDesktopChrome {
                            PlatformHaptics.messageMenuOpened()
                            showActionsSheet = true
                        }
                    }
                    .sheet(isPresented: $showActionsSheet) {
                        ChatMessageActionsSheet(
                            message: message,
                            bodyText: parsed.body,
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
                            suggestedEmojis: postReactionSuggestions,
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
                    // Swipe-to-reply is scoped to the bubble itself, not
                    // the whole row, so dragging from the gutter or below a
                    // reaction pill leaves the chat ScrollView free to
                    // scroll — matches Signal's UIPanGestureRecognizer
                    // attached to the cell's bubble subview.
                    .applyMessageBubbleSwipe(onReply: onReply, onInfo: onInfo)

                    if showActionDock && !message.isOutgoing {
                        ChatMessageActionDock(
                            onShowReactionPicker: { showReactionPicker = true },
                            onReply: onReply,
                            onCopy: {
                                PlatformClipboard.setString(copyableMessageText(message))
                            },
                            onInfo: onInfo,
                            onDelete: onDelete
                        )
                    }
                    if !message.isOutgoing {
                        Spacer(minLength: 56)
                    }
                }

                if !reactions.isEmpty {
                    ReactionRow(
                        reactions: reactions,
                        isOutgoing: message.isOutgoing,
                        onTap: onShowReactors
                    )
                        // Tuck the reaction pills up under the bubble's
                        // bottom edge so they read as attached to the
                        // message rather than a separate row below it.
                        .padding(.top, -14)
                        .padding(message.isOutgoing ? .trailing : .leading, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: message.isOutgoing ? .trailing : .leading)
            .contentShape(Rectangle())
            .onHover { isHovering = $0 }
            // Within-cluster gap was 4pt (top=4, bottom=0) — visually
            // smushed once the dark theme moved to pure-black panels,
            // because the thin slice of background between two
            // similarly-coloured bubbles read as zero. Bumped to
            // ~8pt so consecutive same-author messages stay obviously
            // distinct without losing the visual grouping. Between
            // clusters stays at the previous ~20pt.
            .padding(.top, isFirstInCluster ? 10 : 8)
            .padding(.bottom, isLastInCluster ? 10 : 0)
            }
        }
    }
}

// Caps tall message bubbles behind a Show more/less toggle. Uses
// ViewThatFits — a rendered-size check — so weird unicode that
// renders unusually tall but has few visible newlines still gets
// caught.
private struct TruncatableMessageBody: View {
    let attributed: AttributedString
    let isOutgoing: Bool
    @Environment(\.irisPalette) private var palette
    @State private var isExpanded = false

    private let collapsedMaxHeight: CGFloat = 320
    private let toggleReserve: CGFloat = 30

    var body: some View {
        if isExpanded {
            VStack(alignment: .leading, spacing: 4) {
                bodyText
                toggleButton(label: "Show less")
            }
        } else {
            ViewThatFits(in: .vertical) {
                bodyText
                VStack(alignment: .leading, spacing: 4) {
                    bodyText
                        .frame(
                            maxHeight: collapsedMaxHeight - toggleReserve,
                            alignment: .topLeading
                        )
                        .clipped()
                    toggleButton(label: "Show more")
                }
            }
            .frame(maxHeight: collapsedMaxHeight, alignment: .topLeading)
        }
    }

    private var bodyText: some View {
        Text(attributed)
            .font(.system(.body, design: .rounded))
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func toggleButton(label: String) -> some View {
        Button {
            withAnimation(.easeInOut(duration: 0.18)) { isExpanded.toggle() }
        } label: {
            Text(label)
                .font(.system(.caption, design: .rounded, weight: .semibold))
                // Match the bubble's text colour with a mute, same
                // pattern the timestamp uses. The brand purple
                // (`palette.accent`) is reserved for surfaces — never
                // text — so the toggle stays readable on either bubble
                // without lighting up purple on the chat canvas.
                .foregroundStyle(
                    (isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs)
                        .opacity(0.85)
                )
                .padding(.top, 2)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("chatMessageBodyToggle")
    }
}

private struct MessageInfoSheet: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let chat: CurrentChatSnapshot?
    @ObservedObject var manager: AppManager
    let onClose: () -> Void

    private func participantInfo(_ pubkeyHex: String) -> ParticipantInfo {
        if let account = manager.state.account, account.publicKeyHex == pubkeyHex {
            let name = account.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            return ParticipantInfo(
                name: name.isEmpty ? "You" : name,
                pictureUrl: account.pictureUrl,
                isMe: true
            )
        }
        if let chat, chat.kind == .direct, chat.chatId == pubkeyHex {
            return ParticipantInfo(name: chat.displayName, pictureUrl: chat.pictureUrl, isMe: false)
        }
        return ParticipantInfo(name: shortNpub(pubkeyHex), pictureUrl: nil, isMe: false)
    }

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
                    rumorSection
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 16)
                .frame(maxWidth: IrisLayout.scrollMaxWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
                // Message details is a wall of identifiers — message
                // id, source event id, sender hex, attachment urls.
                // Default SwiftUI Text isn't selectable, so enable it
                // for the whole sheet so a long-press can copy any of
                // them onto the clipboard.
                .textSelection(.enabled)
            }
            .background(palette.background)
            .navigationTitle("Message Details")
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
                        let info = participantInfo(recipient.ownerPubkeyHex)
                        MessageInfoRecipientRow(
                            info: info,
                            subtitle: messageInfoDateTime(recipient.updatedAtSecs),
                            delivery: recipient.delivery,
                            manager: manager
                        )
                    }
                }
            } else {
                let info = ParticipantInfo(name: message.author, pictureUrl: chat?.pictureUrl, isMe: false)
                MessageInfoRecipientRow(
                    info: info,
                    subtitle: messageInfoDateTime(message.createdAtSecs),
                    delivery: message.delivery,
                    manager: manager
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
                        info: participantInfo(reactor.author),
                        emoji: reactor.emoji,
                        manager: manager
                    )
                }
            }
        }
    }

    private var rumorJson: String {
        synthesizeMessageRumorJson(
            message: message,
            chat: chat,
            account: manager.state.account
        )
    }

    private var rumorSection: some View {
        MessageInfoSection(title: "Inner rumor") {
            VStack(alignment: .leading, spacing: 8) {
                Text(rumorJson)
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(palette.textPrimary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                IrisCopyButton(
                    label: "Copy rumor JSON",
                    value: rumorJson,
                    compact: true
                )
                .accessibilityIdentifier("messageInfoRumorCopyButton")
            }
        }
    }
}

// Synthesizes a Nostr-rumor-shaped JSON from the snapshot. `pubkey` is a
// best-effort lookup: account.publicKeyHex for outgoing, the direct
// chat_id for incoming direct chats, and empty for groups (where the
// snapshot doesn't carry the sender's owner pubkey hex). The `id` field
// matches the rumor hash for messages that arrived as runtime rumors.
private func synthesizeMessageRumorJson(
    message: ChatMessageSnapshot,
    chat: CurrentChatSnapshot?,
    account: AccountSnapshot?
) -> String {
    let pubkey: String = {
        if message.isOutgoing, let account {
            return account.publicKeyHex
        }
        if let chat, chat.kind == .direct {
            return chat.chatId
        }
        return ""
    }()

    var tags: [[String]] = []
    if let expiresAtSecs = message.expiresAtSecs {
        tags.append(["expiration", String(expiresAtSecs)])
    }
    for attachment in message.attachments {
        tags.append(["imeta", "url \(attachment.htreeUrl)"])
    }

    var content = message.body
    if !message.attachments.isEmpty {
        let urls = message.attachments.map { $0.htreeUrl }.joined(separator: "\n")
        content = content.isEmpty ? urls : content + "\n" + urls
    }

    let rumor: [String: Any] = [
        "id": message.id,
        "pubkey": pubkey,
        "created_at": message.createdAtSecs,
        "kind": 14,
        "tags": tags,
        "content": content,
    ]

    if let data = try? JSONSerialization.data(
        withJSONObject: rumor,
        options: [.prettyPrinted, .sortedKeys]
    ),
        let text = String(data: data, encoding: .utf8)
    {
        return text
    }
    return "{}"
}

private struct ParticipantInfo {
    let name: String
    let pictureUrl: String?
    let isMe: Bool
}

private struct MessageInfoReactorRow: View {
    @Environment(\.irisPalette) private var palette
    let info: ParticipantInfo
    let emoji: String
    @ObservedObject var manager: AppManager

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            IrisAvatar(
                label: info.name,
                size: 32,
                pictureUrl: info.pictureUrl,
                preferences: manager.state.preferences,
                manager: manager
            )
            Text(info.name)
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
                .buttonStyle(.irisPlain)
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
    let info: ParticipantInfo
    let subtitle: String
    let delivery: DeliveryState
    @ObservedObject var manager: AppManager

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            IrisAvatar(
                label: info.name,
                size: 32,
                pictureUrl: info.pictureUrl,
                preferences: manager.state.preferences,
                manager: manager
            )
            VStack(alignment: .leading, spacing: 3) {
                Text(info.name)
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                Text("\(irisDeliveryLabel(delivery)) - \(subtitle)")
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
            }
            Spacer(minLength: 0)
            IrisDeliveryGlyph(delivery: delivery)
                .frame(width: 18, height: 18)
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
    let onCopy: () -> Void
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
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("messageReactButton")
            dockButton("arrowshape.turn.up.left", identifier: "messageReplyButton", action: onReply)
            Menu {
                Button("Copy text", action: onCopy)
                Button("Message Details", action: onInfo)
                Button("Delete message", role: .destructive, action: onDelete)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .bold))
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.irisPlain)
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
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier(identifier)
    }
}

private let quickReactionEmojis: [String] = ["❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"]

func irisPostReactionSuggestionEmojis(_ reactions: [MessageReactionSnapshot]) -> [String] {
    reactions
        .filter { reaction in
            reaction.count > (reaction.reactedByMe ? UInt64(1) : UInt64(0))
        }
        .map(\.emoji)
}

private func irisUniqueEmojis(_ emojis: [String]) -> [String] {
    var seen = Set<String>()
    var result: [String] = []
    for emoji in emojis {
        let trimmed = emoji.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, seen.insert(trimmed).inserted else {
            continue
        }
        result.append(trimmed)
    }
    return result
}

private func irisReactionQuickChoices(postSuggestions: [String]) -> [String] {
    Array(irisUniqueEmojis(postSuggestions + IrisRecentEmojiStore.emojis() + quickReactionEmojis).prefix(7))
}

private enum IrisRecentEmojiStore {
    private static let key = "iris.recentReactionEmojis"
    private static let limit = 16

    static func emojis() -> [String] {
        guard let values = UserDefaults.standard.stringArray(forKey: key) else {
            return []
        }
        return irisUniqueEmojis(values)
    }

    static func remember(_ emoji: String) {
        let trimmed = emoji.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        let values = [trimmed] + emojis().filter { $0 != trimmed }
        UserDefaults.standard.set(Array(values.prefix(limit)), forKey: key)
    }
}

func irisEmojiMatchesSearch(_ emoji: String, category: String, query: String) -> Bool {
    let tokens = irisNormalizeEmojiSearchText(query)
        .split(separator: " ")
        .map(String.init)
    guard !tokens.isEmpty else {
        return true
    }

    let scalarNames = emoji.unicodeScalars
        .compactMap { $0.properties.name }
        .joined(separator: " ")
    let aliases = irisEmojiSearchAliases[emoji] ?? ""
    let haystack = irisNormalizeEmojiSearchText("\(emoji) \(category) \(scalarNames) \(aliases)")
    return tokens.allSatisfy { haystack.contains($0) }
}

private func irisNormalizeEmojiSearchText(_ value: String) -> String {
    value
        .folding(options: [.caseInsensitive, .diacriticInsensitive], locale: .current)
        .replacingOccurrences(of: "_", with: " ")
        .replacingOccurrences(of: "-", with: " ")
        .lowercased()
}

private let irisEmojiSearchAliases: [String: String] = [
    "😂": "laugh laughing lol haha",
    "🤣": "laugh laughing lol haha rolling",
    "😊": "smile smiling happy",
    "🙂": "smile smiling happy",
    "😍": "love heart eyes",
    "🥰": "love hearts",
    "😘": "kiss love",
    "😢": "sad tear crying",
    "😭": "sad cry crying",
    "😠": "angry mad",
    "🤬": "angry mad swearing",
    "🙏": "pray praying thanks thank you please",
    "👏": "clap applause",
    "🙌": "hooray yay hands",
    "❤️": "love heart red",
    "♥️": "love heart red",
    "🔥": "fire lit hot",
    "🎉": "party celebrate celebration",
    "🎊": "party celebrate celebration",
    "✨": "sparkle sparkles",
    "✅": "yes check done",
    "❌": "no cross x",
    "👀": "eyes look watching",
    "💯": "hundred perfect",
]

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

    private var postReactionSuggestions: [String] {
        irisPostReactionSuggestionEmojis(message.reactions)
    }

    var body: some View {
        VStack(spacing: 12) {
            quickReactionRow
            previewCard
            VStack(spacing: 0) {
                actionRow(icon: "arrowshape.turn.up.left", label: "Reply", action: onReply)
                actionRow(icon: "doc.on.doc", label: "Copy", action: onCopy)
                actionRow(icon: "info.circle", label: "Message Details", action: onInfo)
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
            ForEach(irisReactionQuickChoices(postSuggestions: postReactionSuggestions), id: \.self) { emoji in
                Button {
                    IrisRecentEmojiStore.remember(emoji)
                    onReact(emoji)
                } label: {
                    Text(emoji)
                        .font(.system(size: 26))
                        .frame(maxWidth: .infinity)
                        .frame(height: 40)
                }
                .buttonStyle(.irisPlain)
            }
            Button(action: onShowFullReactionPicker) {
                Image(systemName: "plus.circle")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .frame(height: 40)
            }
            .buttonStyle(.irisPlain)
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
        .buttonStyle(.irisPlain)
    }
}

struct IrisEmojiPicker: View {
    @Environment(\.irisPalette) private var palette
    let suggestedEmojis: [String]
    let onPick: (String) -> Void
    let onClose: (() -> Void)?

    @State private var query: String = ""
    @State private var recentEmojis: [String] = IrisRecentEmojiStore.emojis()

    init(
        suggestedEmojis: [String] = [],
        onClose: (() -> Void)? = nil,
        onPick: @escaping (String) -> Void
    ) {
        self.suggestedEmojis = suggestedEmojis
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
        guard !q.isEmpty else {
            var sections: [(String, String, [String])] = []
            let postEmojis = irisUniqueEmojis(suggestedEmojis)
            if !postEmojis.isEmpty {
                sections.append(("On this post", "bubble.left.and.bubble.right.fill", postEmojis))
            }
            let recent = irisUniqueEmojis(recentEmojis).filter { !postEmojis.contains($0) }
            if !recent.isEmpty {
                sections.append(("Recent", "clock.fill", recent))
            }
            return sections + Self.categories
        }
        return Self.categories.compactMap { name, icon, list in
            let hits = list.filter { irisEmojiMatchesSearch($0, category: name, query: q) }
            return hits.isEmpty ? nil : (name, icon, hits)
        }
    }

    private let columns = [GridItem(.adaptive(minimum: 40), spacing: 4)]

    private func pick(_ emoji: String) {
        IrisRecentEmojiStore.remember(emoji)
        recentEmojis = IrisRecentEmojiStore.emojis()
        onPick(emoji)
    }

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
                                ForEach(Array(list.enumerated()), id: \.offset) { _, emoji in
                                    Button {
                                        pick(emoji)
                                    } label: {
                                        Text(emoji)
                                            .font(.system(size: 26))
                                            .frame(width: 36, height: 36)
                                    }
                                    .buttonStyle(.irisPlain)
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
        .onAppear {
            recentEmojis = IrisRecentEmojiStore.emojis()
        }
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

private struct MessageReactorsSheet: View {
    @Environment(\.irisPalette) private var palette
    let reactors: [MessageReactor]
    let chat: CurrentChatSnapshot?
    @ObservedObject var manager: AppManager
    let onClose: () -> Void

    private var visibleReactors: [MessageReactor] {
        reactors.filter { !$0.emoji.isEmpty }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(visibleReactors, id: \.author) { reactor in
                        MessageInfoReactorRow(
                            info: participantInfo(reactor.author),
                            emoji: reactor.emoji,
                            manager: manager
                        )
                        .padding(.horizontal, 18)
                    }
                }
                .padding(.vertical, 8)
                .frame(maxWidth: IrisLayout.scrollMaxWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
            }
            .background(palette.background)
            .navigationTitle("Reactions")
#if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done", action: onClose)
                }
            }
#elseif os(macOS)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: onClose)
                }
            }
#endif
        }
        .accessibilityIdentifier("messageReactorsSheet")
    }

    private func participantInfo(_ pubkeyHex: String) -> ParticipantInfo {
        if let account = manager.state.account, account.publicKeyHex == pubkeyHex {
            let name = account.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            return ParticipantInfo(
                name: name.isEmpty ? "You" : name,
                pictureUrl: account.pictureUrl,
                isMe: true
            )
        }
        if let chat, chat.kind == .direct, chat.chatId == pubkeyHex {
            return ParticipantInfo(name: chat.displayName, pictureUrl: chat.pictureUrl, isMe: false)
        }
        return ParticipantInfo(name: shortNpub(pubkeyHex), pictureUrl: nil, isMe: false)
    }
}

private struct ReactionRow: View {
    @Environment(\.irisPalette) private var palette
    let reactions: [MessageReactionSnapshot]
    let isOutgoing: Bool
    var onTap: (() -> Void)? = nil

    @ViewBuilder
    var body: some View {
        let pills = HStack(spacing: 5) {
            ForEach(reactions, id: \.emoji) { reaction in
                Text("\(reaction.emoji) \(reaction.count)")
                    .font(.system(.caption, design: .rounded, weight: reaction.reactedByMe ? .bold : .semibold))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 4)
                    .background(
                        Capsule(style: .continuous)
                            .fill(palette.panelAlt)
                    )
                    // Chat-background-coloured ring carves a visible gap
                    // around the pill so it reads as a floating chip
                    // when tucked under the bubble's lower edge — same
                    // trick Signal uses.
                    .overlay(
                        Capsule(style: .continuous)
                            .strokeBorder(palette.background, lineWidth: 2)
                    )
            }
        }
        .frame(maxWidth: .infinity, alignment: isOutgoing ? .trailing : .leading)
        .accessibilityIdentifier("chatReactionRow")

        if let onTap {
            Button(action: onTap) { pills }
                .buttonStyle(.irisPlain)
                .accessibilityHint("Tap to see who reacted")
        } else {
            pills
        }
    }
}

extension View {
    @ViewBuilder
    func applyMessageBubbleSwipe(onReply: @escaping () -> Void, onInfo: @escaping () -> Void) -> some View {
#if os(iOS)
        modifier(MessageBubbleSwipeActions(onReply: onReply, onInfo: onInfo))
#else
        self
#endif
    }
}

#if os(iOS)
// SwiftUI doesn't expose UIKit's `state = .failed` semantics, so we can't
// match Signal's UIPanGestureRecognizer subclass approach (a previous
// attempt wrapped the recognizer via UIHostingController, which broke
// XCTest hit-testing on the inner Text). The simultaneousGesture path
// below approximates the same intent: a 22pt activation distance keeps
// short scroll drags from triggering us, and an axis lock based on the
// first past-threshold sample keeps us from offsetting on diagonal drags
// where vertical dominates.
private struct MessageBubbleSwipeActions: ViewModifier {
    let onReply: () -> Void
    let onInfo: () -> Void

    @State private var dragOffset: CGFloat = 0
    @State private var hasFedHaptic = false
    @State private var locked: Axis? = nil

    private let threshold: CGFloat = 60
    private let maxOffset: CGFloat = 90
    // High enough to clear the ScrollView's ~10pt pan threshold so vertical
    // scrolls win the gesture race; low enough that a deliberate horizontal
    // swipe still feels responsive.
    private let activationDistance: CGFloat = 22

    private enum Axis { case horizontal, vertical }

    func body(content: Content) -> some View {
        // Use overlays anchored to the bubble's natural frame instead of a
        // ZStack with a Spacer-expanded HStack — that pattern made the
        // wrapper balloon to the row's full width, which centered the
        // bubble and dropped invisible hint icons across the whole row
        // (eating taps and scroll). With overlays, the hint icons sit on
        // the bubble's leading/trailing edges and don't grow the layout
        // frame; .allowsHitTesting(false) makes sure they never steal
        // touches from the bubble below.
        content
            .offset(x: dragOffset)
            .overlay(alignment: .leading) {
                Image(systemName: "arrowshape.turn.up.left.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .opacity(reveal(forwards: true))
                    .scaleEffect(0.7 + 0.3 * reveal(forwards: true))
                    .padding(.leading, 14)
                    .allowsHitTesting(false)
            }
            .overlay(alignment: .trailing) {
                Image(systemName: "info.circle.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .opacity(reveal(forwards: false))
                    .scaleEffect(0.7 + 0.3 * reveal(forwards: false))
                    .padding(.trailing, 14)
                    .allowsHitTesting(false)
            }
        // simultaneousGesture lets the chat ScrollView's pan run alongside us;
        // the activation threshold + axis lock keep us from offsetting on
        // vertical drags even when the gesture fires.
        .simultaneousGesture(
            DragGesture(minimumDistance: activationDistance, coordinateSpace: .local)
                .onChanged { value in
                    let dx = value.translation.width
                    let dy = value.translation.height
                    if locked == nil {
                        // First sample past the threshold sets the axis. If
                        // vertical wins the slope test, lock to .vertical and
                        // never touch dragOffset for the rest of this drag.
                        locked = abs(dx) > abs(dy) ? .horizontal : .vertical
                    }
                    guard locked == .horizontal else { return }
                    let clamped = max(-maxOffset, min(maxOffset, dx))
                    dragOffset = clamped
                    let crossed = abs(clamped) >= threshold
                    if crossed && !hasFedHaptic {
                        PlatformHaptics.messageMenuOpened()
                        hasFedHaptic = true
                    } else if !crossed {
                        hasFedHaptic = false
                    }
                }
                .onEnded { value in
                    let final = locked == .horizontal ? max(-maxOffset, min(maxOffset, value.translation.width)) : 0
                    withAnimation(.spring(response: 0.32, dampingFraction: 0.74)) {
                        dragOffset = 0
                    }
                    if final >= threshold {
                        onReply()
                    } else if final <= -threshold {
                        onInfo()
                    }
                    hasFedHaptic = false
                    locked = nil
                }
        )
    }

    private func handleChange(_ dx: CGFloat) {
        let clamped = max(-maxOffset, min(maxOffset, dx))
        dragOffset = clamped
        let crossed = abs(clamped) >= threshold
        if crossed && !hasFedHaptic {
            PlatformHaptics.messageMenuOpened()
            hasFedHaptic = true
        } else if !crossed {
            hasFedHaptic = false
        }
    }

    private func handleEnded(_ dx: CGFloat) {
        let final = max(-maxOffset, min(maxOffset, dx))
        withAnimation(.spring(response: 0.32, dampingFraction: 0.74)) {
            dragOffset = 0
        }
        if final >= threshold {
            onReply()
        } else if final <= -threshold {
            onInfo()
        }
        hasFedHaptic = false
    }

    private func reveal(forwards: Bool) -> Double {
        let v = forwards ? dragOffset : -dragOffset
        return Double(min(1, max(0, v / threshold)))
    }
}
#endif

private struct EscDismissesReply: ViewModifier {
    @Binding var replyTarget: ChatMessageSnapshot?

    func body(content: Content) -> some View {
        if #available(iOS 17.0, macOS 14.0, *) {
            content.onKeyPress(.escape) {
                if replyTarget != nil {
                    replyTarget = nil
                    return .handled
                }
                return .ignored
            }
        } else {
            content
        }
    }
}

private struct IrisReplyComposerStrip: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Capsule()
                .fill(palette.accent)
                .frame(width: 3, height: 32)
            VStack(alignment: .leading, spacing: 1) {
                Text(message.author)
                    .font(.system(.caption, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
                    .lineLimit(1)
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
            .buttonStyle(.irisPlain)
        }
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 18 : 16)
        .padding(.vertical, 6)
        .background(palette.toolbar)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityIdentifier("chatReplyComposer")
    }
}

private struct ReplyPreviewView: View {
    @Environment(\.irisPalette) private var palette
    let reply: ReplyPreview
    let isOutgoing: Bool
    let onTap: () -> Void

    private let collapsedLineLimit = 4

    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 8) {
                Rectangle()
                    // Match the quote text color (the bubble's foreground)
                    // so the rule reads as a margin marker, not an accent
                    // band — it was previously the app accent (purple) on
                    // incoming bubbles, which fought with the message.
                    .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.6))
                    .frame(width: 3)
                    .clipShape(Capsule())
                VStack(alignment: .leading, spacing: 2) {
                    Text(reply.author)
                        .font(.system(.caption, design: .rounded, weight: .bold))
                    Text(reply.body)
                        .font(.system(.caption, design: .rounded, weight: .medium))
                        .lineLimit(collapsedLineLimit)
                        .multilineTextAlignment(.leading)
                        .opacity(0.82)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            // Stretch the rounded background to the bubble's full inner
            // width so the quote pill matches what the body text below
            // gets. Spacer above pushes the rule+text to leading; the
            // frame here just lets the surrounding fill grow.
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
            )
        }
        .buttonStyle(.irisPlain)
        .accessibilityHint("Tap to scroll to the quoted message")
        .accessibilityIdentifier("chatReplyPreview")
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

private enum ChatAttachmentPreviewImageCache {
    private static let cache: NSCache<NSString, PlatformImage> = {
        let cache = NSCache<NSString, PlatformImage>()
        cache.countLimit = 120
        cache.totalCostLimit = 48 * 1024 * 1024
        return cache
    }()

    static func image(for key: String) -> PlatformImage? {
        cache.object(forKey: key as NSString)
    }

    static func store(_ image: PlatformImage, for key: String) {
        cache.setObject(image, forKey: key as NSString, cost: imagePreviewCost(image))
    }
}

private func makeChatAttachmentPreviewImage(data: Data, filename: String) -> PlatformImage? {
    guard !isAnimatedImage(data: data, filename: filename) else {
        return nil
    }

    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions as CFDictionary) else {
        return nil
    }

    let maxPixelSize = 512
    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: maxPixelSize
    ]
    let fullImageOptions: [CFString: Any] = [
        kCGImageSourceShouldCacheImmediately: true
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(
        source,
        0,
        thumbnailOptions as CFDictionary
    ) ?? CGImageSourceCreateImageAtIndex(source, 0, fullImageOptions as CFDictionary) else {
        return nil
    }

    #if os(iOS)
    return PlatformImage(cgImage: cgImage)
    #elseif os(macOS)
    return PlatformImage(
        cgImage: cgImage,
        size: NSSize(width: cgImage.width, height: cgImage.height)
    )
    #else
    return nil
    #endif
}

private func imagePreviewCost(_ image: PlatformImage) -> Int {
    #if os(iOS)
    let width = max(1, Int(image.size.width * image.scale))
    let height = max(1, Int(image.size.height * image.scale))
    let pixels = width * height
    return pixels * 4
    #elseif os(macOS)
    let width = max(1, Int(image.size.width))
    let height = max(1, Int(image.size.height))
    let pixels = width * height
    return pixels * 4
    #else
    return 1
    #endif
}

private struct ChatAttachmentView: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var localImageData: Data?
    @State private var localPreviewImage: PlatformImage?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false
    @State private var isOpeningAttachment = false

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
                    if let localPreviewImage {
                        Image(platformImage: localPreviewImage)
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
            .buttonStyle(.irisPlain)
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
            .buttonStyle(.irisPlain)
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
        if let cached = ChatAttachmentPreviewImageCache.image(for: attachment.htreeUrl) {
            localPreviewImage = cached
        }
        guard let data = await downloadAttachment(attachment) else {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        let isAnimated = isAnimatedImage(data: data, filename: attachment.filename)
        if !isAnimated, localPreviewImage == nil {
            guard let preview = makeChatAttachmentPreviewImage(data: data, filename: attachment.filename) else {
                isLoadingImage = false
                failedImageLoad = true
                return
            }
            ChatAttachmentPreviewImageCache.store(preview, for: attachment.htreeUrl)
            localPreviewImage = preview
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
                    .buttonStyle(.irisPlain)
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
