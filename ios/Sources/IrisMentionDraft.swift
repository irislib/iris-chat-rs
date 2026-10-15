import Foundation

/// The editor displays names, while drafts and outgoing messages retain the
/// selected identity. Editing inside a mention deliberately turns it into
/// ordinary text; matching a name later can never recreate a notification.
struct IrisMentionDraft {
    struct Span: Equatable {
        var range: NSRange
        let label: String
        let token: String
        let owner: String
    }

    private(set) var spans: [Span] = []

    mutating func restore(wireText: String, encodedSpans: [Span]) -> String {
        let text = NSMutableString(string: wireText)
        spans = []
        var shift = 0
        for span in encodedSpans.sorted(by: { $0.range.location < $1.range.location }) {
            let range = NSRange(location: span.range.location + shift, length: span.range.length)
            guard range.location >= 0, NSMaxRange(range) <= text.length else { continue }
            text.replaceCharacters(in: range, with: span.label)
            let visible = NSRange(location: range.location, length: span.label.utf16.count)
            spans.append(Span(range: visible, label: span.label, token: span.token, owner: span.owner))
            shift += visible.length - range.length
        }
        return text as String
    }

    mutating func textDidChange(from old: String, to new: String) {
        guard old != new else { return }
        let before = Array(old.utf16), after = Array(new.utf16)
        var prefix = 0
        while prefix < min(before.count, after.count), before[prefix] == after[prefix] { prefix += 1 }
        var suffix = 0
        while suffix < min(before.count, after.count) - prefix,
              before[before.count - suffix - 1] == after[after.count - suffix - 1] { suffix += 1 }
        let oldEnd = before.count - suffix
        let delta = after.count - before.count
        spans = spans.compactMap { span in
            // Appending a word character changes the visible name, so the old
            // identity must not silently survive that edit.
            if NSMaxRange(span.range) == prefix, prefix < after.count,
               let scalar = Unicode.Scalar(after[prefix]), CharacterSet.alphanumerics.contains(scalar) { return nil }
            if NSMaxRange(span.range) <= prefix { return span }
            if span.range.location >= oldEnd {
                var moved = span
                moved.range.location += delta
                return moved
            }
            return nil
        }
    }

    mutating func insert(label: String, token: String, owner: String, at location: Int) {
        let range = NSRange(location: location, length: label.utf16.count)
        spans.removeAll { NSIntersectionRange($0.range, range).length > 0 }
        spans.append(Span(range: range, label: label, token: token, owner: owner))
        spans.sort { $0.range.location < $1.range.location }
    }

    func wireText(from visible: String) -> String {
        let text = NSMutableString(string: visible)
        for span in spans.reversed() {
            guard NSMaxRange(span.range) <= text.length,
                  text.substring(with: span.range) == span.label else { continue }
            text.replaceCharacters(in: span.range, with: span.token)
        }
        return text as String
    }
}
