import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

struct IrisBlockedComposerBar: View {
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
struct IrisMessageRequestBar: View {
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
struct BlockConfirmationTarget: Identifiable {
    let chatId: String
    let displayName: String
    var id: String { chatId }
}

/// Signal-style block confirmation: tapping Block on the
/// message-request bar lifts a sheet with "Block" (hide the thread but
/// keep local evidence) and "Block and Delete" (wipe the chat too).
/// Mirrors `ConversationViewController+MessageRequest.swift` in signal-ios.
/// Extracted to a `ViewModifier` so the ChatScreen body type-checks
/// in reasonable time — the inline form pushed the closure over the
/// compiler's expression-complexity threshold.
struct BlockConfirmationModifier: ViewModifier {
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
                    manager.navigateAwayFromBlockedChat(item.chatId)
                    target = nil
                }
                .accessibilityIdentifier("messageRequestBlockConfirmKeep")
                Button("Block and Delete", role: .destructive) {
                    manager.setUserBlocked(item.chatId, blocked: true)
                    manager.dispatch(.deleteChat(chatId: item.chatId))
                    manager.navigateAwayFromBlockedChat(item.chatId)
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
