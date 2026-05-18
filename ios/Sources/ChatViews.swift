import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

private struct IrisBlockedComposerBar: View {
    @Environment(\.irisPalette) private var palette
    let onUnblock: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "nosign")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(.red)
            Text("User blocked")
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
            Spacer(minLength: 0)
            Button("Unblock", action: onUnblock)
                .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .background(.regularMaterial)
        .accessibilityIdentifier("blockedComposerBar")
    }
}

/// Message-request gate shown in place of the composer when the user
/// hasn't replied to a stranger yet. Mirrors Signal's pattern — the
/// recipient can read the message and decide whether to engage. While
/// this bar is up, the Rust core suppresses outgoing delivered / read
/// receipts so the sender gets no signal about whether the message
/// was seen.
private struct IrisMessageRequestBar: View {
    @Environment(\.irisPalette) private var palette
    let displayName: String
    let onAccept: () -> Void
    let onDelete: () -> Void
    let onBlock: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text("Message request from \(displayName)")
                .font(.system(.footnote, design: .rounded, weight: .medium))
                .foregroundStyle(palette.muted)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 14)

            HStack(spacing: 8) {
                requestButton(
                    "Block",
                    accessibilityId: "messageRequestBlockButton",
                    role: .destructive,
                    action: onBlock
                )
                requestButton(
                    "Delete",
                    accessibilityId: "messageRequestDeleteButton",
                    role: nil,
                    action: onDelete
                )
                requestButton(
                    "Accept",
                    accessibilityId: "messageRequestAcceptButton",
                    role: nil,
                    emphasized: true,
                    action: onAccept
                )
            }
            .padding(.horizontal, 14)
        }
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity)
        .background(.regularMaterial)
        .accessibilityIdentifier("messageRequestBar")
    }

    @ViewBuilder
    private func requestButton(
        _ label: String,
        accessibilityId: String,
        role: ButtonRole?,
        emphasized: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            Text(label)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .frame(maxWidth: .infinity, minHeight: 36)
                .foregroundStyle(emphasized ? Color.white : palette.textPrimary)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(emphasized ? palette.accent : palette.panelAlt)
                )
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(accessibilityId)
    }
}

/// Identifies the chat the block confirmation dialog is acting on.
/// `Identifiable` lets `.confirmationDialog(item:)` rebuild the sheet
/// when the user changes target without a separate `isPresented` flag.
private struct BlockConfirmationTarget: Identifiable {
    let chatId: String
    let displayName: String
    var id: String { chatId }
}

/// Signal-style block confirmation: tapping Block on the
/// message-request bar lifts a sheet with "Block" (keep the thread
/// for evidence) and "Block and Delete" (wipe the chat too). Mirrors
/// `ConversationViewController+MessageRequest.swift` in signal-ios.
/// Extracted to a `ViewModifier` so the ChatScreen body type-checks
/// in reasonable time — the inline form pushed the closure over the
/// compiler's expression-complexity threshold.
private struct BlockConfirmationModifier: ViewModifier {
    @Binding var target: BlockConfirmationTarget?
    let manager: AppManager

    func body(content: Content) -> some View {
        content.confirmationDialog(
            target.map { "Block \($0.displayName)?" } ?? "Block?",
            isPresented: Binding(
                get: { target != nil },
                set: { presented in
                    if !presented { target = nil }
                }
            ),
            titleVisibility: .visible,
            presenting: target,
            actions: { item in
                Button("Block", role: .destructive) {
                    manager.setUserBlocked(item.chatId, blocked: true)
                    target = nil
                }
                .accessibilityIdentifier("messageRequestBlockConfirmKeep")
                Button("Block and Delete", role: .destructive) {
                    manager.setUserBlocked(item.chatId, blocked: true)
                    manager.dispatch(.deleteChat(chatId: item.chatId))
                    manager.navigateBack()
                    target = nil
                }
                .accessibilityIdentifier("messageRequestBlockConfirmDelete")
                Button("Cancel", role: .cancel) {
                    target = nil
                }
            },
            message: { _ in
                Text("They won't be able to message you. No notification is sent.")
            }
        )
    }
}

