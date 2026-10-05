import Foundation
import SwiftUI

// Caps tall message bubbles behind a Show more/less toggle.
// Only limit lines when the expansion control is available: wrapping
// can exceed 14 lines below the character/newline thresholds, especially
// at narrow widths or large text sizes. Keep intrinsic text sizing so
// ordinary short messages do not grow into oversized bubbles.
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
                .lineLimit(needsTruncation && !isExpanded ? collapsedLineLimit : nil)
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
        .irisHoverPointer()
        .accessibilityIdentifier("chatMessageBodyToggle")
    }
}

extension View {
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
