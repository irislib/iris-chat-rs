import SwiftUI

// Timeline placement and explicit navigation share one window/anchor owner.
extension ChatScreen {
    func updateTimelineViewport(maxY: CGFloat, proxy: ScrollViewProxy, chat: CurrentChatSnapshot) {
#if os(iOS)
        // Message frames are scroll-local. SwiftUI's surrounding safe-area
        // overlay includes a different origin; use UIKit's visible bounds.
        guard let value = timelineCoordinator.visibleViewportMaxY else { return }
#else
        let value = maxY
#endif
        guard !chatTimelineGeometryMatches(timelineViewportMaxY, value) else { return }
        timelineCoordinator.resizeViewport(from: timelineViewportMaxY, to: value,
                                           preservingPosition: pendingTimelineScroll == nil)
        timelineViewportMaxY = value
        updateTimelineFollowState(nearBottom: false, messageCount: chat.messages.count)
        advanceInitialPlacement(proxy: proxy, chat: chat)
        recordInteractionLayout()
    }

    func preserveBrowsingPosition() {
#if os(iOS)
        guard timelineIsVisible, !shouldFollowLatest, !forceScrollToLatest,
              pendingTimelineScroll == nil, manager.pendingScrollMessageId == nil,
              timelineCoordinator.historyViewportAnchor == nil,
              let chat else { return }
        let ids = chat.messages.map(\.id)
        let range = renderedTimelineRange(ids: ids)
        guard !range.isEmpty else { return }
        timelineCoordinator.captureHistoryViewportAnchor(
            chatID: chatId, firstMessageID: ids[range.lowerBound], layoutGeneration: timelineLayoutGeneration,
            viewportMinY: timelineViewportMinY, viewportMaxY: timelineViewportMaxY)
        timelineLayoutGeneration += 1
#endif
    }

    func fulfillTimelineScroll(proxy: ScrollViewProxy) {
#if os(iOS)
        guard var request = pendingTimelineScroll, request.chatGeneration == timelineChatGeneration,
              let chat else { return }
        let page = timelineCoordinator.latestPage
        let ids = chat.messages.map(\.id)
        guard ids.contains(request.targetID) else { pendingTimelineScroll = nil; return }
        let range = renderedTimelineRange(ids: ids)
        guard !range.isEmpty, page.chatID == chatId, page.firstMessageID == ids[range.lowerBound],
              page.lastMessageID == ids[range.upperBound - 1],
              let frame = page.frames[request.targetID] else { return }
        guard timelineCoordinator.hasCommittedTimelineExtent(page.contentHeight) else { return }
        if request.hasIssued {
            let landed = request.anchor == .bottom
                ? frame.maxY > timelineViewportMinY && frame.maxY <= timelineViewportMaxY + 1
                : frame.maxY > timelineViewportMinY && frame.minY < timelineViewportMaxY
            if landed {
                timelineCoordinator.prepareForExplicitScroll()
                pendingTimelineScroll = nil
                updateTimelineFollowState(nearBottom: false, messageCount: chat.messages.count)
            }
            return
        }
        // Own this window through the animation. Otherwise its old viewport
        // can immediately shift the newly selected window away from the target.
        request.hasIssued = true
        pendingTimelineScroll = request
        if request.anchor == .bottom, request.targetID == chat.messages.last?.id,
           timelineCoordinator.alignTimelineBottom(frame: frame, viewportMaxY: timelineViewportMaxY,
               bottomSpacing: SignalConversationLayout.contentBottomMargin, animated: request.animated) { return }
        if request.animated {
            withAnimation(.easeOut(duration: 0.25)) { proxy.scrollTo(request.targetID, anchor: request.anchor) }
        } else {
            proxy.scrollTo(request.targetID, anchor: request.anchor)
        }
#endif
    }

    func renderedTimelineRange(ids: [String]) -> Range<Int> {
#if os(iOS)
        renderWindow.range(in: ids)
#else
        ids.indices
#endif
    }