struct ChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.irisNavigationHeaderTopInset) private var navigationHeaderTopInset
    @ObservedObject var manager: AppManager
    let chatId: String

    @State private var draft = ""
    @State private var selectedAttachments: [StagedAttachment] = []
    @State private var isNearBottom = true
    @State private var shouldFollowLatest = true
    @State private var forceScrollToLatest = false
    @State private var pendingScrollSettle: DispatchWorkItem?
    @State private var timelineUserScrollGeneration = 0
    @State private var timelineScrollSettleGeneration = 0
    @State private var timelineAutoFollowSuppressedUntil: Date?
    @State private var timelineViewportMinY: CGFloat = 0
    @State private var timelineViewportMaxY: CGFloat = 0
    @State private var timelineTopMinY: CGFloat = -.greatestFiniteMagnitude
    @State private var timelineBottomMaxY: CGFloat = .greatestFiniteMagnitude
    @State private var timelineContentHeight: CGFloat = 0
    @State private var timelineDaySeparatorFrames: [String: ChatTimelineDaySeparatorFrame] = [:]
    @State private var initialScrollPending = true
    @State private var timelineReadyForDisplay = false
    @State private var renderedMessageCount = 0
    @State private var pendingPrependAnchorMessageId: String?
    @StateObject private var timelineCoordinator = ChatTimelineInteractionCoordinator()
    @State private var activeBubbleSwipe: ActiveMessageBubbleSwipe?
    @State private var replyTarget: ChatMessageSnapshot?
    @State private var imageViewerItem: ImageViewerItem?
    @State private var lastTypingSentAt: Date?
    @State private var sentTypingIndicator = false
    @State private var messageInfoSelection: MessageInfoSelection?
    @State private var reactorsSelection: MessageReactorsSelection?
    @State private var lastPersistedDraft: String?
    @State private var draftFlushWork: DispatchWorkItem?
    /// Session-scoped acceptance for message-request chats. While
    /// `chat.isRequest` is still true at the model layer (Rust),
    /// tapping Accept just hides the gate locally so the user can
    /// reply; sending a message naturally clears `isRequest` for good
    /// because there's now an outgoing message in the thread.
    @State private var acceptedRequestChatId: String?
    @State private var blockConfirmationChat: BlockConfirmationTarget?
    @FocusState private var isComposerFocused: Bool

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    private var persistedDraftToken: String {
        "\(chatId)|\(persistedDraftForCurrentChat())"
    }

    var body: some View {
        let floatingSeparator = floatingDaySeparator()
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
                                            Color.clear
                                                .frame(height: 1)
                                                .id(ChatTimelineAnchor.top)
                                                .background(
                                                    GeometryReader { geometry in
                                                        Color.clear.preference(
                                                            key: ChatTimelineTopMinYPreferenceKey.self,
                                                            value: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name)).minY
                                                        )
                                                    }
                                                )
                                                .accessibilityHidden(true)

                                            ForEach(Array(visibleMessages.enumerated()), id: \.element.id) { index, message in
                                                let previous = index > 0 ? visibleMessages[index - 1] : nil
                                                let next = index + 1 < visibleMessages.count ? visibleMessages[index + 1] : nil
                                                chatMessageRow(
                                                    message: message,
                                                    previous: previous,
                                                    next: next,
                                                    chat: chat,
                                                    hidesInlineDayChip: floatingSeparator?.messageId == message.id,
                                                    proxy: proxy
                                                )
                                            }
                                        }
                                        .padding(
                                            .horizontal,
                                            IrisLayout.usesDesktopChrome ? 18 : SignalConversationLayout.contentGutter
                                        )
                                        .padding(.top, SignalConversationLayout.contentTopMargin + navigationHeaderTopInset)
                                        .padding(.bottom, SignalConversationLayout.contentBottomMargin)
                                        .contentShape(Rectangle())
                                        .simultaneousGesture(
                                            TapGesture().onEnded {
                                                dismissComposerFocus()
                                            }
                                        )
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
                                        // Vertical ScrollView children do not
                                        // automatically fill the viewport
                                        // width on macOS. If the timeline stack
                                        // keeps its ideal width, outgoing
                                        // bubbles align to a narrow centered
                                        // column instead of the chat pane edge.
                                        .frame(width: viewport.size.width)
                                        .frame(minHeight: viewport.size.height, alignment: .bottom)

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
                                    .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
                                    .accessibilityIdentifier("chatTimeline")
                                    .observeChatTimelineScroll(coordinator: timelineCoordinator) { translationY, velocityY in
                                        handleTimelineUserPan(translationY: translationY, velocityY: velocityY)
                                    }
                                    .overlay {
                                        GeometryReader { geometry in
                                            let frame = geometry.frame(in: .named(ChatTimelineCoordinateSpace.name))
                                            Color.clear
                                                .preference(
                                                    key: ChatTimelineViewportMinYPreferenceKey.self,
                                                    value: frame.minY
                                                )
                                                .preference(
                                                    key: ChatTimelineViewportMaxYPreferenceKey.self,
                                                    value: frame.maxY
                                                )
                                        }
                                    }
                                    .irisInteractiveKeyboardDismiss()
                                    .simultaneousGesture(timelineDragGesture)
                                    .simultaneousGesture(
                                        TapGesture().onEnded {
                                            dismissComposerFocus()
                                        }
                                    )
                                    .opacity(timelineReadyForDisplay ? 1 : 0)
                                    .allowsHitTesting(timelineReadyForDisplay)
                                }
                                .irisOnChange(of: chatId) { _ in
                                    initialScrollPending = true
                                    timelineReadyForDisplay = false
                                    isNearBottom = true
                                    shouldFollowLatest = true
                                    forceScrollToLatest = false
                                    timelineAutoFollowSuppressedUntil = nil
                                    renderedMessageCount = 0
                                    pendingPrependAnchorMessageId = nil
                                    activeBubbleSwipe = nil
                                    timelineCoordinator.bubblePanRejected = false
                                    timelineTopMinY = -.greatestFiniteMagnitude
                                    timelineContentHeight = 0
                                    timelineDaySeparatorFrames = [:]
                                    lastTypingSentAt = nil
                                    sentTypingIndicator = false
                                }
                                .onPreferenceChange(ChatTimelineViewportMinYPreferenceKey.self) { value in
                                    if !chatTimelineGeometryMatches(timelineViewportMinY, value) {
                                        timelineViewportMinY = value
                                    }
                                    maybeLoadOlderMessages(chat: chat)
                                }
                                .onPreferenceChange(ChatTimelineTopMinYPreferenceKey.self) { value in
                                    if !chatTimelineGeometryMatches(timelineTopMinY, value) {
                                        timelineTopMinY = value
                                    }
                                    maybeLoadOlderMessages(chat: chat)
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
                                    let canAutoFollow = (shouldFollowLatest || isNearBottom)
                                        && !timelineAutoFollowIsSuppressed()
                                    if !initialScrollPending, canAutoFollow, grew {
                                        scrollToBottom(proxy: proxy, animated: false)
                                    }
                                }
                                .onPreferenceChange(ChatMessageBubbleFramePreferenceKey.self) { value in
                                    timelineCoordinator.messageBubbleFrames = value
                                }
                                .onPreferenceChange(ChatTimelineDaySeparatorFramePreferenceKey.self) { value in
                                    timelineDaySeparatorFrames = value
                                }
                                .task(id: chatTimelineScrollTaskToken(for: chat)) {
                                    guard !chat.messages.isEmpty else {
                                        initialScrollPending = true
                                        revealTimelineAfterLayout()
                                        shouldFollowLatest = true
                                        forceScrollToLatest = false
                                        renderedMessageCount = 0
                                        return
                                    }
                                    let messageCount = chat.messages.count
                                    if let anchorId = pendingPrependAnchorMessageId,
                                       chat.messages.contains(where: { $0.id == anchorId }) {
                                        renderedMessageCount = messageCount
                                        initialScrollPending = false
                                        scrollToMessage(proxy: proxy, messageId: anchorId, anchor: .top, animated: false)
                                        revealTimelineAfterLayout()
                                        pendingPrependAnchorMessageId = nil
                                        return
                                    }
                                    let messageCountIncreased = messageCount > renderedMessageCount
                                    // Search hits ask us to land on a
                                    // specific bubble instead of the
                                    // bottom of the timeline. Consume
                                    // the manager-side flag here so a
                                    // tap on a "Messages" row scrolls
                                    // straight to that message; falls
                                    // through to the regular bottom
                                    // scroll for normal opens.
                                    if let targetId = manager.pendingScrollMessageId {
                                        if chat.messages.contains(where: { $0.id == targetId }) {
                                            renderedMessageCount = messageCount
                                            initialScrollPending = false
                                            shouldFollowLatest = false
                                            forceScrollToLatest = false
                                            scrollToMessage(proxy: proxy, messageId: targetId)
                                            revealTimelineAfterLayout()
                                            manager.consumePendingScrollMessage()
                                            return
                                        }
                                        manager.loadChatAroundMessage(chatId: chat.chatId, messageId: targetId)
                                    }
                                    let shouldScroll = initialScrollPending
                                        || forceScrollToLatest
                                        || (
                                            messageCountIncreased
                                                && (shouldFollowLatest || isNearBottom)
                                                && !timelineAutoFollowIsSuppressed()
                                        )
                                    renderedMessageCount = messageCount
                                    if shouldScroll {
                                        let wasInitialScroll = initialScrollPending
                                        scrollToBottom(proxy: proxy, animated: !wasInitialScroll)
                                        initialScrollPending = false
                                        shouldFollowLatest = true
                                        if wasInitialScroll {
                                            revealTimelineAfterLayout()
                                        }
                                    } else {
                                        revealTimelineAfterLayout()
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
                                // the timeline scroll task and the
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

                                if timelineReadyForDisplay && !isNearBottom && !chat.messages.isEmpty {
                                    ChatJumpToBottomButton {
                                        jumpToLatest(proxy: proxy)
                                    }
                                    .padding(.trailing, 8)
                                    .padding(.bottom, 8)
                                    .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
                                }

                                if timelineReadyForDisplay && !chat.typingIndicators.isEmpty {
                                    IrisTypingIndicatorRow(indicators: chat.typingIndicators)
                                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                                        .padding(.leading, IrisLayout.usesDesktopChrome ? 22 : 16)
                                        .padding(.trailing, 76)
                                        .padding(.bottom, 16)
                                        .allowsHitTesting(false)
                                }

                                if timelineReadyForDisplay,
                                   let separator = floatingSeparator {
                                    HStack {
                                        Spacer()
                                        IrisDayChip(text: separator.text)
                                        Spacer()
                                    }
                                    .accessibilityElement(children: .ignore)
                                    .accessibilityLabel(separator.text)
                                    .accessibilityIdentifier("chatFloatingDaySeparator")
                                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                                    .offset(y: separator.offsetY)
                                    .allowsHitTesting(false)
                                    .zIndex(3)
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
                                let composerBlocked = chat.kind == .direct && manager.isUserBlocked(chat.chatId)
                                let isRequest = chat.isRequest && acceptedRequestChatId != chat.chatId
                                VStack(spacing: 0) {
                                    if let replyTarget, !composerBlocked, !isRequest {
                                        IrisReplyComposerStrip(message: replyTarget) {
                                            self.replyTarget = nil
                                        }
                                    }
                                    if composerBlocked {
                                        IrisBlockedComposerBar {
                                            manager.setUserBlocked(chat.chatId, blocked: false)
                                        }
                                    } else if isRequest {
                                        IrisMessageRequestBar(
                                            displayName: chat.displayName,
                                            onAccept: {
                                                acceptedRequestChatId = chat.chatId
                                                manager.dispatch(.setMessageRequestAccepted(chatId: chat.chatId))
                                                isComposerFocused = true
                                            },
                                            onDelete: {
                                                manager.dispatch(.deleteChat(chatId: chat.chatId))
                                                manager.navigateBack()
                                            },
                                            onBlock: {
                                                blockConfirmationChat = BlockConfirmationTarget(
                                                    chatId: chat.chatId,
                                                    displayName: chat.displayName
                                                )
                                            }
                                        )
                                    } else {
                                        IrisComposerBar(
                                            draft: $draft,
                                            attachments: $selectedAttachments,
                                            placeholder: "Message",
                                            isSending: manager.state.busy.sendingMessage,
                                            isUploading: manager.state.busy.uploadingAttachment,
                                            uploadFraction: uploadFraction(manager.state.busy.uploadProgress),
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
                                        ) { composerText in
                                            let text = composerText.trimmingCharacters(in: .whitespacesAndNewlines)
                                            guard !text.isEmpty || !selectedAttachments.isEmpty else { return }
                                            stopTypingIfNeeded()
                                            resumeTimelineAutoFollow()
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
        .modifier(ChatImageViewerPresenter(item: $imageViewerItem) { text in
            manager.startForward(text: text)
        })
        .sheet(item: $messageInfoSelection) { selection in
            let context = messageInfoContext(for: selection)
            MessageInfoSheet(message: context.message, chat: context.chat, manager: manager) {
                messageInfoSelection = nil
            }
            .irisModalSurface()
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
            .irisModalSurface()
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .irisDismissOnMacOutsideClick {
                reactorsSelection = nil
            }
        }
        .modifier(BlockConfirmationModifier(target: $blockConfirmationChat, manager: manager))
        .onDisappear {
            pendingScrollSettle?.cancel()
            pendingScrollSettle = nil
            stopTypingIfNeeded()
            flushDraftImmediately()
        }
        .task(id: chatId) {
            seedDraftFromPersistedState(replaceExisting: true)
        }
        .task(id: persistedDraftToken) {
            seedDraftFromPersistedState(replaceExisting: false)
        }
        .irisOnChange(of: draft) { newValue in
            scheduleDraftFlush(text: newValue)
        }
        .task(id: seenReceiptToken(for: chat)) {
            guard let chat else { return }
            guard manager.canMarkActiveChatSeen else { return }
            let incomingIds = chat.messages
                .filter { !$0.isOutgoing && $0.delivery != .seen }
                .map(\.id)
            guard !incomingIds.isEmpty else { return }
            manager.dispatch(.markMessagesSeen(chatId: chat.chatId, messageIds: incomingIds))
        }
    }

    private var timelineDragGesture: some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .named(ChatTimelineCoordinateSpace.name))
            .onChanged { value in
                handleTimelineUserPan(
                    translationY: value.translation.height,
                    velocityY: value.predictedEndTranslation.height - value.translation.height
                )
                handleMessageBubbleDragChanged(value)
            }
            .onEnded { value in
                handleMessageBubbleDragEnded(value)
            }
    }

    private func chatMessageRow(
        message: ChatMessageSnapshot,
        previous: ChatMessageSnapshot?,
        next: ChatMessageSnapshot?,
        chat: CurrentChatSnapshot,
        hidesInlineDayChip: Bool,
        proxy: ScrollViewProxy
    ) -> some View {
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
        let showsGroupSenderName = irisShowsGroupSenderName(
            previous: previous,
            message: message,
            chatKind: chat.kind
        )
        let showsGroupSenderAvatar = irisShowsGroupSenderAvatar(
            message: message,
            next: next,
            chatKind: chat.kind
        )

        return EquatableView(content: ChatMessageRow(
            message: message,
            chatKind: chat.kind,
            showDayChip: showDayChip,
            hidesInlineDayChip: hidesInlineDayChip,
            isFirstInCluster: isFirstInCluster,
            isLastInCluster: isLastInCluster,
            showsGroupSenderName: showsGroupSenderName,
            showsGroupSenderAvatar: showsGroupSenderAvatar,
            reactions: message.reactions,
            swipeOffset: activeBubbleSwipe?.messageId == message.id ? activeBubbleSwipe?.offset ?? 0 : 0,
            onReply: {
                replyTarget = message
                isComposerFocused = true
            },
            onForward: {
                manager.startForward(text: forwardableMessageText(message))
            },
            onForwardAttachment: { attachment in
                manager.startForward(text: forwardableAttachmentText(attachment))
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
                manager.dispatch(.deleteLocalMessage(chatId: chatId, messageId: message.id))
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
            onOpenImage: { data, attachment in
                let imageAttachments = message.attachments.filter { $0.isImage }
                let initialIndex = imageAttachments.firstIndex {
                    $0.htreeUrl == attachment.htreeUrl
                } ?? 0
                imageViewerItem = ImageViewerItem(
                    attachments: imageAttachments,
                    initialIndex: initialIndex,
                    initialData: data,
                    senderName: message.isOutgoing ? "You" : message.author,
                    createdAtSecs: message.createdAtSecs,
                    downloadAttachment: { att in
                        await manager.downloadAttachment(att)
                    },
                    forwardableTextFor: { att in
                        forwardableAttachmentText(att)
                    }
                )
            }
        ))
        .id(message.id)
    }

    private func maybeLoadOlderMessages(chat: CurrentChatSnapshot) {
        guard !initialScrollPending,
              let firstMessageId = chat.messages.first?.id,
              timelineTopMinY.isFinite,
              timelineViewportMinY.isFinite,
              timelineTopMinY >= timelineViewportMinY - 44 else {
            return
        }
        if pendingPrependAnchorMessageId == nil {
            pendingPrependAnchorMessageId = firstMessageId
        }
        if !manager.loadOlderMessages(chatId: chat.chatId, completion: { loaded in
            if !loaded {
                pendingPrependAnchorMessageId = nil
            }
        }) {
            pendingPrependAnchorMessageId = nil
        }
    }

    private func handleTimelineUserPan(translationY: CGFloat, velocityY: CGFloat) {
        if pendingScrollSettle != nil {
            pendingScrollSettle?.cancel()
            pendingScrollSettle = nil
            timelineUserScrollGeneration += 1
            timelineScrollSettleGeneration += 1
        }

        if translationY > 6 || velocityY > 60 {
            shouldFollowLatest = false
            timelineAutoFollowSuppressedUntil = Date().addingTimeInterval(1.2)
        } else if translationY < -6 || velocityY < -60 {
            pendingPrependAnchorMessageId = nil
            if isNearBottom {
                resumeTimelineAutoFollow()
                shouldFollowLatest = true
            }
        }
    }

    private func jumpToLatest(proxy: ScrollViewProxy) {
        pendingPrependAnchorMessageId = nil
        pendingScrollSettle?.cancel()
        pendingScrollSettle = nil
        timelineScrollSettleGeneration += 1
        dismissComposerFocus()
        resumeTimelineAutoFollow()
        timelineCoordinator.stopScrolling()
        // Match Signal's behavior: the button is a one-shot request to
        // land on the newest message. The normal geometry observer will
        // re-enable latest-following once the viewport is actually at
        // bottom; arming it here lets delayed layout work pin the user
        // down if they immediately drag upward again.
        shouldFollowLatest = false
        scrollToBottom(proxy: proxy, animated: true, settleAfterLayout: false)
    }

    private func handleMessageBubbleDragChanged(_ value: DragGesture.Value) {
        guard !timelineCoordinator.bubblePanRejected else { return }

        let horizontal = abs(value.translation.width)
        let vertical = abs(value.translation.height)
        if activeBubbleSwipe == nil {
            guard horizontal > ChatMessageBubbleSwipeMetrics.activationDistance
                    || vertical > ChatMessageBubbleSwipeMetrics.activationDistance else {
                return
            }
            guard horizontal > vertical else {
                timelineCoordinator.bubblePanRejected = true
                return
            }
            guard let messageId = timelineCoordinator.messageBubbleId(at: value.startLocation)
                    ?? timelineCoordinator.messageBubbleId(at: value.location) else {
                timelineCoordinator.bubblePanRejected = true
                return
            }
            activeBubbleSwipe = ActiveMessageBubbleSwipe(messageId: messageId, offset: 0, hasFedHaptic: false)
        } else if vertical > horizontal && vertical > ChatMessageBubbleSwipeMetrics.activationDistance {
            activeBubbleSwipe = nil
            timelineCoordinator.bubblePanRejected = true
            return
        }

        guard var swipe = activeBubbleSwipe else { return }
        let clamped = max(
            -ChatMessageBubbleSwipeMetrics.maxOffset,
            min(ChatMessageBubbleSwipeMetrics.maxOffset, value.translation.width)
        )
        swipe.offset = clamped
        let crossed = abs(clamped) >= ChatMessageBubbleSwipeMetrics.threshold
        if crossed && !swipe.hasFedHaptic {
            PlatformHaptics.messageMenuOpened()
            swipe.hasFedHaptic = true
        } else if !crossed {
            swipe.hasFedHaptic = false
        }
        activeBubbleSwipe = swipe
    }

    private func handleMessageBubbleDragEnded(_: DragGesture.Value) {
        timelineCoordinator.bubblePanRejected = false
        guard let chat, let swipe = activeBubbleSwipe else { return }
        activeBubbleSwipe = nil
        guard let message = chat.messages.first(where: { $0.id == swipe.messageId }) else { return }
        if swipe.offset >= ChatMessageBubbleSwipeMetrics.threshold {
            replyTarget = message
            isComposerFocused = true
        } else if swipe.offset <= -ChatMessageBubbleSwipeMetrics.threshold {
            messageInfoSelection = MessageInfoSelection(
                chatId: chat.chatId,
                messageId: message.id,
                snapshot: message
            )
        }
    }

    private func dismissComposerFocus() {
        isComposerFocused = false
#if os(iOS)
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
#endif
    }

    private func timelineAutoFollowIsSuppressed(now: Date = Date()) -> Bool {
        guard let until = timelineAutoFollowSuppressedUntil else { return false }
        return until > now
    }

    private func resumeTimelineAutoFollow() {
        timelineAutoFollowSuppressedUntil = nil
    }

    private func chatTimelineScrollTaskToken(for chat: CurrentChatSnapshot) -> String {
        [
            chat.chatId,
            chat.messages.first?.id ?? "",
            chat.messages.last?.id ?? "",
            String(chat.messages.count),
            manager.pendingScrollMessageId ?? "",
            pendingPrependAnchorMessageId ?? "",
        ].joined(separator: "|")
    }

    private func revealTimelineAfterLayout() {
        guard !timelineReadyForDisplay else { return }
        DispatchQueue.main.async {
            guard !timelineReadyForDisplay else { return }
            var transaction = Transaction()
            transaction.disablesAnimations = true
            withTransaction(transaction) {
                timelineReadyForDisplay = true
            }
        }
    }

    /// Centre the targeted bubble in the viewport for search-hit
    /// taps. Reuses the multi-tick re-scroll pattern from
    /// `scrollToBottom` so quoted-reply previews / images that
    /// resolve a moment after layout don't end up just off-screen.
    private func scrollToMessage(
        proxy: ScrollViewProxy,
        messageId: String,
        anchor: UnitPoint = .center,
        animated: Bool = true
    ) {
        let scroll = {
            let action = {
                proxy.scrollTo(messageId, anchor: anchor)
            }
            if animated {
                withAnimation(.easeOut(duration: 0.25)) {
                    action()
                }
            } else {
                action()
            }
        }
        DispatchQueue.main.async { scroll() }
        if animated {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { scroll() }
        }
    }

    /// Debounce composer writes so a fast typist generates one
    /// SQLite-row update every ~500ms instead of one per keystroke.
    /// On disappear / send we flush eagerly so the latest text always
    /// hits disk before the view goes away.
    private func persistedDraftForCurrentChat() -> String {
        if let currentChat = manager.state.currentChat, currentChat.chatId == chatId {
            return currentChat.draft
        }
        return manager.state.chatList.first { $0.chatId == chatId }?.draft ?? ""
    }

    private func seedDraftFromPersistedState(replaceExisting: Bool) {
        let persisted = persistedDraftForCurrentChat()
        if replaceExisting || draft.isEmpty {
            lastPersistedDraft = persisted
            draft = persisted
            return
        }
        if draft == persisted {
            lastPersistedDraft = persisted
        }
    }

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
        guard !manager.isUserBlocked(chatId) else { return }
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
        let messageIds = chat.messages
            .filter { !$0.isOutgoing && $0.delivery != .seen }
            .map(\.id)
            .joined(separator: ",")
        return [manager.seenEligibilityToken, messageIds].joined(separator: "|")
    }

    private func scrollToBottom(
        proxy: ScrollViewProxy,
        animated: Bool,
        settleAfterLayout: Bool = true
    ) {
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
        guard settleAfterLayout else {
            pendingScrollSettle = nil
            timelineScrollSettleGeneration += 1
            return
        }
        let userScrollGeneration = timelineUserScrollGeneration
        timelineScrollSettleGeneration += 1
        let settleGeneration = timelineScrollSettleGeneration
        let guardedItem = DispatchWorkItem {
            guard timelineScrollSettleGeneration == settleGeneration else { return }
            guard timelineUserScrollGeneration == userScrollGeneration else { return }
            guard !timelineAutoFollowIsSuppressed() else { return }
            scroll()
        }
        pendingScrollSettle = guardedItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: guardedItem)
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
        let nextShouldFollow = nearBottom && !timelineAutoFollowIsSuppressed()
        if messageCount == renderedMessageCount, shouldFollowLatest != nextShouldFollow {
            shouldFollowLatest = nextShouldFollow
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

    private func floatingDaySeparator() -> ChatFloatingDaySeparator? {
        let stickyOffsetY = navigationHeaderTopInset + SignalConversationLayout.stickyDateHeaderTopSpacing
        let topY = timelineViewportMinY + stickyOffsetY
        return irisFloatingDaySeparator(
            frames: Array(timelineDaySeparatorFrames.values),
            viewportMinY: timelineViewportMinY,
            stickyTopY: topY
        )
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

private enum SignalConversationLayout {
    static let bubbleWideCornerRadius: CGFloat = 18
    static let bubbleSharpCornerRadius: CGFloat = 4
    static let messageStackSpacing: CGFloat = 8
    static let defaultMessageSpacing: CGFloat = 12
    static let compactMessageSpacing: CGFloat = 2
    static let systemMessageSpacing: CGFloat = 20
    static let groupMessageAvatarSize: CGFloat = 28
    static let messageDirectionSpacing: CGFloat = 12
    static let textInsetHorizontal: CGFloat = 12
    static let textInsetVertical: CGFloat = 7
    static let contentGutter: CGFloat = 16
    static let contentTopMargin: CGFloat = 24
    static let contentBottomMargin: CGFloat = 8
    static let daySeparatorTopPadding: CGFloat = 12
    static let daySeparatorBottomPadding: CGFloat = 8
    static let daySeparatorMinimumHeight: CGFloat = 22
    static let stickyDateHeaderTopSpacing: CGFloat = 12
    static let stickyDateHeaderMinimumSpacing: CGFloat = 5
    static let reactionPillHeight: CGFloat = 24
    static let reactionPillOverlap: CGFloat = 7
    static let reactionPillHorizontalInset: CGFloat = 6
    static let reactionPillProtrusion: CGFloat = reactionPillHeight - reactionPillOverlap
}

func irisGroupSenderNameColorHex(for senderKey: String, isDarkMode: Bool) -> UInt32 {
    let values = isDarkMode
        ? irisGroupSenderNameDarkColorHexes
        : irisGroupSenderNameLightColorHexes
    let normalized = senderKey
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .lowercased()
    guard !normalized.isEmpty else {
        return values[0]
    }

    var hash: UInt64 = 0xcbf29ce484222325
    for byte in normalized.utf8 {
        hash ^= UInt64(byte)
        hash = hash &* 0x100000001b3
    }
    return values[Int(hash % UInt64(values.count))]
}

private func irisGroupSenderNameColor(for senderKey: String, isDarkMode: Bool) -> Color {
    irisColor(hex: irisGroupSenderNameColorHex(for: senderKey, isDarkMode: isDarkMode))
}

private func irisColor(hex: UInt32) -> Color {
    Color(
        red: Double((hex >> 16) & 0xff) / 255.0,
        green: Double((hex >> 8) & 0xff) / 255.0,
        blue: Double(hex & 0xff) / 255.0
    )
}

// Signal-iOS GroupNameColors, trimmed to avoid reusing the Iris brand
// purple for sender labels. We still keep Signal's high-contrast spread
// across blues, greens, teals, reds, oranges, yellows, and slate.
private let irisGroupSenderNameLightColorHexes: [UInt32] = [
    0x006DA3, 0x067906, 0xC13215, 0x5B6976, 0x2E51FF,
    0x007575, 0x9C5711, 0x3D7406, 0xD00B0B, 0x007A3D,
    0x866118, 0x067953, 0x4B7000, 0xB34209, 0x06792D,
    0x6B6B24, 0xD00B2C, 0x2D7906, 0x32763E, 0x2662D9,
    0x76681E, 0x067462, 0x5E6E0C, 0x077288, 0x2D761E,
]

private let irisGroupSenderNameDarkColorHexes: [UInt32] = [
    0x00A7FA, 0x0AB80A, 0xFF6F52, 0x8BA1B6, 0x8599FF,
    0x00B2B2, 0xD5920B, 0x5EB309, 0xFF7070, 0x00B85C,
    0xD68F00, 0x00B87A, 0x74AD00, 0xF57A3D, 0x0AB844,
    0xA4A437, 0xF77389, 0x42B309, 0x4BAF5C, 0x7DA1E8,
    0xB89B0A, 0x09B397, 0x8FAA09, 0x00AED1, 0x43B42D,
]

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

private func irisIsIncomingGroupUserMessage(_ message: ChatMessageSnapshot, chatKind: ChatKind) -> Bool {
    chatKind == .group && message.kind == .user && !message.isOutgoing
}

func irisShowsGroupSenderName(
    previous: ChatMessageSnapshot?,
    message: ChatMessageSnapshot,
    chatKind: ChatKind
) -> Bool {
    guard irisIsIncomingGroupUserMessage(message, chatKind: chatKind) else {
        return false
    }
    guard let previous,
          irisIsIncomingGroupUserMessage(previous, chatKind: chatKind),
          irisSameTimelineDay(previous.createdAtSecs, message.createdAtSecs) else {
        return true
    }
    return previous.author != message.author
}

func irisShowsGroupSenderAvatar(
    message: ChatMessageSnapshot,
    next: ChatMessageSnapshot?,
    chatKind: ChatKind
) -> Bool {
    guard irisIsIncomingGroupUserMessage(message, chatKind: chatKind) else {
        return false
    }
    guard let next,
          irisIsIncomingGroupUserMessage(next, chatKind: chatKind),
          irisSameTimelineDay(message.createdAtSecs, next.createdAtSecs) else {
        return true
    }
    return message.author != next.author
}

private enum ChatTimelineCoordinateSpace {
    static let name = "chatTimelineCoordinateSpace"
}

private enum ChatTimelineAnchor {
    static let top = "chatTimelineTop"
    static let bottom = "chatTimelineBottom"
}

private struct ChatJumpToBottomButton: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
#if os(iOS)
        label
            .accessibilityHidden(true)
            .overlay {
                TouchDownControl(
                    accessibilityIdentifier: "chatJumpToBottom",
                    accessibilityLabel: "Jump to latest",
                    action: action
                )
            }
#else
        Button(action: action) {
            label
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("chatJumpToBottom")
#endif
    }

    private var label: some View {
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
}

private struct ActiveMessageBubbleSwipe {
    let messageId: String
    var offset: CGFloat
    var hasFedHaptic: Bool
}

private enum ChatMessageBubbleSwipeMetrics {
    static let threshold: CGFloat = 60
    static let maxOffset: CGFloat = 90
    static let activationDistance: CGFloat = 12
}

private final class ChatTimelineInteractionCoordinator: ObservableObject {
#if os(iOS)
    weak var scrollView: UIScrollView?
#endif
    var messageBubbleFrames: [String: CGRect] = [:]
    var bubblePanRejected = false

    func stopScrolling() {
#if os(iOS)
        guard let scrollView else { return }
        scrollView.layer.removeAllAnimations()
        scrollView.setContentOffset(scrollView.contentOffset, animated: false)
#endif
    }

    func messageBubbleId(at location: CGPoint) -> String? {
        messageBubbleFrames
            .filter { _, frame in frame.insetBy(dx: -10, dy: -8).contains(location) }
            .min { lhs, rhs in lhs.value.midY < rhs.value.midY }
            .map(\.key)
    }
}

private struct ChatTimelineViewportMinYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private struct ChatTimelineViewportMaxYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private struct ChatTimelineTopMinYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = -.greatestFiniteMagnitude

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

private struct ChatMessageBubbleFramePreferenceKey: PreferenceKey {
    static var defaultValue: [String: CGRect] = [:]

    static func reduce(value: inout [String: CGRect], nextValue: () -> [String: CGRect]) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}

struct ChatTimelineDaySeparatorFrame: Equatable {
    let messageId: String
    let text: String
    let frame: CGRect
}

struct ChatFloatingDaySeparator: Equatable {
    let messageId: String
    let text: String
    let offsetY: CGFloat
}

func irisFloatingDaySeparator(
    frames: [ChatTimelineDaySeparatorFrame],
    viewportMinY: CGFloat,
    stickyTopY: CGFloat
) -> ChatFloatingDaySeparator? {
    let sortedFrames = frames.sorted { lhs, rhs in
        lhs.frame.minY < rhs.frame.minY
    }
    guard !sortedFrames.isEmpty else {
        return nil
    }

    let currentIndex: Int
    let next: ChatTimelineDaySeparatorFrame?
    if let firstInOrBelowViewport = sortedFrames.firstIndex(where: { $0.frame.minY >= stickyTopY }) {
        guard firstInOrBelowViewport > sortedFrames.startIndex else {
            return nil
        }
        currentIndex = sortedFrames.index(before: firstInOrBelowViewport)
        next = sortedFrames[firstInOrBelowViewport]
    } else {
        currentIndex = sortedFrames.index(before: sortedFrames.endIndex)
        next = nil
    }

    let current = sortedFrames[currentIndex]
    let currentHeight = max(current.frame.height, SignalConversationLayout.daySeparatorMinimumHeight)
    let stickyOffsetY = stickyTopY - viewportMinY
    var offsetY = stickyOffsetY
    if let next {
        let nextOffset = next.frame.minY - viewportMinY
        let maxOffsetY = nextOffset - currentHeight - SignalConversationLayout.stickyDateHeaderMinimumSpacing
        offsetY = min(offsetY, maxOffsetY)
    }
    return ChatFloatingDaySeparator(
        messageId: current.messageId,
        text: current.text,
        offsetY: offsetY
    )
}

private struct ChatTimelineDaySeparatorFramePreferenceKey: PreferenceKey {
    static var defaultValue: [String: ChatTimelineDaySeparatorFrame] = [:]

    static func reduce(
        value: inout [String: ChatTimelineDaySeparatorFrame],
        nextValue: () -> [String: ChatTimelineDaySeparatorFrame]
    ) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
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
            && lhs.hidesInlineDayChip == rhs.hidesInlineDayChip
            && lhs.isFirstInCluster == rhs.isFirstInCluster
            && lhs.isLastInCluster == rhs.isLastInCluster
            && lhs.showsGroupSenderName == rhs.showsGroupSenderName
            && lhs.showsGroupSenderAvatar == rhs.showsGroupSenderAvatar
            && lhs.swipeOffset == rhs.swipeOffset
    }

    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let chatKind: ChatKind
    let showDayChip: Bool
    let hidesInlineDayChip: Bool
    let isFirstInCluster: Bool
    let isLastInCluster: Bool
    let showsGroupSenderName: Bool
    let showsGroupSenderAvatar: Bool
    let reactions: [MessageReactionSnapshot]
    let swipeOffset: CGFloat
    let onReply: () -> Void
    let onForward: () -> Void
    let onForwardAttachment: (MessageAttachmentSnapshot) -> Void
    let onReact: (String) -> Void
    let onInfo: () -> Void
    let onDelete: () -> Void
    let onScrollToQuote: (ReplyPreview) -> Void
    let onShowReactors: () -> Void
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void

    @State private var isHovering = false
    @State private var showReactionPicker = false
    @State private var showActionsSheet = false

    private var showActionDock: Bool {
        IrisLayout.usesDesktopChrome && isHovering
    }

    private var postReactionSuggestions: [String] {
        irisPostReactionSuggestionEmojis(reactions)
    }

    @ViewBuilder
    private func actionDock() -> some View {
        ChatMessageActionDock(
            onShowReactionPicker: { showReactionPicker = true },
            onReply: onReply,
            onForward: onForward,
            onCopy: {
                PlatformClipboard.setString(copyableMessageText(message))
            },
            onInfo: onInfo,
            onDelete: onDelete
        )
    }

    var body: some View {
        // Hoist a couple of computed values that are read 3-4 times in this
        // body so we don't pay for parsing/struct construction on every
        // access. SwiftUI re-evaluates body whenever the parent ChatScreen
        // re-runs, which happens on any AppManager publish.
        let parsed = parseReplyEncodedMessage(message.body)
        let dayText = irisTimelineDay(message.createdAtSecs)
        let bubble = ChatMessageBubbleShape(
            isOutgoing: message.isOutgoing,
            isFirstInCluster: isFirstInCluster,
            isLastInCluster: isLastInCluster
        )
        return VStack(spacing: 0) {
            if showDayChip {
                HStack {
                    Spacer()
                    IrisInlineDaySeparator(text: dayText)
                        .background(
                            GeometryReader { geometry in
                                Color.clear.preference(
                                    key: ChatTimelineDaySeparatorFramePreferenceKey.self,
                                    value: [
                                        message.id: ChatTimelineDaySeparatorFrame(
                                            messageId: message.id,
                                            text: dayText,
                                            frame: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name))
                                        )
                                    ]
                                )
                            }
                        )
                        .accessibilityIdentifier("chatInlineDaySeparator-\(message.id)")
                    Spacer()
                }
                .padding(.top, SignalConversationLayout.daySeparatorTopPadding)
                .padding(.bottom, SignalConversationLayout.daySeparatorBottomPadding)
                .opacity(hidesInlineDayChip ? 0 : 1)
                .accessibilityHidden(hidesInlineDayChip)
            }

            if message.kind == .system {
                HStack {
                    Spacer(minLength: 24)
                    Text(message.body)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .multilineTextAlignment(.center)
                        .irisDesktopTextSelection()
                        .padding(.horizontal, 12)
                        .padding(.vertical, 7)
                        .background(Capsule(style: .continuous).fill(palette.panel.opacity(0.74)))
                    Spacer(minLength: 24)
                }
                .padding(.vertical, 8)
                .padding(.top, showDayChip ? 0 : SignalConversationLayout.systemMessageSpacing)
                .accessibilityIdentifier("chatSystemMessage-\(message.id)")
            } else {
                VStack(
                    alignment: message.isOutgoing ? .trailing : .leading,
                    spacing: 0
                ) {
                    HStack(alignment: .bottom, spacing: SignalConversationLayout.messageStackSpacing) {
                        if message.isOutgoing {
                            Spacer(minLength: SignalConversationLayout.messageDirectionSpacing)
                        } else if chatKind == .group {
                            groupSenderAvatar
                        }

                        if message.isOutgoing {
                            desktopActionDockSlot
                        }

                        VStack(alignment: message.isOutgoing ? .trailing : .leading, spacing: 4) {
                            if showsGroupSenderName {
                                Text(message.author)
                                    .font(.system(.footnote, design: .rounded, weight: .semibold))
                                    .foregroundStyle(irisGroupSenderNameColor(
                                        for: message.author,
                                        isDarkMode: colorScheme == .dark
                                    ))
                                    .lineLimit(1)
                            }

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
                                        foreground: message.isOutgoing
                                            ? palette.onBubbleMine
                                            : palette.onBubbleTheirs
                                    ),
                                    isOutgoing: message.isOutgoing,
                                    bodyFont: irisMessageBodyFont(for: parsed.body)
                                )
                            }
                            let imageAttachments = message.attachments.filter { $0.isImage }
                            let nonImageAttachments = message.attachments.filter { !$0.isImage }
                            if !imageAttachments.isEmpty {
                                ChatImageAlbumView(
                                    attachments: imageAttachments,
                                    isOutgoing: message.isOutgoing,
                                    downloadAttachment: downloadAttachment,
                                    onOpenImage: onOpenImage,
                                    onForward: { attachment in
                                        onForwardAttachment(attachment)
                                    }
                                )
                            }
                            ForEach(Array(nonImageAttachments.enumerated()), id: \.offset) { _, attachment in
                                ChatAttachmentView(
                                    attachment: attachment,
                                    isOutgoing: message.isOutgoing,
                                    downloadAttachment: downloadAttachment,
                                    openAttachment: openAttachment,
                                    onOpenImage: onOpenImage,
                                    onForward: {
                                        onForwardAttachment(attachment)
                                    }
                                )
                            }
                            if isLastInCluster {
                                // Footer inherits the bubble VStack's
                                // alignment (.trailing for outgoing,
                                // .leading for incoming). No frame /
                                // Spacer here on purpose — both pull the
                                // bubble wider than its content. Footer
                                // alignment for incoming bubbles is
                                // Signal-ish-but-leading on iOS.
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
                        .padding(.horizontal, SignalConversationLayout.textInsetHorizontal)
                        .padding(.vertical, SignalConversationLayout.textInsetVertical)
                        .background(
                            bubble
                                .fill(message.isOutgoing ? palette.bubbleMine : palette.bubbleTheirs)
                        )
                        .clipShape(bubble)
                        .contentShape(bubble)
                        // Long-press -> actions sheet is an iOS-only
                        // gesture (desktop has the hover-revealed action
                        // dock instead). Attaching it on macOS, even
                        // with a no-op closure, captures the mouse
                        // press, which swallows AttributedString link
                        // clicks and prevents SwiftUI from drawing the
                        // pointing-hand cursor over URL spans. Gating it
                        // here is the actual fix for "link clicks dead +
                        // no link cursor" on the macOS chat view.
#if os(iOS)
                        .onLongPressGesture(minimumDuration: 0.4) {
                            if !IrisLayout.usesDesktopChrome {
                                PlatformHaptics.messageMenuOpened()
                                showActionsSheet = true
                            }
                        }
#endif
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
                                onForward: {
                                    showActionsSheet = false
                                    onForward()
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
                            .irisModalSurface()
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
                            .irisModalSurface()
                            .presentationDetents([.medium, .large])
                            .presentationDragIndicator(.visible)
                            .irisDismissOnMacOutsideClick { showReactionPicker = false }
                        }
                        .accessibilityIdentifier("chatMessage-\(message.id)")
                        .background(
                            GeometryReader { geometry in
                                Color.clear.preference(
                                    key: ChatMessageBubbleFramePreferenceKey.self,
                                    value: [
                                        message.id: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name))
                                    ]
                                )
                            }
                        )
                        .padding(.bottom, reactions.isEmpty ? 0 : SignalConversationLayout.reactionPillProtrusion)
                        .overlay(alignment: message.isOutgoing ? .bottomLeading : .bottomTrailing) {
                            if !reactions.isEmpty {
                                ReactionRow(
                                    reactions: reactions,
                                    onTap: onShowReactors
                                )
                                .fixedSize()
                                .offset(
                                    x: message.isOutgoing
                                        ? SignalConversationLayout.reactionPillHorizontalInset
                                        : -SignalConversationLayout.reactionPillHorizontalInset
                                )
                            }
                        }
#if canImport(AppKit)
                        // Cap bubble width on macOS; iOS has no hover and
                        // phone widths self-limit.
                        .frame(
                            maxWidth: IrisLayout.chatBubbleMaxWidth,
                            alignment: message.isOutgoing ? .trailing : .leading
                        )
#endif
                        // The actual pan is owned by the timeline scroll view.
                        // This modifier only renders the offset/reveal state,
                        // keeping vertical flicks on bubbles in the scroll path.
                        .applyMessageBubbleSwipe(offset: swipeOffset)

                        if !message.isOutgoing {
                            desktopActionDockSlot
                        }

                        if !message.isOutgoing {
                            Spacer(minLength: SignalConversationLayout.messageDirectionSpacing)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: message.isOutgoing ? .trailing : .leading)
                .contentShape(Rectangle())
                .onHover { isHovering = $0 }
                .padding(.top, rowTopSpacing)
            }
        }
    }

    private var rowTopSpacing: CGFloat {
        if showDayChip { return 0 }
        if message.kind == .system { return SignalConversationLayout.systemMessageSpacing }
        return isFirstInCluster
            ? SignalConversationLayout.defaultMessageSpacing
            : SignalConversationLayout.compactMessageSpacing
    }

    @ViewBuilder
    private var desktopActionDockSlot: some View {
#if canImport(AppKit)
        actionDock()
            .fixedSize()
            .opacity(showActionDock ? 1 : 0)
            .allowsHitTesting(showActionDock)
#else
        EmptyView()
#endif
    }

    @ViewBuilder
    private var groupSenderAvatar: some View {
        if showsGroupSenderAvatar {
            IrisAvatar(
                label: message.author,
                size: SignalConversationLayout.groupMessageAvatarSize
            )
            .accessibilityHidden(true)
        } else {
            Color.clear
                .frame(
                    width: SignalConversationLayout.groupMessageAvatarSize,
                    height: SignalConversationLayout.groupMessageAvatarSize
                )
                .accessibilityHidden(true)
        }
    }
}

// Caps tall message bubbles behind a Show more/less toggle.
// Mirrors the Android implementation: lineLimit caps the visible
// lines, and the toggle only appears when newline count or
// character count crosses the same thresholds Android uses. The
// previous ViewThatFits-with-outer-.frame approach worked on
// paper but in practice SwiftUI ended up promoting the
// .frame(maxHeight:) proposal into a force — short messages got
// rendered as half-screen-tall bubbles. Caught by the
// `single-line bubble height` UI assertion.
private struct TruncatableMessageBody: View {
    let attributed: AttributedString
    let isOutgoing: Bool
    let bodyFont: Font
    @Environment(\.irisPalette) private var palette
    @State private var isExpanded = false

    private let collapsedLineLimit = 14
    private let longBodyCharThreshold = 800

    private var needsTruncation: Bool {
        let plain = String(attributed.characters)
        if plain.count > longBodyCharThreshold { return true }
        let newlines = plain.reduce(into: 0) { count, ch in
            if ch == "\n" { count += 1 }
        }
        return newlines >= collapsedLineLimit
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(attributed)
                .font(bodyFont)
                .multilineTextAlignment(.leading)
                .lineLimit(isExpanded ? nil : collapsedLineLimit)
                .fixedSize(horizontal: false, vertical: true)
                .irisDesktopTextSelection()
            if needsTruncation {
                toggleButton(label: isExpanded ? "Show less" : "Show more")
            }
        }
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

private extension View {
    @ViewBuilder
    func irisDesktopTextSelection() -> some View {
#if canImport(AppKit)
        textSelection(.enabled)
#else
        self
#endif
    }
}

private func irisMessageBodyFont(for text: String) -> Font {
    switch irisJumbomojiCount(text) {
    case 1:
        return .system(size: 56, weight: .regular, design: .rounded)
    case 2:
        return .system(size: 48, weight: .regular, design: .rounded)
    case 3:
        return .system(size: 40, weight: .regular, design: .rounded)
    case 4:
        return .system(size: 36, weight: .regular, design: .rounded)
    case 5:
        return .system(size: 32, weight: .regular, design: .rounded)
    default:
        return .system(.body, design: .rounded)
    }
}

func irisJumbomojiCount(_ text: String) -> Int {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return 0 }

    var count = 0
    for character in trimmed {
        if character.unicodeScalars.allSatisfy({ CharacterSet.whitespacesAndNewlines.contains($0) }) {
            continue
        }
        guard irisIsEmojiCluster(character) else {
            return 0
        }
        count += 1
        if count > 5 {
            return 0
        }
    }
    return count
}

private func irisIsEmojiCluster(_ character: Character) -> Bool {
    var hasEmojiBase = false
    for scalar in character.unicodeScalars {
        let value = scalar.value
        if value == 0x200D || value == 0xFE0F || (0x1F3FB...0x1F3FF).contains(value) {
            continue
        }
        if (0x1F000...0x1FAFF).contains(value) || (0x2600...0x27BF).contains(value) {
            hasEmojiBase = true
            continue
        }
        return false
    }
    return hasEmojiBase
}

private struct MessageInfoSheet: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let chat: CurrentChatSnapshot?
    @ObservedObject var manager: AppManager
    let onClose: () -> Void

    private func messageAuthorInfo() -> ParticipantInfo {
        let owner = message.authorOwnerPubkeyHex?.isEmpty == false
            ? message.authorOwnerPubkeyHex
            : ((!message.isOutgoing && chat?.kind == .direct) ? chat?.chatId : nil)
        return participantInfo(
            ownerPubkeyHex: owner,
            displayName: message.author,
            pictureUrl: message.authorPictureUrl,
            chat: chat
        )
    }

    private func openPerson(_ info: ParticipantInfo) {
        guard let owner = info.ownerPubkeyHex, !owner.isEmpty, !info.isMe else { return }
        onClose()
        manager.dispatch(.createChat(peerInput: owner))
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
        .irisModalSurface()
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
                    if let chat, chat.kind == .direct {
                        MessageInfoRecipientRow(
                            info: directRecipientInfo(chat),
                            subtitle: "No receipt",
                            delivery: message.delivery,
                            manager: manager,
                            onTap: openPerson
                        )
                    } else {
                        MessageInfoValueRow(label: "Recipients", value: "No receipts")
                    }
                } else {
                    ForEach(message.recipientDeliveries, id: \.ownerPubkeyHex) { recipient in
                        MessageInfoRecipientRow(
                            info: recipientInfo(recipient, chat: chat),
                            subtitle: messageInfoDateTime(recipient.updatedAtSecs),
                            delivery: recipient.delivery,
                            manager: manager,
                            onTap: openPerson
                        )
                    }
                }
            } else {
                MessageInfoRecipientRow(
                    info: messageAuthorInfo(),
                    subtitle: messageInfoDateTime(message.createdAtSecs),
                    delivery: message.delivery,
                    manager: manager,
                    onTap: openPerson
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
                        info: reactorInfo(reactor, chat: chat),
                        emoji: reactor.emoji,
                        manager: manager,
                        onTap: openPerson
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
    let ownerPubkeyHex: String?
    let name: String
    let pictureUrl: String?
    let isMe: Bool
}

private func participantInfo(
    ownerPubkeyHex: String?,
    displayName: String,
    pictureUrl: String?,
    chat: CurrentChatSnapshot?
) -> ParticipantInfo {
    let owner = ownerPubkeyHex?.trimmingCharacters(in: .whitespacesAndNewlines)
    let participant = owner.flatMap { owner in
        chat?.participants.first { $0.ownerPubkeyHex == owner }
    }
    let name = participant?.displayName
        ?? nonEmptyTrimmed(displayName)
        ?? "Iris user"
    return ParticipantInfo(
        ownerPubkeyHex: owner?.isEmpty == false ? owner : nil,
        name: name,
        pictureUrl: participant?.pictureUrl ?? pictureUrl,
        isMe: participant?.isLocalOwner ?? false
    )
}

private func recipientInfo(
    _ recipient: MessageRecipientDeliverySnapshot,
    chat: CurrentChatSnapshot?
) -> ParticipantInfo {
    participantInfo(
        ownerPubkeyHex: recipient.ownerPubkeyHex,
        displayName: recipient.displayName,
        pictureUrl: recipient.pictureUrl,
        chat: chat
    )
}

private func reactorInfo(_ reactor: MessageReactor, chat: CurrentChatSnapshot?) -> ParticipantInfo {
    participantInfo(
        ownerPubkeyHex: reactor.author,
        displayName: reactor.displayName,
        pictureUrl: reactor.pictureUrl,
        chat: chat
    )
}

private func directRecipientInfo(_ chat: CurrentChatSnapshot) -> ParticipantInfo {
    participantInfo(
        ownerPubkeyHex: chat.chatId,
        displayName: chat.displayName,
        pictureUrl: chat.pictureUrl,
        chat: chat
    )
}

private func nonEmptyTrimmed(_ value: String) -> String? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}

