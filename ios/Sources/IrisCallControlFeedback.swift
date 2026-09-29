import SwiftUI

extension View {
    func irisCallControlHelp(_ text: String, onDarkBackground: Bool = false) -> some View {
#if os(macOS)
        modifier(IrisCallControlFeedback(text: text, onDarkBackground: onDarkBackground))
#else
        self
#endif
    }
}

#if os(macOS)
private struct IrisCallControlFeedback: ViewModifier {
    let text: String
    let onDarkBackground: Bool
    @Environment(\.isEnabled) private var isEnabled
    @State private var hovered = false
    @FocusState private var focused: Bool

    func body(content: Content) -> some View {
        content
            .focused($focused)
            .focusEffectDisabled()
            .background {
                RoundedRectangle(cornerRadius: 12)
                    .fill((onDarkBackground ? Color.white : .primary).opacity(isEnabled && hovered ? 0.12 : 0))
            }
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(onDarkBackground ? Color.white : .accentColor, lineWidth: focused ? 2 : 0)
                    .allowsHitTesting(false)
            }
            .onHover { hovered = $0 }
            .help(text)
    }
}
#endif
