import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

func irisPostReactionSuggestionEmojis(_ reactions: [MessageReactionSnapshot]) -> [String] {
    irisUniqueEmojis(reactions.map(\.emoji))
}

func irisUniqueEmojis(_ emojis: [String]) -> [String] {
    var seen = Set<String>()
    var result: [String] = []
    for emoji in emojis {
        let trimmed = emoji.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, seen.insert(trimmed).inserted else {
            continue
        }
        result.append(trimmed)
    }
    return result
}

func irisReactionQuickChoices() -> [String] {
    quickReactionEmojis
}

enum IrisRecentEmojiStore {
    private static let key = "iris.recentReactionEmojis"
    private static let limit = 16

    static func emojis() -> [String] {
        guard let values = UserDefaults.standard.stringArray(forKey: key) else {
            return []
        }
        return irisUniqueEmojis(values)
    }

    static func remember(_ emoji: String) {
        let trimmed = emoji.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        let values = [trimmed] + emojis().filter { $0 != trimmed }
        UserDefaults.standard.set(Array(values.prefix(limit)), forKey: key)
    }
}

func irisEmojiMatchesSearch(_ emoji: String, category: String, query: String) -> Bool {
    let tokens = irisNormalizeEmojiSearchText(query)
        .split(separator: " ")
        .map(String.init)
    guard !tokens.isEmpty else {
        return true
    }

    let scalarNames = emoji.unicodeScalars
        .compactMap { $0.properties.name }
        .joined(separator: " ")
    let aliases = irisEmojiSearchAliasMap[emoji] ?? ""
    let haystack = irisNormalizeEmojiSearchText("\(emoji) \(category) \(scalarNames) \(aliases)")
    return tokens.allSatisfy { haystack.contains($0) }
}

func irisNormalizeEmojiSearchText(_ value: String) -> String {
    value
        .folding(options: [.caseInsensitive, .diacriticInsensitive], locale: .current)
        .replacingOccurrences(of: "_", with: " ")
        .replacingOccurrences(of: "-", with: " ")
        .lowercased()
}

let irisEmojiSearchAliasMap: [String: String] = {
    var map: [String: String] = [:]
    for entry in irisEmojiSearchAliases() {
        map[entry.emoji] = entry.keywords
    }
    return map
}()

struct ChatMessageActionsSheet: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let bodyText: String
    let onReact: (String) -> Void
    let onShowFullReactionPicker: () -> Void
    let onReply: () -> Void
    let onForward: () -> Void
    let onCopy: () -> Void
    let onInfo: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            quickReactionRow
            previewCard
            VStack(spacing: 0) {
                actionRow(icon: "arrowshape.turn.up.left", label: "Reply", action: onReply)
                actionRow(icon: "arrowshape.turn.up.right", label: "Forward", action: onForward)
                actionRow(icon: "doc.on.doc", label: "Copy", action: onCopy)
                actionRow(icon: "info.circle", label: "Info", action: onInfo)
                actionRow(icon: "trash", label: "Delete locally", destructive: true, action: onDelete)
            }
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(palette.panel)
            )
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.top, 14)
        .padding(.bottom, 6)
        .background(palette.background)
        .accessibilityIdentifier("messageActionsSheet")
    }

    private var quickReactionRow: some View {
        HStack(spacing: 4) {
            ForEach(irisReactionQuickChoices(), id: \.self) { emoji in
                Button {
                    IrisRecentEmojiStore.remember(emoji)
                    onReact(emoji)
                } label: {
                    Text(emoji)
                        .font(.system(size: 26))
                        .frame(maxWidth: .infinity)
                        .frame(height: 40)
                }
                .buttonStyle(.irisPlain)
            }
            Button(action: onShowFullReactionPicker) {
                Image(systemName: "plus.circle")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .frame(height: 40)
            }
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("messageReactButton")
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 6)
        .background(
            Capsule(style: .continuous)
                .fill(palette.panel)
        )
    }

    private var previewText: String {
        if !bodyText.isEmpty { return bodyText }
        if let first = message.attachments.first {
            return first.filename.isEmpty ? "Attachment" : first.filename
        }
        return ""
    }

    @ViewBuilder
    private var previewCard: some View {
        if !previewText.isEmpty || !message.attachments.isEmpty || !message.reactions.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text(message.author)
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                if !previewText.isEmpty {
                    Text(previewText)
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                }
                if !message.attachments.isEmpty,
                   previewText != message.attachments.first?.filename {
                    Text(message.attachments.count == 1 ? "1 attachment" : "\(message.attachments.count) attachments")
                        .font(.system(.caption2, design: .rounded, weight: .medium))
                        .foregroundStyle(palette.muted)
                }
                if !message.reactions.isEmpty {
                    ReactionRow(reactions: message.reactions)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(palette.panel)
            )
        }
    }

    private func actionRow(
        icon: String,
        label: String,
        destructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: icon)
                    .font(.system(size: 18, weight: .semibold))
                    .frame(width: 22)
                Text(label)
                    .font(.system(.body, design: .rounded, weight: .medium))
                Spacer()
            }
            .foregroundStyle(destructive ? Color.red : palette.textPrimary)
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
    }
}