private struct MessageInfoReactorRow: View {
    @Environment(\.irisPalette) private var palette
    let info: ParticipantInfo
    let emoji: String
    @ObservedObject var manager: AppManager
    let onTap: (ParticipantInfo) -> Void

    var body: some View {
        MessageInfoUserRow(
            info: info,
            subtitle: nil,
            manager: manager,
            onTap: onTap
        ) {
            Text(emoji.isEmpty ? "Removed" : emoji)
                .font(emoji.isEmpty ? .system(.caption, design: .rounded, weight: .medium) : .system(size: 22))
                .foregroundStyle(emoji.isEmpty ? palette.muted : palette.textPrimary)
        }
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
    let info: ParticipantInfo
    let subtitle: String
    let delivery: DeliveryState
    @ObservedObject var manager: AppManager
    let onTap: (ParticipantInfo) -> Void

    var body: some View {
        MessageInfoUserRow(
            info: info,
            subtitle: "\(irisDeliveryLabel(delivery)) - \(subtitle)",
            manager: manager,
            onTap: onTap
        ) {
            IrisDeliveryGlyph(delivery: delivery)
                .frame(width: 18, height: 18)
        }
    }
}

private struct MessageInfoUserRow<Trailing: View>: View {
    @Environment(\.irisPalette) private var palette
    let info: ParticipantInfo
    let subtitle: String?
    @ObservedObject var manager: AppManager
    let onTap: (ParticipantInfo) -> Void
    let trailing: () -> Trailing

