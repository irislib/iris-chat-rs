import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

struct MessageInfoSheet: View {
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
func synthesizeMessageRumorJson(
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

struct ParticipantInfo {
    let ownerPubkeyHex: String?
    let name: String
    let pictureUrl: String?
    let isMe: Bool
}

func participantInfo(
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

func recipientInfo(
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

func reactorInfo(_ reactor: MessageReactor, chat: CurrentChatSnapshot?) -> ParticipantInfo {
    participantInfo(
        ownerPubkeyHex: reactor.author,
        displayName: reactor.displayName,
        pictureUrl: reactor.pictureUrl,
        chat: chat
    )
}

func directRecipientInfo(_ chat: CurrentChatSnapshot) -> ParticipantInfo {
    participantInfo(
        ownerPubkeyHex: chat.chatId,
        displayName: chat.displayName,
        pictureUrl: chat.pictureUrl,
        chat: chat
    )
}

func nonEmptyTrimmed(_ value: String) -> String? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}

struct MessageInfoReactorRow: View {
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

struct MessageInfoSection<Content: View>: View {
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

struct MessageInfoValueRow: View {
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

struct MessageInfoMultiValueRow: View {
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

struct MessageInfoCopyListRow: View {
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

struct MessageInfoRecipientRow: View {
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

struct MessageInfoUserRow<Trailing: View>: View {
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

struct ChatMessageBubbleShape: Shape {
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

struct IrisDeliveryGlyph: View {
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
            Image(systemName: "paperplane.fill")
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

struct ChatMessageActionDock: View {
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
                    .font(.system(size: 13, weight: .semibold))
                    .frame(width: ChatMessageActionDock.buttonWidth, height: ChatMessageActionDock.buttonHeight)
                    .contentShape(Rectangle())
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
                    .font(.system(size: 14, weight: .bold))
                    .frame(width: ChatMessageActionDock.buttonWidth, height: ChatMessageActionDock.buttonHeight)
                    // Menu's default hit area on macOS shrinks to the glyph;
                    // contentShape pushes it back out to the full button frame.
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
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
                .font(.system(size: 13, weight: .semibold))
                .frame(width: ChatMessageActionDock.buttonWidth, height: ChatMessageActionDock.buttonHeight)
                .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier(identifier)
    }

    fileprivate static let buttonWidth: CGFloat = 30
    fileprivate static let buttonHeight: CGFloat = 28
}

let quickReactionEmojis: [String] = ["❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"]
