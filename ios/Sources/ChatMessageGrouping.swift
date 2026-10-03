import Foundation

let irisMessageClusterGapSecs: UInt64 = 180

func irisStartsMessageCluster(
    previous: ChatMessageSnapshot?,
    message: ChatMessageSnapshot,
    chatKind: ChatKind
) -> Bool {
    guard let previous else {
        return true
    }
    if previous.kind == .system || message.kind == .system || !previous.reactions.isEmpty {
        return true
    }
    if !irisSameTimelineDay(previous.createdAtSecs, message.createdAtSecs) {
        return true
    }
    if previous.isOutgoing != message.isOutgoing {
        return true
    }
    if chatKind == .group && !message.isOutgoing && !irisSameMessageAuthor(previous, message) {
        return true
    }
    guard message.createdAtSecs >= previous.createdAtSecs else { return true }
    return message.createdAtSecs - previous.createdAtSecs >= irisMessageClusterGapSecs
}

private func irisSameMessageAuthor(_ lhs: ChatMessageSnapshot, _ rhs: ChatMessageSnapshot) -> Bool {
    (lhs.authorOwnerPubkeyHex ?? lhs.author) == (rhs.authorOwnerPubkeyHex ?? rhs.author)
}

func irisShowsMessageFooter(
    message: ChatMessageSnapshot,
    next: ChatMessageSnapshot?,
    chatKind: ChatKind
) -> Bool {
    guard let next,
          !irisStartsMessageCluster(previous: message, message: next, chatKind: chatKind),
          message.createdAtSecs / 60 == next.createdAtSecs / 60,
          message.expiresAtSecs == nil else { return true }
    guard message.isOutgoing else { return false }
    switch message.delivery {
    case .queued, .pending, .failed: return true
    case .sent, .received, .seen: return message.delivery != next.delivery
    }
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
    return !irisSameMessageAuthor(previous, message)
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
    return !irisSameMessageAuthor(message, next)
}