    init(
        info: ParticipantInfo,
        subtitle: String?,
        manager: AppManager,
        onTap: @escaping (ParticipantInfo) -> Void,
        @ViewBuilder trailing: @escaping () -> Trailing
    ) {
        self.info = info
        self.subtitle = subtitle
        self.manager = manager
        self.onTap = onTap
        self.trailing = trailing
    }

    var body: some View {
        if info.ownerPubkeyHex != nil && !info.isMe {
            Button {
                onTap(info)
            } label: {
                rowContent
            }
            .buttonStyle(.irisPlain)
        } else {
            rowContent
        }
    }

    private var rowContent: some View {
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
                    .lineLimit(1)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.system(.caption, design: .rounded, weight: .medium))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            trailing()
        }
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ChatMessageBubbleShape: Shape {
    let isOutgoing: Bool
    let isFirstInCluster: Bool
    let isLastInCluster: Bool

    func path(in rect: CGRect) -> Path {
        let large = SignalConversationLayout.bubbleWideCornerRadius
        let tail = SignalConversationLayout.bubbleSharpCornerRadius
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
    let onForward: () -> Void
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
                Button("Forward", action: onForward)
                Button("Copy text", action: onCopy)
                Button("Info", action: onInfo)
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
    irisUniqueEmojis(reactions.map(\.emoji))
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

func irisReactionQuickChoices() -> [String] {
    quickReactionEmojis
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
    let onForward: () -> Void
    let onCopy: () -> Void
    let onInfo: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            quickReactionRow
            previewCard
            VStack(spacing: 0) {
                actionRow(icon: "arrowshape.turn.up.left", label: "Reply", action: onReply)
                actionRow(icon: "arrowshape.turn.up.right", label: "Forward", action: onForward)
                actionRow(icon: "doc.on.doc", label: "Copy", action: onCopy)
                actionRow(icon: "info.circle", label: "Info", action: onInfo)
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
            ForEach(irisReactionQuickChoices(), id: \.self) { emoji in
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
                    ReactionRow(reactions: message.reactions)
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
                sections.append(("This message", "bubble.left.and.bubble.right.fill", postEmojis))
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
                            info: reactorInfo(reactor, chat: chat),
                            emoji: reactor.emoji,
                            manager: manager,
                            onTap: openPerson
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
                    IrisModalCloseButton(action: onClose)
                        .accessibilityIdentifier("messageReactorsCloseButton")
                }
            }
#elseif os(macOS)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    IrisModalCloseButton(action: onClose)
                }
            }
#endif
        }
        .accessibilityIdentifier("messageReactorsSheet")
        .irisModalSurface()
    }

    private func openPerson(_ info: ParticipantInfo) {
        guard let owner = info.ownerPubkeyHex, !owner.isEmpty, !info.isMe else { return }
        onClose()
        manager.dispatch(.createChat(peerInput: owner))
    }
}

