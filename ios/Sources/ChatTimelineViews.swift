import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

struct MessageReactorsSelection: Identifiable {
    let messageId: String
    var id: String { messageId }
}

struct MessageInfoSelection: Identifiable {
    let chatId: String
    let messageId: String
    let snapshot: ChatMessageSnapshot

    var id: String {
        "\(chatId):\(messageId)"
    }
}

let irisMessageClusterGapSecs: UInt64 = 60

enum SignalConversationLayout {
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
    static let messageActionDockSpacing: CGFloat = 8
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

func irisGroupSenderNameColor(for senderKey: String, isDarkMode: Bool) -> Color {
    irisColor(hex: irisGroupSenderNameColorHex(for: senderKey, isDarkMode: isDarkMode))
}

func irisColor(hex: UInt32) -> Color {
    Color(
        red: Double((hex >> 16) & 0xff) / 255.0,
        green: Double((hex >> 8) & 0xff) / 255.0,
        blue: Double(hex & 0xff) / 255.0
    )
}

// Signal-iOS GroupNameColors, trimmed to avoid reusing the Iris brand
// purple for sender labels. We still keep Signal's high-contrast spread
// across blues, greens, teals, reds, oranges, yellows, and slate.
let irisGroupSenderNameLightColorHexes: [UInt32] = [
    0x006DA3, 0x067906, 0xC13215, 0x5B6976, 0x2E51FF,
    0x007575, 0x9C5711, 0x3D7406, 0xD00B0B, 0x007A3D,
    0x866118, 0x067953, 0x4B7000, 0xB34209, 0x06792D,
    0x6B6B24, 0xD00B2C, 0x2D7906, 0x32763E, 0x2662D9,
    0x76681E, 0x067462, 0x5E6E0C, 0x077288, 0x2D761E,
]

let irisGroupSenderNameDarkColorHexes: [UInt32] = [
    0x00A7FA, 0x0AB80A, 0xFF6F52, 0x8BA1B6, 0x8599FF,
    0x00B2B2, 0xD5920B, 0x5EB309, 0xFF7070, 0x00B85C,
    0xD68F00, 0x00B87A, 0x74AD00, 0xF57A3D, 0x0AB844,
    0xA4A437, 0xF77389, 0x42B309, 0x4BAF5C, 0x7DA1E8,
    0xB89B0A, 0x09B397, 0x8FAA09, 0x00AED1, 0x43B42D,
]

func irisStartsMessageCluster(
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

func irisIsIncomingGroupUserMessage(_ message: ChatMessageSnapshot, chatKind: ChatKind) -> Bool {
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

enum ChatTimelineCoordinateSpace {
    static let name = "chatTimelineCoordinateSpace"
}

enum ChatTimelineAnchor {
    static let top = "chatTimelineTop"
    static let bottom = "chatTimelineBottom"
}

struct ChatJumpToBottomButton: View {
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

struct ActiveMessageBubbleSwipe {
    let messageId: String
    var offset: CGFloat
    var hasFedHaptic: Bool
}

enum ChatMessageBubbleSwipeMetrics {
    static let threshold: CGFloat = 60
    static let maxOffset: CGFloat = 90
    static let activationDistance: CGFloat = 12
}

final class ChatTimelineInteractionCoordinator: ObservableObject {
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

struct ChatTimelineViewportMinYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct ChatTimelineViewportMaxYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct ChatTimelineTopMinYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = -.greatestFiniteMagnitude

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct ChatTimelineBottomMaxYPreferenceKey: PreferenceKey {
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
struct ChatTimelineContentHeightPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct ChatMessageBubbleFramePreferenceKey: PreferenceKey {
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

struct ChatTimelineDaySeparatorFramePreferenceKey: PreferenceKey {
    static var defaultValue: [String: ChatTimelineDaySeparatorFrame] = [:]

    static func reduce(
        value: inout [String: ChatTimelineDaySeparatorFrame],
        nextValue: () -> [String: ChatTimelineDaySeparatorFrame]
    ) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}

func chatTimelineIsNearBottom(viewportMaxY: CGFloat, bottomMaxY: CGFloat) -> Bool {
    guard viewportMaxY > 0, bottomMaxY.isFinite else {
        return true
    }
    return bottomMaxY <= viewportMaxY + 24
}

func chatTimelineGeometryMatches(_ lhs: CGFloat, _ rhs: CGFloat) -> Bool {
    if lhs == rhs {
        return true
    }
    guard lhs.isFinite, rhs.isFinite else {
        return false
    }
    return abs(lhs - rhs) < 0.5
}

struct ChatMessageRow: View, Equatable {
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
    @State private var showOverflowActions = false
    @State private var hideActionDockTask: DispatchWorkItem?

    private var showActionDock: Bool {
        IrisLayout.usesDesktopChrome && (isHovering || showOverflowActions)
    }

    private var postReactionSuggestions: [String] {
        irisPostReactionSuggestionEmojis(reactions)
    }

    @ViewBuilder
    private func actionDock() -> some View {
        ChatMessageActionDock(
            isOverflowPresented: $showOverflowActions,
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
                            desktopActionDockSlot()
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
                        .accessibilityValue(message.isOutgoing ? irisDeliveryLabel(message.delivery) : "")
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
                            desktopActionDockSlot()
                            Spacer(minLength: SignalConversationLayout.messageDirectionSpacing)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: message.isOutgoing ? .trailing : .leading)
                .contentShape(Rectangle())
                .onHover(perform: updateActionDockHover)
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

#if canImport(AppKit)
    private func desktopActionDockSlot() -> some View {
        actionDock()
            .fixedSize()
            .opacity(showActionDock ? 1 : 0)
            .allowsHitTesting(showActionDock)
            .accessibilityHidden(!showActionDock)
            .onHover(perform: updateActionDockHover)
            .frame(width: ChatMessageActionDock.dockWidth)
    }
#else
    private func desktopActionDockSlot() -> EmptyView { EmptyView() }
#endif

    private func updateActionDockHover(_ hovering: Bool) {
        hideActionDockTask?.cancel()
        if hovering {
            isHovering = true
            return
        }
        let task = DispatchWorkItem {
            if !showOverflowActions {
                isHovering = false
            }
        }
        hideActionDockTask = task
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0, execute: task)
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
struct TruncatableMessageBody: View {
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

func irisMessageBodyFont(for text: String) -> Font {
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

func irisIsEmojiCluster(_ character: Character) -> Bool {
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