    func maybeShiftRenderWindow(page: ChatTimelinePageFrames, chat: CurrentChatSnapshot) {
#if os(iOS)
        guard timelineIsVisible, !shouldFollowLatest, pendingPrependAnchorMessageId == nil,
              pendingTimelineScroll == nil else { return }
        let ids = chat.messages.map(\.id)
        let range = renderWindow.range(in: ids)
        guard let visible = timelineCoordinator.measuredVisibleMessageRange(in: ids, renderedRange: range,
                  chatID: chat.chatId, layoutGeneration: timelineLayoutGeneration),
              let scroll = timelineCoordinator.scrollView,
              let first = timelineCoordinator.latestPage.contentFrames[ids[range.lowerBound]],
              let last = timelineCoordinator.latestPage.contentFrames[ids[range.upperBound - 1]] else { return }
        let minimum = scroll.bounds.minY + scroll.adjustedContentInset.top
        let maximum = scroll.bounds.maxY - scroll.adjustedContentInset.bottom
        let margin = (maximum - minimum) * 0.75
        let start: Int
        if range.lowerBound > 0 && first.maxY > minimum - margin {
            start = max(0, range.lowerBound - ChatTimelineRenderWindow.step)
        } else if range.upperBound < ids.count && last.minY < maximum + margin {
            start = min(ids.count - range.count,
                        range.lowerBound + ChatTimelineRenderWindow.step)
        } else { return }
        guard timelineCoordinator.historyViewportAnchor == nil else { return }
        timelineCoordinator.captureHistoryViewportAnchor(
            chatID: chat.chatId, firstMessageID: ids[range.lowerBound], layoutGeneration: timelineLayoutGeneration,
            viewportMinY: timelineViewportMinY, viewportMaxY: timelineViewportMaxY)
        guard let anchor = timelineCoordinator.historyViewportAnchor else { return }
        var nextWindow = renderWindow
        guard nextWindow.preserveVisible(visible, including: anchor.messageID, in: ids, startAt: start),
              nextWindow.range(in: ids) != range else {
            timelineCoordinator.historyViewportAnchor = nil
            return
        }
        if ProcessInfo.processInfo.environment["IRIS_TIMELINE_TRACE"] == "1" {
            NSLog("WINDOWTRACE swap %@ -> %@ anchor=%@ oldY=%f offset=%f size=%f pan=%f state=%ld", String(describing: range), String(describing: nextWindow.range(in: ids)), anchor.messageID, anchor.originalContentY, scroll.contentOffset.y, scroll.contentSize.height, scroll.panGestureRecognizer.translation(in: scroll).y, scroll.panGestureRecognizer.state.rawValue)
        }
        renderWindow = nextWindow
#endif
    }

    var timelineIsNearHistoryStart: Bool {
        let margin = max(44, (timelineViewportMaxY - timelineViewportMinY) * 0.75)
#if os(iOS)
        guard let scroll = timelineCoordinator.scrollView else { return false }
        // A frame preference can still describe the pre-correction viewport.
        return scroll.contentOffset.y <= -scroll.adjustedContentInset.top + margin
#else
        return timelineTopMinY.isFinite && timelineTopMinY >= timelineViewportMinY - margin
#endif
    }

    func maybeLoadOlderMessages(chat: CurrentChatSnapshot) {
        guard timelineIsVisible, !initialPlacement.isPending,
              pendingTimelineScroll == nil, manager.pendingScrollMessageId == nil,
              let firstMessageId = chat.messages.first?.id,
              timelineIsNearHistoryStart,
              pendingPrependAnchorMessageId == nil else {
            return
        }
        pendingPrependAnchorMessageId = firstMessageId
        if !manager.loadOlderMessages(chatId: chat.chatId, willMerge: {
#if os(iOS)
            guard pendingPrependAnchorMessageId == firstMessageId,
                  pendingTimelineScroll == nil, manager.pendingScrollMessageId == nil else { return }
            let ids = chat.messages.map(\.id)
            let range = renderWindow.range(in: ids)
            let visible = timelineCoordinator.measuredVisibleMessageRange(in: ids, renderedRange: range,
                chatID: chat.chatId, layoutGeneration: timelineLayoutGeneration)
            timelineCoordinator.captureHistoryViewportAnchor(
                chatID: chat.chatId, firstMessageID: firstMessageId, layoutGeneration: timelineLayoutGeneration,
                viewportMinY: timelineViewportMinY, viewportMaxY: timelineViewportMaxY)
            var preserved = false
            if let visible, let anchor = timelineCoordinator.historyViewportAnchor {
                preserved = renderWindow.preserveVisible(visible, including: anchor.messageID, in: ids)
            }
            if !preserved {
                renderWindow.start(at: range.lowerBound, in: ids)
            }
            timelineLayoutGeneration += 1
#endif
        }, completion: { loaded in
            if !loaded {
                pendingPrependAnchorMessageId = nil
            }
        }) {
            pendingPrependAnchorMessageId = nil
        }
    }

    func timelineAutoFollowIsSuppressed(now: Date = Date()) -> Bool {
        guard let until = timelineAutoFollowSuppressedUntil else { return false }
        return until > now
    }