private struct ReactionRow: View {
    @Environment(\.irisPalette) private var palette
    let reactions: [MessageReactionSnapshot]
    var onTap: (() -> Void)? = nil

    @ViewBuilder
    var body: some View {
        let pills = HStack(spacing: 0) {
            ForEach(reactions, id: \.emoji) { reaction in
                HStack(spacing: 2) {
                    Text(reaction.emoji)
                        .font(.system(size: 14, weight: .bold))
                    if reaction.count > 1 {
                        Text("\(reaction.count)")
                            .font(.system(size: 12, weight: .bold, design: .monospaced))
                            .foregroundStyle(palette.muted)
                    }
                }
                .padding(.horizontal, 7)
                .frame(height: SignalConversationLayout.reactionPillHeight)
                .background(
                    Capsule(style: .continuous)
                        .fill(reaction.reactedByMe ? palette.panel : palette.panelAlt)
                )
                .overlay(
                    Capsule(style: .continuous)
                        .strokeBorder(palette.background, lineWidth: 1)
                )
            }
        }

        if let onTap {
            Button(action: onTap) { pills }
                .buttonStyle(.irisPlain)
                .accessibilityHint("Tap to see who reacted")
                .accessibilityIdentifier("chatReactionRow")
        } else {
            pills
                .accessibilityIdentifier("chatReactionRow")
        }
    }
}