func irisEmojiCategorySymbol(for name: String) -> String {
    switch name {
    case "Smileys": return "face.smiling"
    case "Hearts": return "heart.fill"
    case "Hands": return "hand.thumbsup.fill"
    case "Animals": return "pawprint.fill"
    case "Food": return "fork.knife"
    case "Activities": return "sportscourt"
    case "Travel": return "airplane"
    case "Objects": return "lightbulb.fill"
    case "Symbols": return "sparkles"
    default: return "square.grid.2x2.fill"
    }
}

struct IrisEmojiPicker: View {
    @Environment(\.irisPalette) private var palette
    let suggestedEmojis: [String]
    let onPick: (String) -> Void
    let onClose: (() -> Void)?

    @State private var query: String = ""
    @State private var recentEmojis: [String] = IrisRecentEmojiStore.emojis()

    init(
        suggestedEmojis: [String] = [],
        onClose: (() -> Void)? = nil,
        onPick: @escaping (String) -> Void
    ) {
        self.suggestedEmojis = suggestedEmojis
        self.onClose = onClose
        self.onPick = onPick
    }

    private static let categories: [(String, String, [String])] = irisEmojiCatalog().map {
        ($0.name, irisEmojiCategorySymbol(for: $0.name), $0.emojis)
    }

    private var filteredCategories: [(String, String, [String])] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else {
            var sections: [(String, String, [String])] = []
            let postEmojis = irisUniqueEmojis(suggestedEmojis)
            if !postEmojis.isEmpty {
                sections.append(("This message", "bubble.left.and.bubble.right.fill", postEmojis))
            }
            let recent = irisUniqueEmojis(recentEmojis).filter { !postEmojis.contains($0) }
            if !recent.isEmpty {
                sections.append(("Recent", "clock.fill", recent))
            }
            return sections + Self.categories
        }
        return Self.categories.compactMap { name, icon, list in
            let hits = list.filter { irisEmojiMatchesSearch($0, category: name, query: q) }
            return hits.isEmpty ? nil : (name, icon, hits)
        }
    }

    private let columns = [GridItem(.adaptive(minimum: 40), spacing: 4)]

    private func pick(_ emoji: String) {
        IrisRecentEmojiStore.remember(emoji)
        recentEmojis = IrisRecentEmojiStore.emojis()
        onPick(emoji)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                HStack {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(palette.muted)
                    TextField("Search", text: $query)
                        .textFieldStyle(.plain)
                        .autocorrectionDisabled()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(palette.panel)
                )
                .frame(maxWidth: .infinity)

                if let onClose {
                    IrisModalCloseButton(action: onClose)
                        .accessibilityIdentifier("reactionPickerCloseButton")
                }
            }
            .padding(10)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12, pinnedViews: [.sectionHeaders]) {
                    ForEach(filteredCategories, id: \.0) { name, icon, list in
                        Section {
                            LazyVGrid(columns: columns, spacing: 4) {
                                ForEach(Array(list.enumerated()), id: \.offset) { _, emoji in
                                    Button {
                                        pick(emoji)
                                    } label: {
                                        Text(emoji)
                                            .font(.system(size: 26))
                                            .frame(width: 36, height: 36)
                                    }
                                    .buttonStyle(.irisPlain)
                                }
                            }
                            .padding(.horizontal, 10)
                        } header: {
                            HStack(spacing: 6) {
                                Image(systemName: icon)
                                    .font(.system(size: 11, weight: .semibold))
                                Text(name)
                                    .font(.system(.caption, weight: .semibold))
                            }
                            .foregroundStyle(palette.muted)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(palette.background)
                        }
                    }
                }
                .padding(.bottom, 10)
            }
        }
        .frame(minWidth: 280, idealWidth: 320, minHeight: 320, idealHeight: 420)
        .background(palette.background)
        .onAppear {
            recentEmojis = IrisRecentEmojiStore.emojis()
        }
    }
}

