import SwiftUI

struct IrisMentionCandidate: Identifiable, Equatable {
    let id: String
    let name: String
    let token: String
}

struct IrisMentionQuery: Equatable {
    let range: NSRange
    let query: String

    static func find(in text: String, selection: NSRange) -> Self? {
        guard selection.length == 0, let caret = Range(selection, in: text)?.lowerBound,
              let start = text[..<caret].lastIndex(of: "@"),
              start == text.startIndex || text[text.index(before: start)].isWhitespace else { return nil }
        let query = text[text.index(after: start)..<caret]
        guard query.prefix(65).count <= 64,
              query.allSatisfy({ $0.isLetter || $0.isNumber || " _-".contains($0) }) else { return nil }
        var end = caret
        var count = query.count
        while end < text.endIndex, text[end].isLetter || text[end].isNumber || "_-".contains(text[end]) {
            count += 1
            guard count <= 64 else { return nil }
            end = text.index(after: end)
        }
        return Self(range: NSRange(start..<end, in: text), query: String(query))
    }
}

@MainActor
final class IrisComposerMentionCompletion: ObservableObject {
    @Published private(set) var results: [IrisMentionCandidate] = []
    @Published private(set) var selectedIndex = 0
    private var context: IrisComposerCompletionContext?
    private var token: IrisMentionQuery?
    private var dismissedText: String?
    var onInsert: (IrisMentionCandidate, String, Int) -> Void = { _, _, _ in }

    func update(_ context: IrisComposerCompletionContext, candidates: [IrisMentionCandidate]) {
        self.context = context
        guard !context.hasMarkedText, !context.isUndoOrRedo, context.text != dismissedText,
              let token = IrisMentionQuery.find(in: context.text, selection: context.selection) else {
            clearResults(); return
        }
        let previous = self.token
        self.token = token
        results = Array(candidates.filter { token.query.isEmpty || $0.name.localizedCaseInsensitiveContains(token.query) }.prefix(6))
        selectedIndex = previous?.query == token.query ? min(selectedIndex, max(0, results.count - 1)) : 0
    }

    func dismiss() { dismissedText = context?.text; clearResults() }
    func draftDidChange(_ text: String) { if context?.text != text { dismiss() } }
    func selectionDidChange(text: String, selection: NSRange) {
        if let context, context.text == text, context.selection != selection { dismiss() }
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
    func insert(_ person: IrisMentionCandidate) {
        guard let context, let token, !context.hasMarkedText, !person.token.isEmpty else { return }
        let tail = (context.text as NSString).substring(from: NSMaxRange(token.range))
        let space = tail.isEmpty || tail.first?.isWhitespace == false ? " " : ""
        let label = "@" + person.name
        clearResults()
        if context.replace(token.range, label + space) { onInsert(person, label, token.range.location) }
    }
    private func clearResults() {
        if !results.isEmpty { results = [] }
        selectedIndex = 0; token = nil
    }
}

struct IrisComposerMentionSuggestions: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var completion: IrisComposerMentionCompletion
    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(completion.results.enumerated()), id: \.element.id) { index, person in
                Button { completion.insert(person) } label: {
                    HStack(spacing: 10) {
                        Image(systemName: "person.fill").frame(width: 28)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(person.name).font(.callout).lineLimit(1)
                            if completion.results.filter({ $0.name == person.name }).count > 1 {
                                Text(String(person.id.prefix(8))).font(.caption2).foregroundStyle(palette.muted)
                            }
                        }
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(palette.textPrimary)
                    .padding(.horizontal, 10).padding(.vertical, 7)
                    .background(index == completion.selectedIndex ? palette.accent.opacity(0.14) : Color.clear)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisUnpressed)
                .accessibilityLabel(completion.results.filter { $0.name == person.name }.count > 1
                    ? "Mention \(person.name), \(String(person.id.prefix(8)))" : "Mention \(person.name)")
                .accessibilityIdentifier("chatMentionCompletion-\(person.id)")
                #if os(macOS)
                .environment(\.irisKeyboardButtonIsTabStop, false)
                #endif
            }
        }
        .padding(4).frame(maxWidth: 340)
        .background(palette.panel, in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatMentionSuggestions")
    }
}