extension View {
    @ViewBuilder
    fileprivate func observeChatTimelineScroll(
        coordinator: ChatTimelineInteractionCoordinator,
        onPan: @escaping (CGFloat, CGFloat) -> Void
    ) -> some View {
#if os(iOS)
        background(
            ChatTimelineScrollObserver(
                timelineCoordinator: coordinator,
                onPan: onPan
            )
            .frame(width: 0, height: 0)
            .allowsHitTesting(false)
        )
#else
        self
#endif
    }

    @ViewBuilder
    func applyMessageBubbleSwipe(offset: CGFloat) -> some View {
#if os(iOS)
        modifier(MessageBubbleSwipeActions(offset: offset))
#else
        self
#endif
    }
}

#if os(iOS)
private struct TouchDownControl: UIViewRepresentable {
    let accessibilityIdentifier: String
    let accessibilityLabel: String
    let action: () -> Void

    func makeUIView(context: Context) -> TouchDownControlView {
        let view = TouchDownControlView()
        view.backgroundColor = .clear
        view.isAccessibilityElement = true
        view.accessibilityTraits = [.button]
        view.accessibilityIdentifier = accessibilityIdentifier
        view.accessibilityLabel = accessibilityLabel
        view.onTouchDown = action
        return view
    }

    func updateUIView(_ uiView: TouchDownControlView, context: Context) {
        uiView.accessibilityIdentifier = accessibilityIdentifier
        uiView.accessibilityLabel = accessibilityLabel
        uiView.onTouchDown = action
    }
}

private final class TouchDownControlView: UIControl {
    var onTouchDown: (() -> Void)?
    private var lastFireAt: CFTimeInterval = 0
    private weak var observedWindow: UIWindow?
    private var windowRecognizer: WindowTouchDownGestureRecognizer?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        updateWindowRecognizer()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        updateWindowRecognizer()
    }

    deinit {
        if let windowRecognizer, let observedWindow {
            observedWindow.removeGestureRecognizer(windowRecognizer)
        }
    }

    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        fireOnce()
        return true
    }

    override func continueTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        true
    }

    func fireOnce() {
        let now = CACurrentMediaTime()
        guard now - lastFireAt > 0.15 else { return }
        lastFireAt = now
        onTouchDown?()
    }

    private func updateWindowRecognizer() {
        if observedWindow === window {
            windowRecognizer?.targetView = self
            return
        }
        if let windowRecognizer, let observedWindow {
            observedWindow.removeGestureRecognizer(windowRecognizer)
        }
        observedWindow = window
        guard let window else {
            windowRecognizer = nil
            return
        }
        // UIScrollView can consume the first touch while decelerating.
        // Watching the window lets the jump button interrupt that coast.
        let recognizer = WindowTouchDownGestureRecognizer()
        recognizer.targetView = self
        recognizer.cancelsTouchesInView = false
        recognizer.delaysTouchesBegan = false
        recognizer.delaysTouchesEnded = false
        window.addGestureRecognizer(recognizer)
        windowRecognizer = recognizer
    }
}

private final class WindowTouchDownGestureRecognizer: UIGestureRecognizer, UIGestureRecognizerDelegate {
    weak var targetView: TouchDownControlView?

