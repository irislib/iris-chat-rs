import SwiftUI

@MainActor
final class IrisComposerEmojiCompletion: ObservableObject {
    @Published private(set) var results: [IrisEmojiCompletionEntry] = []
    @Published private(set) var selectedIndex = 0
    private var context: IrisComposerCompletionContext?
    private var token: IrisComposerCompletionToken?
    private var dismissedText: String?
    private let entries: [IrisEmojiCompletionEntry]

    init(entries: [IrisEmojiCompletionEntry] = irisComposerEmojiEntries) { self.entries = entries }

    func update(_ context: IrisComposerCompletionContext) {
        self.context = context
        if context.isUndoOrRedo {
            dismiss()
            return
        }
        guard !context.hasMarkedText, context.text != dismissedText,
              let token = IrisComposerCompletionToken.find(in: context.text, selection: context.selection) else {
            clearResults()
            return
        }
        let previousQuery = self.token?.query
        self.token = token
        if token.isClosed {
            if let exact = entries.first(where: { $0.exactlyMatches(token.query) }) {
                insert(exact, trailingSpace: false)
            } else { clearResults() }
            return
        }
        results = IrisEmojiCompletionEntry.search(token.query, in: entries)
        selectedIndex = previousQuery == token.query ? min(selectedIndex, max(0, results.count - 1)) : 0
    }

    func dismiss() {
        dismissedText = context?.text
        clearResults()
    }

    func draftDidChange(_ text: String) {
        if context?.text != text { dismiss() }
    }

    /// Selection changes, focus loss, and cursor movement cancel completion;
    /// another actual edit can reopen it, as in Signal's composer.
    func selectionDidChange(text: String, selection: NSRange) {
        guard let context, context.text == text, context.selection != selection else { return }
        dismiss()
    }

    func handle(_ command: IrisComposerCompletionCommand) -> Bool {
        guard !results.isEmpty else { return false }
        switch command {
        case .previous: selectedIndex = (selectedIndex + results.count - 1) % results.count
        case .next: selectedIndex = (selectedIndex + 1) % results.count
        case .accept: insert(results[selectedIndex])
        case .dismiss: dismiss()
        }
        return true
    }

    func insert(_ entry: IrisEmojiCompletionEntry, trailingSpace: Bool = true) {
        guard let context, let token, !context.hasMarkedText else { return }
        let tailStart = min(NSMaxRange(token.range), context.text.utf16.count)
        let tail = (context.text as NSString).substring(from: tailStart)
        let space = trailingSpace && (tail.isEmpty || tail.first?.isWhitespace == false) ? " " : ""
        clearResults()
        if context.replace(token.range, entry.emoji + space) { IrisRecentEmojiStore.remember(entry.emoji) }
    }

    private func clearResults() {
        if !results.isEmpty { results = [] }
        selectedIndex = 0
        token = nil
    }
}

/// Shortcodes and search words come from the existing cross-platform catalog.
private let irisComposerEmojiEntries: [IrisEmojiCompletionEntry] = {
    irisUniqueEmojis(irisEmojiCatalog().flatMap(\.emojis)).map { emoji in
        let aliases = (irisEmojiSearchAliasMap[emoji] ?? "").split(separator: " ").map(String.init)
        return IrisEmojiCompletionEntry(emoji: emoji, shortcode: aliases.first, aliases: aliases)
    }
}()

struct IrisComposerEmojiSuggestions: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var completion: IrisComposerEmojiCompletion

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(completion.results.enumerated()), id: \.element.id) { index, entry in
                Button { completion.insert(entry) } label: {
                    HStack(spacing: 10) {
                        Text(entry.emoji).font(.system(size: 24)).frame(width: 32)
                        Text(":\(entry.shortcode):").font(.callout).lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(palette.textPrimary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(index == completion.selectedIndex ? palette.accent.opacity(0.14) : Color.clear)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisUnpressed)
                .accessibilityLabel("\(entry.emoji) \(entry.shortcode.replacingOccurrences(of: "_", with: " "))")
                .accessibilityIdentifier("chatEmojiCompletion-\(entry.shortcode)")
                #if os(macOS)
                .environment(\.irisKeyboardButtonIsTabStop, false)
                #endif
            }
        }
        .padding(4)
        .frame(maxWidth: 340)
        .background(palette.panel, in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatEmojiSuggestions")
    }
}
