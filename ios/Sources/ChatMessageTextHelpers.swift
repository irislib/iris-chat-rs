import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

func isAnimatedImage(data: Data, filename: String) -> Bool {
    if filename.lowercased().hasSuffix(".gif") {
        return true
    }
    let gifHeader = [UInt8](data.prefix(6))
    return gifHeader == Array("GIF87a".utf8) || gifHeader == Array("GIF89a".utf8)
}

struct ReplyPreview {
    let author: String
    let body: String
}

struct ReplyParsedMessage {
    let reply: ReplyPreview?
    let body: String
}

func replyEncodedMessage(reply: ChatMessageSnapshot?, text: String) -> String {
    guard let reply else {
        return text
    }
    let snippet = replySnippet(for: reply)
    return "\(replyMessagePrefix)\(reply.author): \(snippet)\n\n\(text)"
}

func parseReplyEncodedMessage(_ text: String) -> ReplyParsedMessage {
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

func replySnippet(for message: ChatMessageSnapshot) -> String {
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

let replyMessagePrefix = "↩ "

// Signal-style link styling: force the body text colour (the
// foreground attribute overrides SwiftUI's default link tint) and
// underline. Without the explicit foregroundColor, AttributedString
// still rendered links in the system accent / blue on iOS, which
// shifted hue between incoming and outgoing bubbles.
func linkedMessageAttributedString(_ text: String, foreground: Color) -> AttributedString {
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

func messageURLMatches(in text: String) -> [(range: Range<String.Index>, url: URL)] {
    var matches: [(Range<String.Index>, URL)] = []
    let pattern = #"(?i)(?:^|(?<=[\s(\[{<]))((?:https?://|www\.)[^\s<]+|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}(?::[0-9]{2,5})?(?:/[^\s<]*)?)"#
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

let messageURLTrailingPunctuation = CharacterSet(charactersIn: ".,;:!?)]")

func copyableMessageText(_ message: ChatMessageSnapshot) -> String {
    var pieces: [String] = []
    if !message.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        pieces.append(message.body)
    }
    pieces.append(contentsOf: message.attachments.map(\.htreeUrl))
    return pieces.joined(separator: "\n")
}

func forwardableMessageText(_ message: ChatMessageSnapshot) -> String {
    let parsed = parseReplyEncodedMessage(message.body)
    var pieces: [String] = []
    let body = parsed.body.trimmingCharacters(in: .whitespacesAndNewlines)
    if !body.isEmpty {
        pieces.append(body)
    }
    pieces.append(contentsOf: message.attachments.map(forwardableAttachmentText).filter { !$0.isEmpty })
    return pieces.joined(separator: "\n")
}

func forwardableAttachmentText(_ attachment: MessageAttachmentSnapshot) -> String {
    attachment.htreeUrl.trimmingCharacters(in: .whitespacesAndNewlines)
}

func messageInfoText(_ message: ChatMessageSnapshot, chat: CurrentChatSnapshot? = nil) -> String {
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

func messageInfoDirection(_ message: ChatMessageSnapshot) -> String {
    if message.kind == .system {
        return "System message"
    }
    return message.isOutgoing ? "Sent message" : "Received message"
}

func messageInfoKind(_ message: ChatMessageSnapshot) -> String {
    switch message.kind {
    case .system:
        return "System"
    case .user:
        return message.isOutgoing ? "Sent" : "Received"
    }
}

func messageInfoRecipientName(_ ownerPubkeyHex: String, chat: CurrentChatSnapshot?) -> String {
    if let chat, chat.kind == .direct && chat.chatId == ownerPubkeyHex {
        return chat.displayName
    }
    return shortNpub(ownerPubkeyHex)
}

func shortNpub(_ pubkeyInput: String) -> String {
    let npub = peerInputToNpub(input: pubkeyInput)
    let value = npub.isEmpty ? pubkeyInput : npub
    return shortMessageIdentifier(value)
}

func prettyTransportChannel(_ channel: String) -> String {
    let prefix = "message server: "
    if channel.hasPrefix(prefix) {
        return String(channel.dropFirst(prefix.count))
    }
    if channel == "message servers" {
        return "Message server"
    }
    return channel
}

func messageInfoDateTime(_ secs: UInt64) -> String {
    messageInfoDateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(secs)))
}

let messageInfoDateFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .medium
    formatter.timeStyle = .short
    return formatter
}()

func shortMessageIdentifierList(_ values: [String]) -> String {
    values.map(shortMessageIdentifier).joined(separator: ", ")
}

func shortMessageIdentifier(_ value: String) -> String {
    guard value.count > 16 else {
        return value
    }
    return "\(value.prefix(8))...\(value.suffix(8))"
}