    func resumeTimelineAutoFollow() {
        timelineAutoFollowSuppressedUntil = nil
    }

    func chatTimelineScrollTaskToken(for chat: CurrentChatSnapshot) -> String {
        [
            chat.chatId,
            chat.messages.first?.id ?? "",
            chat.messages.last?.id ?? "",
            String(chat.messages.count),
            manager.pendingScrollMessageId ?? "",
        ].joined(separator: "|")
    }

    func recordInteractionLayout() {
        guard let timing = manager.interactionTiming, let chat else { return }
        timing.layout(chatID: chat.chatId, frames: timelineCoordinator.messageContentFrames,
                      viewportMinY: timelineViewportMinY, viewportMaxY: timelineViewportMaxY,
                      ready: timelineIsVisible, messageCount: chat.messages.count)
    }

    func advanceInitialPlacement(proxy: ScrollViewProxy, chat: CurrentChatSnapshot) {
        guard initialPlacement.isPending || initialPlacement.isAwaitingVisibility,
              manager.pendingScrollMessageId == nil, pendingPrependAnchorMessageId == nil,
              manager.state.currentChat?.chatId == chat.chatId,
              let last = chat.messages.last,
              manager.state.currentChat?.messages.last?.id == last.id else { return }
        let targetID = last.id
        let step = initialPlacement.update(
            targetID: targetID, frame: timelineCoordinator.messageContentFrames[targetID],
            viewportMinY: timelineViewportMinY, viewportMaxY: timelineViewportMaxY
        )
        if step == .scroll || step == .scrollAndReveal {
            renderedMessageCount = chat.messages.count
            shouldFollowLatest = true
            scrollToBottom(proxy: proxy, animated: false)
        }
        if step == .reveal || step == .scrollAndReveal {
            revealTimelineAfterLayout()
        }
    }

    func revealTimelineAfterLayout() {
        guard !timelineReadyForDisplay else { return }
        let generation = timelineChatGeneration
        DispatchQueue.main.async {
            guard timelineChatGeneration == generation, !timelineReadyForDisplay else { return }
            var transaction = Transaction()
            transaction.disablesAnimations = true
            withTransaction(transaction) {
                timelineReadyForDisplay = true
            }
            recordInteractionLayout()
        }
    }

    /// Realize the requested window before moving to a search hit or reply.
    func scrollToMessage(
        proxy: ScrollViewProxy,
        messageId: String,
        anchor: UnitPoint = .center,
        animated: Bool = true
    ) {
#if os(iOS)
        timelineCoordinator.prepareForExplicitScroll()
        pendingPrependAnchorMessageId = nil
        renderWindow.show(messageId, in: chat?.messages.map(\.id) ?? [])
        pendingTimelineScroll = ChatTimelineScrollRequest(targetID: messageId, anchor: anchor,
            animated: animated, chatGeneration: timelineChatGeneration)
        fulfillTimelineScroll(proxy: proxy)
#else
        pendingScrollSettle?.cancel()
        pendingScrollSettle = nil
        timelineScrollSettleGeneration += 1
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
#endif
    }

    func scrollToBottom(
        proxy: ScrollViewProxy,
        animated: Bool,
        settleAfterLayout: Bool = true
    ) {
#if os(iOS)
        guard let target = chat?.messages.last?.id else { return }
        timelineCoordinator.prepareForExplicitScroll()
        pendingPrependAnchorMessageId = nil
        renderWindow.showLatest()
        pendingTimelineScroll = ChatTimelineScrollRequest(targetID: target, anchor: .bottom,
            animated: animated, chatGeneration: timelineChatGeneration)
        fulfillTimelineScroll(proxy: proxy)
#else
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
#endif
    }

    func updateTimelineFollowState(nearBottom: Bool, messageCount: Int) {
        guard pendingTimelineScroll == nil, manager.pendingScrollMessageId == nil else { return }
        let nearBottom = chat?.messages.last.flatMap { timelineCoordinator.messageContentFrames[$0.id] }
            .map { chatTimelineIsNearBottom(viewportMaxY: timelineViewportMaxY,
                                           bottomMaxY: $0.maxY + SignalConversationLayout.contentBottomMargin) }
            ?? nearBottom
        if isNearBottom != nearBottom {
            isNearBottom = nearBottom
        }
        let nextShouldFollow = nearBottom && !timelineAutoFollowIsSuppressed()
        if messageCount == renderedMessageCount, shouldFollowLatest != nextShouldFollow {
            shouldFollowLatest = nextShouldFollow
        }
    }
}