struct IrisTypingIndicatorRow: View {
    @Environment(\.irisPalette) private var palette
    let indicators: [TypingIndicatorSnapshot]

    private var label: String {
        guard let first = indicators.first else { return "" }
        if indicators.count == 1 {
            return "\(first.displayName) is typing"
        }
        return "\(first.displayName) and \(indicators.count - 1) more are typing"
    }

    var body: some View {
        HStack(spacing: 8) {
            HStack(spacing: 4) {
                Circle().frame(width: 5, height: 5)
                Circle().frame(width: 5, height: 5)
                Circle().frame(width: 5, height: 5)
            }
            .foregroundStyle(palette.muted)

            Text(label)
                .font(.system(.caption, design: .rounded, weight: .medium))
                .foregroundStyle(palette.muted)
                .lineLimit(1)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Capsule(style: .continuous).fill(palette.toolbar.opacity(0.9)))
        .frame(maxWidth: 260, alignment: .leading)
        .accessibilityIdentifier("chatTypingIndicator")
    }
}

struct MessageReactorsSheet: View {
    @Environment(\.irisPalette) private var palette
    let reactors: [MessageReactor]
    let chat: CurrentChatSnapshot?
    @ObservedObject var manager: AppManager
    let onClose: () -> Void

    private var visibleReactors: [MessageReactor] {
        reactors.filter { !$0.emoji.isEmpty }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(visibleReactors, id: \.author) { reactor in
                        MessageInfoReactorRow(
                            info: reactorInfo(reactor, chat: chat),
                            emoji: reactor.emoji,
                            manager: manager,
                            onTap: openPerson
                        )
                        .padding(.horizontal, 18)
                    }
                }
                .padding(.vertical, 8)
                .frame(maxWidth: IrisLayout.scrollMaxWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
            }
            .background(palette.background)
            .navigationTitle("Reactions")
#if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    IrisModalCloseButton(action: onClose)
                        .accessibilityIdentifier("messageReactorsCloseButton")
                }
            }
#elseif os(macOS)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    IrisModalCloseButton(action: onClose)
                }
            }
#endif
        }
        .accessibilityIdentifier("messageReactorsSheet")
        .irisModalSurface()
    }

    private func openPerson(_ info: ParticipantInfo) {
        guard let owner = info.ownerPubkeyHex, !owner.isEmpty, !info.isMe else { return }
        onClose()
        manager.dispatch(.createChat(peerInput: owner))
    }
}

struct ReactionRow: View {
    @Environment(\.irisPalette) private var palette
    let reactions: [MessageReactionSnapshot]
    var onTap: (() -> Void)? = nil

    @ViewBuilder
    var body: some View {
        let pills = HStack(spacing: 0) {
            ForEach(reactions, id: \.emoji) { reaction in
                HStack(spacing: 2) {
                    Text(reaction.emoji)
                        .font(.system(size: 14, weight: .bold))
                    if reaction.count > 1 {
                        Text("\(reaction.count)")
                            .font(.system(size: 12, weight: .bold, design: .monospaced))
                            .foregroundStyle(palette.muted)
                    }
                }
                .padding(.horizontal, 7)
                .frame(height: SignalConversationLayout.reactionPillHeight)
                .background(
                    Capsule(style: .continuous)
                        .fill(reaction.reactedByMe ? palette.panel : palette.panelAlt)
                )
                .overlay(
                    Capsule(style: .continuous)
                        .strokeBorder(palette.background, lineWidth: 1)
                )
            }
        }

        if let onTap {
            Button(action: onTap) { pills }
                .buttonStyle(.irisPlain)
                .accessibilityHint("Tap to see who reacted")
                .accessibilityIdentifier("chatReactionRow")
        } else {
            pills
                .accessibilityIdentifier("chatReactionRow")
        }
    }
}
