import Foundation

/// Older clients put a display name and a shortened preview in the reply.
struct ReplyQuoteLookup {
    let quote: ReplyPreview
    private(set) var uniqueSnippetID: String?
    private var ambiguous = false

    init(quote: ReplyPreview) { self.quote = quote }

    /// Pages and rows are searched newest first, strictly before the reply.
    /// Prefer the sender label. If it has changed, only a unique snippet in
    /// the entire preceding history is safe to use.
    mutating func match(in messages: ArraySlice<ChatMessageSnapshot>) -> String? {
        for message in messages.reversed() where !message.deletedForEveryone {
            guard matchesSnippet(of: message) else { continue }
            if message.author == quote.author { return message.id }
            if let previous = uniqueSnippetID, previous != message.id {
                ambiguous = true
                uniqueSnippetID = nil
            } else if !ambiguous {
                uniqueSnippetID = message.id
            }
        }
        return nil
    }

    private func matchesSnippet(of message: ChatMessageSnapshot) -> Bool {
        func matches(_ version: ChatMessageSnapshot) -> Bool {
            // The sender can truncate directly after a space. The quote parser
            // trims the header, so normalize the local preview the same way.
            replySnippet(for: version).trimmingCharacters(in: .whitespacesAndNewlines) == quote.body
        }
        if matches(message) { return true }
        for edit in message.editHistory where edit.body != message.body {
            var previous = message
            previous.body = edit.body
            if matches(previous) { return true }
        }
        return false
    }
}
