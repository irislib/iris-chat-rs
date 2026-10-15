import Foundation

/// Native editors use UTF-16 offsets. Keep token discovery independent of the
/// view so completion never guesses a replacement range from a stale draft.
struct IrisComposerCompletionToken: Equatable {
    let range: NSRange
    let query: String
    let isClosed: Bool

    static func find(in text: String, selection: NSRange, trigger: Character = ":") -> Self? {
        guard selection.length == 0, selection.location != NSNotFound,
              let caret = Range(selection, in: text)?.lowerBound else { return nil }
        let prefix = text[..<caret]
        let start = prefix.lastIndex(where: \.isWhitespace).map { text.index(after: $0) } ?? text.startIndex
        guard start < caret, text[start] == trigger else { return nil }
        let queryStart = text.index(after: start)
        let isClosed = queryStart < caret && text[text.index(before: caret)] == trigger
        let queryEnd = isClosed ? text.index(before: caret) : caret
        let querySlice = text[queryStart..<queryEnd]
        // Never recursively peel delimiters: :smile:: is ordinary text, and a
        // long pasted run of colons must not consume stack or remove punctuation.
        guard querySlice.prefix(65).count <= 64,
              (!isClosed || !querySlice.isEmpty),
              querySlice.unicodeScalars.allSatisfy(isTokenScalar) else { return nil }
        var end = caret
        if !isClosed {
            var count = querySlice.count
            while end < text.endIndex, text[end].unicodeScalars.allSatisfy(isTokenScalar) {
                count += 1
                guard count <= 64 else { return nil }
                end = text.index(after: end)
            }
            if end < text.endIndex, text[end] == trigger { end = text.index(after: end) }
        }
        return Self(range: NSRange(start..<end, in: text), query: String(querySlice), isClosed: isClosed)
    }

    private static func isTokenScalar(_ scalar: Unicode.Scalar) -> Bool {
        CharacterSet.alphanumerics.contains(scalar) || "_+-".unicodeScalars.contains(scalar)
    }
}

struct IrisEmojiCompletionEntry: Identifiable, Equatable {
    var id: String { emoji }
    let emoji: String
    let shortcode: String
    let aliases: [String]

    init(emoji: String, shortcode: String? = nil, aliases: [String] = []) {
        self.emoji = emoji
        self.shortcode = shortcode ?? Self.normalize(emoji.unicodeScalars.compactMap(\.properties.name).joined(separator: " "))
        self.aliases = aliases.map(Self.normalize)
    }

    static func normalize(_ text: String) -> String {
        text.lowercased().replacingOccurrences(of: "-", with: "_").replacingOccurrences(of: " ", with: "_")
    }

    func exactlyMatches(_ query: String) -> Bool {
        let query = Self.normalize(query)
        return shortcode == query || aliases.contains(query)
    }

    static func search(_ query: String, in entries: [Self], limit: Int = 6) -> [Self] {
        let query = normalize(query)
        guard query.count >= 2, limit > 0 else { return [] }
        return entries.enumerated().compactMap { index, entry -> (Int, Int, Self)? in
            let names = [entry.shortcode] + entry.aliases
            let rank: Int
            if entry.exactlyMatches(query) { rank = 0 }
            else if names.contains(where: { $0.hasPrefix(query) }) { rank = 1 }
            else if names.contains(where: { $0.contains(query) }) { rank = 2 }
            else { return nil }
            return (rank, index, entry)
        }.sorted { ($0.0, $0.1) < ($1.0, $1.1) }.prefix(limit).map(\.2)
    }
}

enum IrisComposerCompletionCommand {
    case previous, next, accept, dismiss
}

struct IrisComposerCompletionContext {
    let text: String
    let selection: NSRange
    let hasMarkedText: Bool
    var isUndoOrRedo = false
    /// The native editor rechecks text/selection before applying this edit and
    /// uses its normal insertion path, preserving undo and draft notifications.
    let replace: (NSRange, String) -> Bool
}
