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
    @Environment(\.irisNavigationHeaderTopInset) private var navigationHeaderTopInset
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.displayScale) private var displayScale
    @ObservedObject var manager: AppManager
    let chatId: String

    @State private var composerState = IrisComposerState()
    @State private var selectedAttachments: [StagedAttachment] = []
    @State private var sendFilesDirectly = false
    @State private var fileDropAvailable = false
    @State private var isPreparingDroppedAttachments = false
    @State var isNearBottom = true
    @State var shouldFollowLatest = true
    @State var forceScrollToLatest = false
    @State var pendingScrollSettle: DispatchWorkItem?
    @State var timelineUserScrollGeneration = 0
    @State var timelineScrollSettleGeneration = 0
    @State var timelineAutoFollowSuppressedUntil: Date?
    @State var timelineViewportMinY: CGFloat = 0
    @State var timelineViewportMaxY: CGFloat = 0
    @State var timelineTopMinY: CGFloat = -.greatestFiniteMagnitude
    @State private var timelineBottomMaxY: CGFloat = .greatestFiniteMagnitude
    @State private var timelineContentHeight: CGFloat = 0
    @State private var timelineDaySeparatorFrames: [String: ChatTimelineDaySeparatorFrame] = [:]
    @State var initialPlacement = ChatTimelineInitialPlacement()
    @State var timelineChatGeneration = 0
    @State var timelineReadyForDisplay = false
    @State var renderedMessageCount = 0
    @State var pendingPrependAnchorMessageId: String?
    @State private var rowHeightCache: [String: CGFloat] = [:]
    @State var renderWindow = ChatTimelineRenderWindow()
    @State var timelineLayoutGeneration = 0
    @State var pendingTimelineScroll: ChatTimelineScrollRequest?
    @StateObject var timelineCoordinator = ChatTimelineInteractionCoordinator()
    @State private var activeBubbleSwipe: ActiveMessageBubbleSwipe?
    @State private var activeMessageActionDockId: String?
    @State private var replyTarget: ChatMessageSnapshot?
    @State private var editTarget: ChatMessageSnapshot?
    @State private var editComposerState = IrisComposerState()
    @State private var deleteForEveryoneTarget: MessageInfoSelection?
    @State private var editHistorySelection: MessageInfoSelection?
    @State private var imageViewerItem: ImageViewerItem?
    @State private var messageInfoSelection: MessageInfoSelection?
    @State private var reactorsSelection: MessageReactorsSelection?
    /// Hide the gate immediately while the core persists acceptance.
    /// Queued snapshots must not bring the request buttons back.
    @State private var acceptedRequestChatId: String?
    @State private var messageRequestBlockChat: MessageRequestActionTarget?
    @State private var messageRequestReportChat: MessageRequestActionTarget?
    @State private var messageRequestDeleteChat: MessageRequestActionTarget?
    @State private var isComposerFocused = false

    var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var timelineIsVisible: Bool {
        timelineReadyForDisplay && (chat?.messages.isEmpty != false
            || (!initialPlacement.isPending && !initialPlacement.isAwaitingVisibility))
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
                                        ChatTimelineContentLayout {
                                            let ids = chat.messages.map(\.id)
                                            let range = renderedTimelineRange(ids: ids)
                                            let visibleMessages = Array(chat.messages[range])
                                            let layoutGeneration = timelineLayoutGeneration
                                            // Lay out one bounded window exactly; measured spacers
                                            // preserve the coordinates of previously visited history.
                                            ChatTimelineMessageLayout {
                                                Color.clear.frame(height: ChatTimelineRenderWindow.spacerHeight(
                                                    ids[..<range.lowerBound], measured: rowHeightCache))
                                                    .accessibilityHidden(true)
                                                Color.clear
                                                    .frame(height: 1)
                                                    .id(ChatTimelineAnchor.top)
                                                    .accessibilityHidden(true)

                                                ForEach(Array(visibleMessages.enumerated()), id: \.element.id) { index, message in
                                                    let fullIndex = range.lowerBound + index
                                                    let previous = fullIndex > 0 ? chat.messages[fullIndex - 1] : nil
                                                    let next = fullIndex + 1 < chat.messages.count ? chat.messages[fullIndex + 1] : nil
                                                    chatMessageRow(
                                                        message: message,
                                                        previous: previous,
                                                        next: next,
                                                        chat: chat,
                                                        hidesInlineDayChip: floatingSeparator?.messageId == message.id,
                                                        proxy: proxy
                                                    )
                                                }
                                                Color.clear.frame(height: ChatTimelineRenderWindow.spacerHeight(
                                                    ids[range.upperBound...], measured: rowHeightCache))
                                                    .accessibilityHidden(true)
                                            }
                                            .transformPreference(ChatMessageContentFramePreferenceKey.self) { page in
                                                page.chatID = chat.chatId
                                                page.firstMessageID = visibleMessages.first?.id
                                                page.lastMessageID = visibleMessages.last?.id
                                                page.layoutGeneration = layoutGeneration
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
                                                // padded timeline stack, so its
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
                                                    .preference(
                                                        key: ChatTimelineTopMinYPreferenceKey.self,
                                                        value: geo.frame(in: .named(ChatTimelineCoordinateSpace.name)).minY
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
                                            .observeChatTimelineScroll(coordinator: timelineCoordinator, viewportHeight: viewport.size.height, onLayout: {
                                                updateTimelineViewport(maxY: timelineViewportMaxY, proxy: proxy, chat: chat)
                                                fulfillTimelineScroll(proxy: proxy)
                                            }) { translationY, velocityY in
                                                handleTimelineUserPan(translationY: translationY, velocityY: velocityY)
                                            }

                                            // Keep the end marker outside the message layout so it
                                            // remains available on both eager and lazy platforms.
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
                                    }
                                    .irisDefaultScrollAnchorBottom()
                                    .irisOnChange(of: viewport.size.width) { _ in preserveBrowsingPosition() }
                                    .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
                                    .accessibilityIdentifier("chatTimeline")
                                    .overlay {
                                        GeometryReader { geometry in
                                            let frame = geometry.frame(in: .named(ChatTimelineCoordinateSpace.name))
#if os(iOS)
                                            let visibleTop = frame.minY + geometry.safeAreaInsets.top
#else
                                            let visibleTop = frame.minY
#endif
                                            Color.clear
                                                .preference(
                                                    key: ChatTimelineViewportMinYPreferenceKey.self,
                                                    value: visibleTop
                                                )
                                                .preference(
                                                    key: ChatTimelineViewportMaxYPreferenceKey.self,
                                                    // safeAreaInset already reduces this overlay frame.
                                                    // Subtracting its inherited inset again hides valid space.
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
                                    .opacity(timelineIsVisible ? 1 : 0)
                                    .allowsHitTesting(timelineIsVisible)
                                }
                                .irisOnChange(of: chatId) { _ in
                                    timelineChatGeneration += 1
                                    timelineScrollSettleGeneration += 1
                                    pendingScrollSettle?.cancel()
                                    pendingScrollSettle = nil
                                    timelineCoordinator.messageContentFrames = [:]
#if os(iOS)
                                    timelineCoordinator.historyViewportAnchor = nil
#endif
                                    initialPlacement.reset()
                                    timelineReadyForDisplay = false
                                    isNearBottom = true
                                    shouldFollowLatest = true
                                    forceScrollToLatest = false
                                    timelineAutoFollowSuppressedUntil = nil
                                    renderedMessageCount = 0
                                    pendingPrependAnchorMessageId = nil
                                    rowHeightCache.removeAll()
                                    renderWindow = ChatTimelineRenderWindow()
                                    pendingTimelineScroll = nil
                                    timelineLayoutGeneration += 1
                                    activeBubbleSwipe = nil
                                    activeMessageActionDockId = nil
                                    timelineCoordinator.bubblePanRejected = false
                                    timelineTopMinY = -.greatestFiniteMagnitude
                                    timelineBottomMaxY = .greatestFiniteMagnitude
                                    timelineContentHeight = 0
                                    timelineDaySeparatorFrames = [:]
                                    composerState.lastTypingSentAt = nil
                                    composerState.sentTypingIndicator = false
                                    editTarget = nil
                                    editHistorySelection = nil
                                    deleteForEveryoneTarget = nil
                                }
                                .onPreferenceChange(ChatTimelineViewportMinYPreferenceKey.self) { value in
                                    if !chatTimelineGeometryMatches(timelineViewportMinY, value) {
                                        timelineViewportMinY = value
                                    }
                                    maybeLoadOlderMessages(chat: chat)
                                    advanceInitialPlacement(proxy: proxy, chat: chat)
                                    recordInteractionLayout()
                                }
                                .onPreferenceChange(ChatTimelineTopMinYPreferenceKey.self) { value in
                                    if !chatTimelineGeometryMatches(timelineTopMinY, value) {
                                        timelineTopMinY = value
                                    }
                                    maybeLoadOlderMessages(chat: chat)
                                }
                                .onPreferenceChange(ChatTimelineViewportMaxYPreferenceKey.self) { value in
                                    updateTimelineViewport(maxY: value, proxy: proxy, chat: chat)
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
                                    advanceInitialPlacement(proxy: proxy, chat: chat)
                                }
                                .onPreferenceChange(ChatTimelineContentHeightPreferenceKey.self) { value in
                                    // Repin to the bottom when the timeline's
                                    // intrinsic content grew (reaction landed,
                                    // attachment finished loading, quote
                                    // preview rendered, etc.) and we were
                                    // already following. Lazy row realization can also change
                                    // estimated height. User scrolling disables
                                    // following before those updates arrive.
                                    let previous = timelineContentHeight
                                    if !chatTimelineGeometryMatches(timelineContentHeight, value) {
                                        timelineContentHeight = value
                                    }
                                    let grew = previous > 0 && value > previous + 1
                                    let canAutoFollow = (shouldFollowLatest || isNearBottom)
                                        && !timelineAutoFollowIsSuppressed()
                                        && pendingTimelineScroll == nil && manager.pendingScrollMessageId == nil
                                        && pendingPrependAnchorMessageId == nil
                                    if !initialPlacement.isPending, canAutoFollow, grew {
                                        scrollToBottom(proxy: proxy, animated: false)
                                    }
                                    advanceInitialPlacement(proxy: proxy, chat: chat)
                                }
                                .onPreferenceChange(ChatMessageContentFramePreferenceKey.self) { page in
                                    guard page.chatID == chatId else { return }
                                    timelineCoordinator.messageContentFrames = page.frames
#if os(iOS)
                                    for (id, height) in page.heights where rowHeightCache[id] != height {
                                        rowHeightCache[id] = height
                                    }
                                    timelineCoordinator.latestPage = page
                                    if timelineCoordinator.historyViewportAnchor == nil,
                                       let boundary = pendingPrependAnchorMessageId,
                                       chat.messages.first?.id != boundary {
                                        pendingPrependAnchorMessageId = nil
                                    }
                                    if let anchor = timelineCoordinator.historyViewportAnchor,
                                       let first = page.firstMessageID,
                                       first != anchor.firstMessageID || page.layoutGeneration != anchor.layoutGeneration {
                                        if !chat.messages.contains(where: { $0.id == anchor.messageID }) {
                                            timelineCoordinator.historyViewportAnchor = nil
                                            pendingPrependAnchorMessageId = nil
                                        } else if timelineCoordinator.restoreHistoryViewportAnchor(page: page) {
                                            pendingPrependAnchorMessageId = nil
                                            // Offset changed; these frames still belong to the old viewport.
                                            return
                                        }
                                    }
#endif
                                    updateTimelineFollowState(nearBottom: chatTimelineIsNearBottom(
                                        viewportMaxY: timelineViewportMaxY, bottomMaxY: timelineBottomMaxY),
                                        messageCount: chat.messages.count)
                                    fulfillTimelineScroll(proxy: proxy)
                                    maybeShiftRenderWindow(page: page, chat: chat)
                                    maybeLoadOlderMessages(chat: chat)
                                    advanceInitialPlacement(proxy: proxy, chat: chat)
                                    recordInteractionLayout()
                                }
                                .onPreferenceChange(ChatAudioControlFramePreferenceKey.self) { value in
                                    timelineCoordinator.audioControlFrames = value
                                }
                                .onPreferenceChange(ChatTimelineDaySeparatorFramePreferenceKey.self) { value in
                                    timelineDaySeparatorFrames = value
                                }
                                .task(id: chatTimelineScrollTaskToken(for: chat)) {
                                    guard !chat.messages.isEmpty else {
                                        initialPlacement.reset()
                                        revealTimelineAfterLayout()
                                        shouldFollowLatest = true
                                        forceScrollToLatest = false
                                        renderedMessageCount = 0
                                        return
                                    }
                                    let messageCount = chat.messages.count
#if os(iOS)
                                    let retainedIDs = Set(chat.messages.map(\.id))
                                    for id in rowHeightCache.keys where !retainedIDs.contains(id) {
                                        rowHeightCache[id] = nil
                                    }
#endif
                                    if let anchorId = pendingPrependAnchorMessageId,
                                       chat.messages.first?.id != anchorId,
                                       chat.messages.contains(where: { $0.id == anchorId }) {
                                        renderedMessageCount = messageCount
                                        initialPlacement.cancel()
#if !os(iOS)
                                        scrollToMessage(proxy: proxy, messageId: anchorId, anchor: .top, animated: false)
                                        pendingPrependAnchorMessageId = nil
#endif
                                        revealTimelineAfterLayout()
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
                                            initialPlacement.cancel()
                                            shouldFollowLatest = false
                                            forceScrollToLatest = false
                                            scrollToMessage(proxy: proxy, messageId: targetId)
                                            revealTimelineAfterLayout()
                                            manager.consumePendingScrollMessage()
                                            return
                                        }
                                        manager.loadChatAroundMessage(chatId: chat.chatId, messageId: targetId)
                                    }
                                    let shouldScroll = initialPlacement.isPending
                                        || forceScrollToLatest
                                        || (
                                            messageCountIncreased
                                                && pendingTimelineScroll == nil && manager.pendingScrollMessageId == nil
                                                && (shouldFollowLatest || isNearBottom)
                                                && !timelineAutoFollowIsSuppressed()
                                        )
                                    renderedMessageCount = messageCount
                                    if initialPlacement.isPending || initialPlacement.isAwaitingVisibility {
                                        advanceInitialPlacement(proxy: proxy, chat: chat)
                                    } else if shouldScroll {
                                        scrollToBottom(proxy: proxy, animated: true)
                                        shouldFollowLatest = true
                                    } else if !initialPlacement.isAwaitingVisibility {
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

                                if timelineIsVisible && !isNearBottom && !chat.messages.isEmpty {
                                    ChatJumpToBottomButton {
                                        jumpToLatest(proxy: proxy)
                                    }
                                    .padding(.trailing, 8)
                                    .padding(.bottom, 8)
                                    .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
                                }

                                if timelineIsVisible && !chat.typingIndicators.isEmpty {
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
                                let capability = chat.kind == .direct ? chat.directChatCapability : nil
                                let capabilityBlocked = capability != nil && capability != .available
                                VStack(spacing: 0) {
                                    IrisNameChangeNotice(manager: manager, chat: chat)
                                    if editTarget != nil, !composerBlocked, !isRequest, !chat.isRemovedFromGroup {
                                        HStack {
                                            Label("Editing message", systemImage: "pencil")
                                            Spacer()
                                            Button("Cancel") { editTarget = nil }
                                        }
                                        .font(.callout)
                                        .padding(.horizontal, 16)
                                        .padding(.vertical, 8)
                                        .accessibilityIdentifier("chatEditingBanner")
                                    }
                                    if editTarget == nil, let replyTarget, !composerBlocked, !isRequest, !chat.isRemovedFromGroup {
                                        IrisReplyComposerStrip(message: replyTarget, explicitAuthorName: explicitPersonName(for: replyTarget.authorOwnerPubkeyHex, state: manager.state)) {
                                            self.replyTarget = nil
                                        }
                                    }
                                    if chat.isRemovedFromGroup {
                                        IrisRemovedGroupBar()
                                    } else if composerBlocked {
                                        IrisBlockedComposerBar {
                                            manager.setUserBlocked(chat.chatId, blocked: false)
                                        } onDelete: {
                                            manager.dispatch(.deleteChat(chatId: chat.chatId))
                                            manager.navigateBack()
                                        }
                                    } else if isRequest {
                                        IrisMessageRequestBar(
                                            displayName: chat.displayName,
                                            onAccept: {
                                                acceptedRequestChatId = chat.chatId
                                                manager.dispatch(.setMessageRequestAccepted(chatId: chat.chatId))
#if os(macOS)
                                                isComposerFocused = true
#endif
                                            },
                                            onBlock: {
                                                messageRequestBlockChat = MessageRequestActionTarget(
                                                    chatId: chat.chatId,
                                                    displayName: chat.displayName
                                                )
                                            },
                                            onBlockAndReport: {
                                                messageRequestReportChat = MessageRequestActionTarget(
                                                    chatId: chat.chatId,
                                                    displayName: chat.displayName
                                                )
                                            }
                                        )
                                    } else {
                                        IrisDelayedCapabilityStatus(state: capability) {
                                            manager.dispatch(.retryDirectChatCapability(chatId: chat.chatId))
                                        }
                                        .id(chat.chatId)
                                        IrisComposerBar(
                                            composerState: editTarget == nil ? composerState : editComposerState,
                                            attachments: editTarget == nil ? $selectedAttachments : .constant([]),
                                            sendFilesDirectly: $sendFilesDirectly,
                                            directFilesAllowed: chat.kind == .direct,
                                            placeholder: "Message",
                                            isSending: manager.state.busy.sendingMessage,
                                            isUploading: manager.state.busy.uploadingAttachment,
                                            uploadFraction: uploadFraction(manager.state.busy.uploadProgress),
                                            isFocused: $isComposerFocused,
                                            onUserEdit: { text in
                                                if editTarget == nil && !capabilityBlocked { sendTypingIfNeeded(text: text) }
                                            },
                                            onDraftChange: {
                                                guard editTarget == nil else { return }
                                                composerState.scheduleSave { text in
                                                    manager.dispatch(.setChatDraft(chatId: chatId, text: text))
                                                }
                                            },
                                            onAttach: stageAttachments,
                                            voiceRecordingAllowed: editTarget == nil && !capabilityBlocked && (manager.state.call == nil || manager.state.call?.phase == "ended"),
                                            onStageVoice: { try await manager.stageOutgoingAttachmentsAsync([$0]) },
                                            onSendVoice: { voice in
                                                guard canAttachToCurrentChat,
                                                      !manager.state.busy.sendingMessage,
                                                      !manager.state.busy.uploadingAttachment,
                                                      !manager.isUserBlocked(chatId) else { return false }
                                                stopTypingIfNeeded()
                                                resumeTimelineAutoFollow()
                                                shouldFollowLatest = true
                                                forceScrollToLatest = true
                                                let caption = replyEncodedMessage(reply: replyTarget, text: "")
                                                replyTarget = nil
                                                manager.sendAttachments(chatId: chatId, attachments: voice, caption: caption)
                                                return true
                                            },
                                            sendAllowed: !capabilityBlocked,
                                            isEditing: editTarget != nil,
                                            isPreparingDroppedAttachments: isPreparingDroppedAttachments,
                                            onFileDropAvailabilityChange: { fileDropAvailable = $0 }
                                        ) { composerText in
                                            guard canAttachToCurrentChat else { return }
                                            let text = composerText.trimmingCharacters(in: .whitespacesAndNewlines)
                                            if let editTarget {
                                                guard !text.isEmpty else { return }
                                                manager.dispatch(.editMessage(chatId: chatId, messageId: editTarget.id, text: editedMessageText(original: editTarget.body, text: text)))
                                                self.editTarget = nil
                                                return
                                            }
                                            guard !text.isEmpty || !selectedAttachments.isEmpty else { return }
                                            resumeTimelineAutoFollow()
                                            shouldFollowLatest = true
                                            forceScrollToLatest = true
                                            let outgoingText = replyEncodedMessage(reply: replyTarget, text: text)
                                            replyTarget = nil
                                            // Queue the message before typing or draft cleanup can
                                            // block the core on encryption or storage.
                                            if selectedAttachments.isEmpty {
                                                manager.dispatch(.sendMessage(chatId: chatId, text: outgoingText))
                                            } else {
                                                let attachments = selectedAttachments
                                                selectedAttachments = []
                                                manager.dispatch(irisAttachmentSendAction(chatId: chatId, attachments: attachments, caption: outgoingText, sendDirectly: sendFilesDirectly))
                                                sendFilesDirectly = false
                                            }
                                            composerState.clearForSend { text in
                                                manager.dispatch(.setChatDraft(chatId: chatId, text: text))
                                            }
                                            stopTypingIfNeeded()
                                        }
                                        .id(editTarget?.id ?? "newMessage")
                                        .task(id: chatId) {
                                            if IrisLayout.usesDesktopChrome {
                                                isComposerFocused = true
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
        .sheet(item: $editHistorySelection) { selection in
            let context = messageInfoContext(for: selection)
            MessageEditHistorySheet(initialMessage: context.message, manager: manager) { editHistorySelection = nil }
                .irisModalSurface()
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
        .confirmationDialog("Delete for everyone?", isPresented: Binding(
            get: { deleteForEveryoneTarget != nil },
            set: { if !$0 { deleteForEveryoneTarget = nil } }
        ), titleVisibility: .visible) {
            Button("Delete for everyone", role: .destructive) {
                guard let target = deleteForEveryoneTarget else { return }
                manager.dispatch(.deleteMessageForEveryone(chatId: target.chatId, messageId: target.messageId))
                if editTarget?.id == target.messageId { editTarget = nil }
                if replyTarget?.id == target.messageId { replyTarget = nil }
                deleteForEveryoneTarget = nil
            }
            Button("Cancel", role: .cancel) { deleteForEveryoneTarget = nil }
        } message: {
            Text("People who allow message deletion will see “Message deleted”.")
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
        .modifier(IrisAttachmentDropModifier(
            enabled: editTarget == nil && fileDropAvailable && canAttachToCurrentChat,
            isPreparing: $isPreparingDroppedAttachments,
            onAttach: stageAttachments
        ))
        .modifier(MessageRequestSafetyModifier(
            blockTarget: $messageRequestBlockChat,
            reportTarget: $messageRequestReportChat,
            deleteTarget: $messageRequestDeleteChat,
            manager: manager
        ))
        .onReceive(manager.$state) { incoming in
            if let current = incoming.currentChat, current.chatId == chatId {
                let deletedIds = Set(current.messages.filter(\.deletedForEveryone).map(\.id))
                if let editTarget, deletedIds.contains(editTarget.id) { self.editTarget = nil }
                if let replyTarget, deletedIds.contains(replyTarget.id) { self.replyTarget = nil }
            }
#if os(iOS)
            guard case .chat(let activeID) = incoming.router.screenStack.last,
                  activeID == chatId, let current = chat, let next = incoming.currentChat,
                  next.chatId == chatId, next.messages != current.messages else { return }
            preserveBrowsingPosition()
#endif
        }
        .irisOnChange(of: dynamicTypeSize) { _ in preserveBrowsingPosition() }
        .irisOnChange(of: displayScale) { _ in preserveBrowsingPosition() }
        .onDisappear {
            pendingTimelineScroll = nil
            timelineChatGeneration += 1
            timelineScrollSettleGeneration += 1
            composerState.invalidatePendingAttachments()
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

    private var canAttachToCurrentChat: Bool {
        guard let chat, !chat.isRemovedFromGroup, !manager.state.busy.sendingMessage,
              !manager.state.busy.uploadingAttachment else { return false }
        if chat.kind == .direct {
            return !manager.isUserBlocked(chatId) &&
                (!chat.isRequest || acceptedRequestChatId == chatId) &&
                (chat.directChatCapability == nil || chat.directChatCapability == .available)
        }
        return true
    }

    private func stageAttachments(_ loadURLs: () async -> [URL]) async {
        guard editTarget == nil, canAttachToCurrentChat else { return }
        let generation = composerState.attachmentGeneration
        do {
            let staged = try await manager.stageOutgoingAttachmentsAsync(loadURLs)
            guard !Task.isCancelled, generation == composerState.attachmentGeneration, canAttachToCurrentChat else {
                await manager.discardOutgoingAttachments(staged)
                return
            }
            guard !staged.isEmpty else { manager.showAttachmentOpenError(); return }
            selectedAttachments.append(contentsOf: staged)
        } catch is CancellationError {
            // Leaving the chat discards an unfinished selection.
        } catch {
            if generation == composerState.attachmentGeneration { manager.showAttachmentOpenError() }
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
            socialConnection: chat.participants.first { $0.ownerPubkeyHex == message.authorOwnerPubkeyHex }?.socialConnection,
            manager: manager,
            message: message,
            chatKind: chat.kind,
            showDayChip: showDayChip,
            hidesInlineDayChip: hidesInlineDayChip,
            isFirstInCluster: isFirstInCluster,
            isLastInCluster: isLastInCluster,
            showsFooter: irisShowsMessageFooter(message: message, next: next, chatKind: chat.kind),
            showsGroupSenderName: showsGroupSenderName,
            showsGroupSenderAvatar: showsGroupSenderAvatar,
            reactions: message.reactions,
            canReplyAndReact: !chat.isRemovedFromGroup && !message.deletedForEveryone,
            swipeOffset: activeBubbleSwipe?.messageId == message.id ? activeBubbleSwipe?.offset ?? 0 : 0,
            isActionDockActive: activeMessageActionDockId == message.id,
            onActionDockActiveChange: { isActive in
                activeMessageActionDockId = irisNextActiveMessageActionDockId(
                    current: activeMessageActionDockId,
                    messageId: message.id,
                    isActive: isActive
                )
            },
            onReply: {
                editTarget = nil
                guard self.chat?.isRemovedFromGroup != true else { return }
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
            onEdit: {
                flushDraftImmediately()
                stopTypingIfNeeded()
                editComposerState.restore(parseReplyEncodedMessage(message.body).body, replaceExisting: true)
                editTarget = message
                isComposerFocused = true
            },
            onEditHistory: {
                editHistorySelection = MessageInfoSelection(chatId: chatId, messageId: message.id, snapshot: message)
            },
            onDeleteForEveryone: {
                deleteForEveryoneTarget = MessageInfoSelection(chatId: chatId, messageId: message.id, snapshot: message)
            },
            onDelete: {
                if editTarget?.id == message.id { editTarget = nil }
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
            previewAudioAttachment: { attachment in
                await manager.previewAudioAttachment(attachment)
            },
            openAttachment: { attachment in
                await manager.openAttachment(attachment)
            },
            directTransferChatId: chatId,
            onDirectTransferAction: manager.dispatch,
            onDirectTransferAccept: manager.acceptDirectFiles,
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
        .irisTimelineRowMeasurement(id: message.id)
        .id(message.id)
    }

    private func handleTimelineUserPan(translationY: CGFloat, velocityY: CGFloat) {
        if abs(translationY) > 6 || abs(velocityY) > 60 { pendingTimelineScroll = nil }
        if pendingScrollSettle != nil {
            pendingScrollSettle?.cancel()
            pendingScrollSettle = nil
            timelineUserScrollGeneration += 1
            timelineScrollSettleGeneration += 1
        }

        if translationY > 6 || velocityY > 60 {
            if shouldFollowLatest, let chat {
                let ids = chat.messages.map(\.id)
                renderWindow.start(at: renderWindow.range(in: ids).lowerBound, in: ids)
            }
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
        if timelineCoordinator.audioControlFrames.contains(where: { $0.contains(value.startLocation) }) {
            timelineCoordinator.bubblePanRejected = true
            return
        }

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
            guard let messageId = timelineCoordinator.messageContentId(at: value.startLocation)
                    ?? timelineCoordinator.messageContentId(at: value.location),
                  let message = chat?.messages.first(where: { $0.id == messageId }),
                  message.kind != .system, message.call == nil, !message.deletedForEveryone else {
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
        if swipe.offset >= ChatMessageBubbleSwipeMetrics.threshold && !chat.isRemovedFromGroup {
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
        composerState.restore(persistedDraftForCurrentChat(), replaceExisting: replaceExisting)
    }

    private func flushDraftImmediately() {
        composerState.flush { text in
            manager.dispatch(.setChatDraft(chatId: chatId, text: text))
        }
    }

    private func sendTypingIfNeeded(text: String) {
        guard chat?.isRemovedFromGroup != true, !manager.isUserBlocked(chatId) else { return }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            stopTypingIfNeeded()
            return
        }
        let now = Date()
        if let lastTypingSentAt = composerState.lastTypingSentAt, now.timeIntervalSince(lastTypingSentAt) < 3 {
            return
        }
        composerState.lastTypingSentAt = now
        composerState.sentTypingIndicator = true
        manager.dispatch(.sendTyping(chatId: chatId))
    }

    private func stopTypingIfNeeded() {
        guard composerState.sentTypingIndicator else { return }
        composerState.sentTypingIndicator = false
        composerState.lastTypingSentAt = nil
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
                scrollToMessage(proxy: proxy, messageId: candidate.id)
                #if os(iOS)
                PlatformHaptics.messageMenuOpened()
                #endif
                return
            }
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