    override init(target: Any?, action: Selector?) {
        super.init(target: target, action: action)
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let touch = touches.first,
              let window = view,
              let targetView,
              !targetView.isHidden,
              targetView.alpha > 0.01 else {
            state = .failed
            return
        }
        let frame = targetView.convert(targetView.bounds, to: window).insetBy(dx: -8, dy: -8)
        if frame.contains(touch.location(in: window)) {
            targetView.fireOnce()
            state = .recognized
        } else {
            state = .failed
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible {
            state = .failed
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible {
            state = .failed
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        state = .cancelled
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        true
    }
}

private struct ChatTimelineScrollObserver: UIViewRepresentable {
    let timelineCoordinator: ChatTimelineInteractionCoordinator
    let onPan: (CGFloat, CGFloat) -> Void

    func makeUIView(context: Context) -> ChatTimelineScrollObserverView {
        let view = ChatTimelineScrollObserverView()
        view.timelineCoordinator = timelineCoordinator
        view.onPan = onPan
        return view
    }

    func updateUIView(_ uiView: ChatTimelineScrollObserverView, context: Context) {
        uiView.timelineCoordinator = timelineCoordinator
        uiView.onPan = onPan
        uiView.bindToEnclosingScrollView()
    }

    static func dismantleUIView(_ uiView: ChatTimelineScrollObserverView, coordinator: ()) {
        uiView.unbind()
    }
}

private final class ChatTimelineScrollObserverView: UIView {
    weak var timelineCoordinator: ChatTimelineInteractionCoordinator?
    var onPan: ((CGFloat, CGFloat) -> Void)?

    private weak var observedScrollView: UIScrollView?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        DispatchQueue.main.async { [weak self] in
            self?.bindToEnclosingScrollView()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        bindToEnclosingScrollView()
    }

    func bindToEnclosingScrollView() {
        guard let scrollView = enclosingScrollView() else { return }
        if observedScrollView === scrollView {
            timelineCoordinator?.scrollView = scrollView
            return
        }

        unbind()
        observedScrollView = scrollView
        timelineCoordinator?.scrollView = scrollView
        scrollView.panGestureRecognizer.addTarget(self, action: #selector(handleScrollPan(_:)))
    }

    func unbind() {
        if let scrollView = observedScrollView {
            scrollView.panGestureRecognizer.removeTarget(self, action: #selector(handleScrollPan(_:)))
        }
        if timelineCoordinator?.scrollView === observedScrollView {
            timelineCoordinator?.scrollView = nil
        }
        observedScrollView = nil
    }

    @objc private func handleScrollPan(_ recognizer: UIPanGestureRecognizer) {
        guard let scrollView = observedScrollView else { return }
        let translation = recognizer.translation(in: scrollView)
        switch recognizer.state {
        case .began, .changed:
            let velocity = recognizer.velocity(in: scrollView)
            onPan?(translation.y, velocity.y)
        default:
            break
        }
    }

    private func enclosingScrollView() -> UIScrollView? {
        var view: UIView? = self
        while let current = view {
            if let scrollView = current as? UIScrollView {
                return scrollView
            }
            view = current.superview
        }
        return nil
    }
}

private struct MessageBubbleSwipeActions: ViewModifier {
    let offset: CGFloat

    func body(content: Content) -> some View {
        content
            .offset(x: offset)
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
    }

    private func reveal(forwards: Bool) -> Double {
        let v = forwards ? offset : -offset
        return Double(min(1, max(0, v / ChatMessageBubbleSwipeMetrics.threshold)))
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

    private var authorName: String {
        message.isOutgoing ? "You" : message.author
    }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            HStack(spacing: 8) {
                Capsule()
                    .fill(palette.muted.opacity(0.55))
                    .frame(width: 4, height: 38)
                    .padding(.vertical, 8)

                VStack(alignment: .leading, spacing: 2) {
                    Text(authorName)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    Text(replySnippet(for: message))
                        .font(.system(.subheadline, design: .rounded, weight: .regular))
                        .foregroundStyle(palette.muted)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                }
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)

                if let attachment = message.attachments.first {
                    IrisReplyComposerAttachmentBadge(attachment: attachment)
                        .padding(.vertical, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            IrisReplyComposerCloseButton(action: onCancel)
                .padding(.top, 4)
        }
        .padding(.horizontal, 8)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(palette.panelAlt)
        )
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 14 : 8)
        .padding(.top, 6)
        .padding(.bottom, 2)
        .accessibilityIdentifier("chatReplyComposer")
    }
}

private struct IrisReplyComposerCloseButton: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ZStack {
                Circle()
                    .fill(palette.toolbar)
                    .frame(width: 24, height: 24)
                Image(systemName: "xmark")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
            }
            .frame(width: 32, height: 32)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Close")
        .accessibilityIdentifier("chatReplyCancelButton")
    }
}

private struct IrisReplyComposerAttachmentBadge: View {
    @Environment(\.irisPalette) private var palette
    let attachment: MessageAttachmentSnapshot

    var body: some View {
        let category = chatAttachmentCategory(for: attachment)
        Image(systemName: category.systemIcon)
            .font(.system(size: 18, weight: .semibold))
            .foregroundStyle(palette.muted)
            .frame(width: 46, height: 46)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(palette.toolbar)
            )
            .accessibilityLabel(category.rawValue)
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

private struct ChatImageAlbumView: View {
    let attachments: [MessageAttachmentSnapshot]
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void
    let onForward: (MessageAttachmentSnapshot) -> Void

    private let albumWidth: CGFloat = 232
    private let gap: CGFloat = 2

    var body: some View {
        switch attachments.count {
        case 0:
            EmptyView()
        case 1:
            cell(attachments[0], width: 220, height: 150, contentMode: .fill)
        case 2:
            HStack(spacing: gap) {
                cell(attachments[0], width: (albumWidth - gap) / 2, height: 150, contentMode: .fill)
                cell(attachments[1], width: (albumWidth - gap) / 2, height: 150, contentMode: .fill)
            }
        case 3:
            HStack(spacing: gap) {
                cell(attachments[0], width: albumWidth * 0.58 - gap / 2, height: albumWidth * 0.86, contentMode: .fill)
                VStack(spacing: gap) {
                    cell(attachments[1], width: albumWidth * 0.42 - gap / 2, height: (albumWidth * 0.86 - gap) / 2, contentMode: .fill)
                    cell(attachments[2], width: albumWidth * 0.42 - gap / 2, height: (albumWidth * 0.86 - gap) / 2, contentMode: .fill)
                }
            }
        default:
            let cellSize = (albumWidth - gap) / 2
            VStack(spacing: gap) {
                HStack(spacing: gap) {
                    cell(attachments[0], width: cellSize, height: cellSize, contentMode: .fill)
                    cell(attachments[1], width: cellSize, height: cellSize, contentMode: .fill)
                }
                HStack(spacing: gap) {
                    cell(attachments[2], width: cellSize, height: cellSize, contentMode: .fill)
                    overflowCell(at: 3, width: cellSize, height: cellSize)
                }
            }
        }
    }

    @ViewBuilder
    private func cell(
        _ attachment: MessageAttachmentSnapshot,
        width: CGFloat,
        height: CGFloat,
        contentMode: ContentMode
    ) -> some View {
        ChatAlbumImageCell(
            attachment: attachment,
            isOutgoing: isOutgoing,
            width: width,
            height: height,
            downloadAttachment: downloadAttachment,
            onOpenImage: onOpenImage,
            onForward: { onForward(attachment) }
        )
    }

    @ViewBuilder
    private func overflowCell(at index: Int, width: CGFloat, height: CGFloat) -> some View {
        ChatAlbumImageCell(
            attachment: attachments[index],
            isOutgoing: isOutgoing,
            width: width,
            height: height,
            downloadAttachment: downloadAttachment,
            onOpenImage: onOpenImage,
            onForward: { onForward(attachments[index]) }
        )
        .overlay {
            if attachments.count > 4 {
                ZStack {
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(Color.black.opacity(0.45))
                    Text("+\(attachments.count - 4)")
                        .font(.system(size: 24, weight: .bold, design: .rounded))
                        .foregroundStyle(.white)
                }
                .frame(width: width, height: height)
                .allowsHitTesting(false)
            }
        }
    }
}

private struct ChatAlbumImageCell: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let width: CGFloat
    let height: CGFloat
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void
    let onForward: () -> Void

    @State private var localImageData: Data?
    @State private var localPreviewImage: PlatformImage?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false

    var body: some View {
        ZStack {
            Rectangle()
                .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
            if let localPreviewImage {
                Image(platformImage: localPreviewImage)
                    .resizable()
                    .scaledToFill()
            } else if let localImageData, isAnimatedImage(data: localImageData, filename: attachment.filename) {
                IrisAnimatedImageDataView(data: localImageData)
                    .allowsHitTesting(false)
            } else if isLoadingImage {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: failedImageLoad ? "exclamationmark.triangle.fill" : "photo.fill")
                    .font(.system(size: 22, weight: .semibold))
                    .opacity(0.72)
            }
        }
        .frame(width: width, height: height)
        .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        .contentShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        .onTapGesture {
            if let localImageData {
                onOpenImage(localImageData, attachment)
            } else {
                Task {
                    await loadImageIfNeeded()
                    if let localImageData {
                        onOpenImage(localImageData, attachment)
                    }
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(attachment.filename)
        .contextMenu {
            Button("Forward", action: onForward)
            Button("Copy link") {
                PlatformClipboard.setString(attachment.htreeUrl)
            }
        }
        .task(id: attachment.htreeUrl) {
            await loadImageIfNeeded()
        }
    }

    @MainActor
    private func loadImageIfNeeded() async {
        guard localImageData == nil, !isLoadingImage else { return }
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
            if let preview = makeChatAttachmentPreviewImage(data: data, filename: attachment.filename) {
                ChatAttachmentPreviewImageCache.store(preview, for: attachment.htreeUrl)
                localPreviewImage = preview
            } else {
                isLoadingImage = false
                failedImageLoad = true
                return
            }
        }
        localImageData = data
        isLoadingImage = false
    }
}

private struct ChatAttachmentView: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void
    let onForward: () -> Void

    @State private var localImageData: Data?
    @State private var localPreviewImage: PlatformImage?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false
    @State private var isOpeningAttachment = false

    var body: some View {
        if attachment.isImage {
            ZStack {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                if let localPreviewImage {
                    Image(platformImage: localPreviewImage)
                        .resizable()
                        .scaledToFill()
                } else if let localImageData, isAnimatedImage(data: localImageData, filename: attachment.filename) {
                    IrisAnimatedImageDataView(data: localImageData)
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
            .frame(width: 220, height: 150)
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .onTapGesture {
                if let localImageData {
                    onOpenImage(localImageData, attachment)
                } else {
                    Task {
                        await loadImageIfNeeded()
                        if let localImageData {
                            onOpenImage(localImageData, attachment)
                        }
                    }
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(attachment.filename)
            .contextMenu {
                Button("Forward", action: onForward)
                Button("Copy link") {
                    PlatformClipboard.setString(attachment.htreeUrl)
                }
            }
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
                Button("Forward", action: onForward)
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
    let attachments: [MessageAttachmentSnapshot]
    let initialIndex: Int
    let initialData: Data
    let senderName: String
    let createdAtSecs: UInt64
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let forwardableTextFor: (MessageAttachmentSnapshot) -> String

    static func == (lhs: ImageViewerItem, rhs: ImageViewerItem) -> Bool {
        lhs.id == rhs.id
    }
}

private struct ChatImageViewerPresenter: ViewModifier {
    @Binding var item: ImageViewerItem?
    let onForwardText: (String) -> Void

    func body(content: Content) -> some View {
        #if os(iOS)
        content
            .fullScreenCover(item: $item) { viewerItem in
                IrisImageViewer(item: viewerItem, onForwardText: onForwardText) {
                    item = nil
                }
            }
        #else
        content
            .overlay {
                if let item {
                    IrisImageViewer(item: item, onForwardText: onForwardText) {
                        self.item = nil
                    }
                }
            }
        #endif
    }
}

private struct IrisImageViewer: View {
    let item: ImageViewerItem
    let onForwardText: (String) -> Void
    let onClose: () -> Void

    @State private var currentIndex: Int
    @State private var loadedData: [String: Data]
    @State private var loadedImages: [String: PlatformImage]
    @State private var sharedFileURL: URL?
    @State private var dragTranslation: CGFloat = 0

    init(item: ImageViewerItem, onForwardText: @escaping (String) -> Void, onClose: @escaping () -> Void) {
        self.item = item
        self.onForwardText = onForwardText
        self.onClose = onClose
        _currentIndex = State(initialValue: item.initialIndex)
        var initial: [String: Data] = [:]
        var initialImages: [String: PlatformImage] = [:]
        if item.attachments.indices.contains(item.initialIndex) {
            let attachment = item.attachments[item.initialIndex]
            initial[attachment.htreeUrl] = item.initialData
            if !isAnimatedImage(data: item.initialData, filename: attachment.filename),
               let image = PlatformImage(data: item.initialData) {
                initialImages[attachment.htreeUrl] = image
            }
        }
        _loadedData = State(initialValue: initial)
        _loadedImages = State(initialValue: initialImages)
    }

    private var currentAttachment: MessageAttachmentSnapshot? {
        item.attachments.indices.contains(currentIndex) ? item.attachments[currentIndex] : nil
    }

    private var currentData: Data? {
        currentAttachment.flatMap { loadedData[$0.htreeUrl] }
    }

    private var loadTaskID: String {
        "\(item.id):\(currentIndex)"
    }

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                Color.black
                    .opacity(backdropOpacity)
                    .ignoresSafeArea()
                    .onTapGesture(perform: onClose)

                carouselContent
                    .padding(.top, geometry.safeAreaInsets.top + 64)
                    .padding(.bottom, geometry.safeAreaInsets.bottom + 92)
                    .offset(y: dragTranslation)
                    #if os(iOS)
                    .simultaneousGesture(dismissDragGesture)
                    #endif

                VStack(spacing: 0) {
                    topChrome(topInset: geometry.safeAreaInsets.top)
                    Spacer(minLength: 0)
                    bottomChrome(bottomInset: geometry.safeAreaInsets.bottom)
                }
                .ignoresSafeArea()
                .opacity(chromeOpacity)
            }
        }
        .background(Color.black.opacity(backdropOpacity).ignoresSafeArea())
        .environment(\.colorScheme, .dark)
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .zIndex(10)
        .task(id: loadTaskID) {
            let index = currentIndex
            await ensureLoaded(index: index)
            updateSharedFile()
            await preloadAdjacent(index: index)
        }
    }

    private var backdropOpacity: Double {
        let fade = min(abs(dragTranslation) / 600, 0.55)
        return 1 - fade
    }

    private var chromeOpacity: Double {
        let fade = min(abs(dragTranslation) / 220, 1)
        return 1 - fade
    }

    #if os(iOS)
    private var dismissDragGesture: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { value in
                let translation = value.translation
                if abs(translation.height) > abs(translation.width) * 1.3 {
                    dragTranslation = translation.height
                }
            }
            .onEnded { value in
                let translation = value.translation.height
                let predicted = value.predictedEndTranslation.height
                if abs(translation) > 140 || abs(predicted) > 360 {
                    onClose()
                } else {
                    withAnimation(.interactiveSpring(response: 0.32, dampingFraction: 0.85)) {
                        dragTranslation = 0
                    }
                }
            }
    }
    #endif

    @ViewBuilder
    private var carouselContent: some View {
        #if os(iOS)
        TabView(selection: $currentIndex) {
            ForEach(Array(item.attachments.enumerated()), id: \.offset) { idx, attachment in
                IrisImageViewerPage(
                    data: loadedData[attachment.htreeUrl],
                    image: loadedImages[attachment.htreeUrl],
                    filename: attachment.filename
                )
                .tag(idx)
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        #else
        ZStack {
            if let attachment = currentAttachment {
                IrisImageViewerPage(
                    data: loadedData[attachment.htreeUrl],
                    image: loadedImages[attachment.htreeUrl],
                    filename: attachment.filename
                )
            }
            if item.attachments.count > 1 {
                HStack {
                    chevronButton(systemName: "chevron.left", disabled: currentIndex == 0) {
                        if currentIndex > 0 { currentIndex -= 1 }
                    }
                    Spacer()
                    chevronButton(systemName: "chevron.right", disabled: currentIndex >= item.attachments.count - 1) {
                        if currentIndex < item.attachments.count - 1 { currentIndex += 1 }
                    }
                }
                .padding(.horizontal, 18)
            }
        }
        .irisOnLeftArrowKey {
            if currentIndex > 0 { currentIndex -= 1 }
        }
        .irisOnRightArrowKey {
            if currentIndex < item.attachments.count - 1 { currentIndex += 1 }
        }
        #endif
    }

    private func chevronButton(systemName: String, disabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            IrisGlassCircleButtonLabel(
                systemName: systemName,
                iconSize: 18,
                hitSize: 44,
                tone: .dark,
                glyphColor: Color.white.opacity(disabled ? 0.4 : 1)
            )
            .opacity(disabled ? 0.55 : 1)
        }
        .buttonStyle(.irisPlain)
        .disabled(disabled)
    }

    @MainActor
    private func ensureLoaded(index: Int) async {
        guard item.attachments.indices.contains(index) else { return }
        let attachment = item.attachments[index]
        if loadedData[attachment.htreeUrl] != nil { return }
        guard let data = await item.downloadAttachment(attachment) else { return }
        loadedData[attachment.htreeUrl] = data
        let isAnimated = isAnimatedImage(data: data, filename: attachment.filename)
        if !isAnimated, loadedImages[attachment.htreeUrl] == nil {
            let bytes = data
            let image = await Task.detached(priority: .userInitiated) {
                PlatformImage(data: bytes)
            }.value
            if let image {
                loadedImages[attachment.htreeUrl] = image
            }
        }
    }

    @MainActor
    private func preloadAdjacent(index: Int) async {
        for neighbor in [index - 1, index + 1] where item.attachments.indices.contains(neighbor) {
            await ensureLoaded(index: neighbor)
        }
    }

    @MainActor
    private func updateSharedFile() {
        guard let attachment = currentAttachment, let data = loadedData[attachment.htreeUrl] else {
            sharedFileURL = nil
            return
        }
        sharedFileURL = writeTempImage(data: data, filename: attachment.filename)
    }

    private func topChrome(topInset: CGFloat) -> some View {
        ZStack {
            HStack {
                backButton
                Spacer(minLength: 0)
            }
            senderHeader
        }
        .padding(.top, topInset + 4)
        .padding(.horizontal, 12)
    }

    private var backButton: some View {
        Button(action: onClose) {
            IrisGlassCircleButtonLabel(
                systemName: "chevron.left",
                iconSize: 16,
                hitSize: 40,
                tone: .dark,
                glyphColor: .white
            )
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Close image")
        .accessibilityIdentifier("imageViewerCloseButton")
    }

    private var senderHeader: some View {
        VStack(spacing: 2) {
            Text(item.senderName)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(Color.white)
                .lineLimit(1)
            Text(imageViewerDate(item.createdAtSecs))
                .font(.system(.caption, design: .rounded, weight: .medium))
                .foregroundStyle(Color.white.opacity(0.72))
                .lineLimit(1)
        }
        .padding(.horizontal, 12)
    }

    private func bottomChrome(bottomInset: CGFloat) -> some View {
        VStack(spacing: 12) {
            pageIndicator
            HStack(alignment: .center, spacing: 0) {
                shareButton
                    .frame(width: 40, height: 40)
                Spacer(minLength: 12)
                forwardButton
                    .frame(width: 40, height: 40)
            }
            .frame(height: 40)
        }
        .padding(.horizontal, 20)
        .padding(.top, 10)
        .padding(.bottom, bottomInset + 14)
        .background(alignment: .bottom) {
            LinearGradient(
                colors: [
                    Color.black.opacity(0),
                    Color.black.opacity(0.42),
                    Color.black.opacity(0.68)
                ],
                startPoint: .top,
                endPoint: .bottom
            )
            .allowsHitTesting(false)
        }
    }

    @ViewBuilder
    private var pageIndicator: some View {
        if item.attachments.count > 1 {
            HStack(spacing: 6) {
                ForEach(0..<item.attachments.count, id: \.self) { idx in
                    Circle()
                        .fill(Color.white.opacity(idx == currentIndex ? 0.95 : 0.38))
                        .frame(width: 6, height: 6)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule(style: .continuous).fill(Color.black.opacity(0.42)))
            .accessibilityHidden(true)
        }
    }

    @ViewBuilder
    private var forwardButton: some View {
        if let attachment = currentAttachment {
            IrisImageViewerForwardButton(
                text: item.forwardableTextFor(attachment),
                onForwardText: onForwardText
            )
        }
    }

    private var shareButton: some View {
        IrisImageViewerShareButton(sharedFileURL: sharedFileURL)
    }
}

private func writeTempImage(data: Data, filename: String) -> URL? {
    let safeName = safeImageShareFilename(data: data, filename: filename)
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

private struct IrisImageViewerShareButton: View {
    let sharedFileURL: URL?

    var body: some View {
        Group {
            if let sharedFileURL {
                ShareLink(item: sharedFileURL) {
                    IrisImageViewerIconButtonLabel(systemName: "square.and.arrow.up")
                }
            } else {
                Button(action: {}) {
                    IrisImageViewerIconButtonLabel(systemName: "square.and.arrow.up", isEnabled: false)
                }
                .disabled(true)
            }
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Share image")
        .accessibilityIdentifier("imageViewerShareButton")
    }
}

private struct IrisImageViewerForwardButton: View {
    let text: String
    let onForwardText: (String) -> Void

    var body: some View {
        if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            Button {
                onForwardText(text)
            } label: {
                IrisImageViewerIconButtonLabel(systemName: "arrowshape.turn.up.right")
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Forward")
            .accessibilityIdentifier("imageViewerForwardButton")
        }
    }
}

private struct IrisImageViewerPage: View {
    let data: Data?
    let image: PlatformImage?
    let filename: String

    var body: some View {
        Group {
            if let data, isAnimatedImage(data: data, filename: filename) {
                IrisAnimatedImageDataView(data: data)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .allowsHitTesting(false)
            } else if let image {
                Image(platformImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if data == nil {
                ProgressView()
                    .tint(.white)
            } else {
                ProgressView()
                    .tint(.white)
            }
        }
    }
}

private struct IrisImageViewerIconButtonLabel: View {
    let systemName: String
    var isEnabled = true

    var body: some View {
        IrisGlassCircleButtonLabel(
            systemName: systemName,
            iconSize: 18,
            hitSize: 40,
            tone: .dark,
            glyphColor: Color.white.opacity(isEnabled ? 1 : 0.38)
        )
    }
}

private func imageViewerDate(_ secs: UInt64) -> String {
    imageViewerDateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(secs)))
}

private let imageViewerDateFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.doesRelativeDateFormatting = true
    formatter.dateStyle = .medium
    formatter.timeStyle = .short
    return formatter
}()

private func uploadFraction(_ progress: UploadProgress?) -> Double? {
    guard let progress, progress.totalBytes > 0 else { return nil }
    let fraction = Double(progress.bytesUploaded) / Double(progress.totalBytes)
    return min(max(fraction, 0), 1)
}

private func safeImageShareFilename(data: Data, filename: String) -> String {
    let trimmed = filename.trimmingCharacters(in: .whitespacesAndNewlines)
    var safeName = trimmed.isEmpty ? "image" : (trimmed as NSString).lastPathComponent
    if safeName.isEmpty || safeName == "." || safeName == "/" {
        safeName = "image"
    }

    let invalidScalars = CharacterSet(charactersIn: "/\\:")
    safeName = safeName.unicodeScalars
        .map { invalidScalars.contains($0) ? "-" : String($0) }
        .joined()
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if safeName.isEmpty {
        safeName = "image"
    }

    let currentExtension = (safeName as NSString).pathExtension
    if currentExtension.isEmpty {
        safeName += ".\(imageShareFileExtension(data: data, filename: filename))"
    }
    return safeName
}

private func imageShareFileExtension(data: Data, filename: String) -> String {
    let originalExtension = (filename as NSString).pathExtension.lowercased()
    if chatImageExtensions.contains(originalExtension) {
        return originalExtension
    }
    let bytes = [UInt8](data.prefix(12))
    if bytes.starts(with: [0x89, 0x50, 0x4E, 0x47]) {
        return "png"
    }
    if bytes.starts(with: [0xFF, 0xD8, 0xFF]) {
        return "jpg"
    }
    if bytes.starts(with: Array("GIF87a".utf8)) || bytes.starts(with: Array("GIF89a".utf8)) {
        return "gif"
    }
    if bytes.count >= 12,
       bytes[0...3] == Array("RIFF".utf8)[0...3],
       bytes[8...11] == Array("WEBP".utf8)[0...3] {
        return "webp"
    }
    return "jpg"
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

// Signal-style link styling: force the body text colour (the
// foreground attribute overrides SwiftUI's default link tint) and
// underline. Without the explicit foregroundColor, AttributedString
// still rendered links in the system accent / blue on iOS, which
// shifted hue between incoming and outgoing bubbles.
private func linkedMessageAttributedString(_ text: String, foreground: Color) -> AttributedString {
    var attributed = AttributedString()
    var cursor = text.startIndex
    for match in messageURLMatches(in: text) {
        if cursor < match.range.lowerBound {
            attributed.append(AttributedString(String(text[cursor..<match.range.lowerBound])))
        }
        var linked = AttributedString(String(text[match.range]))
        linked.link = match.url
        linked.underlineStyle = .single
        linked.foregroundColor = foreground
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

private func forwardableMessageText(_ message: ChatMessageSnapshot) -> String {
    let parsed = parseReplyEncodedMessage(message.body)
    var pieces: [String] = []
    let body = parsed.body.trimmingCharacters(in: .whitespacesAndNewlines)
    if !body.isEmpty {
        pieces.append(body)
    }
    pieces.append(contentsOf: message.attachments.map(forwardableAttachmentText).filter { !$0.isEmpty })
    return pieces.joined(separator: "\n")
}

private func forwardableAttachmentText(_ attachment: MessageAttachmentSnapshot) -> String {
    attachment.htreeUrl.trimmingCharacters(in: .whitespacesAndNewlines)
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
