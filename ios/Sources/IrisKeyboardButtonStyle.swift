import SwiftUI

extension View {
    @ViewBuilder
    func irisDesktopFocusSection() -> some View {
        #if os(macOS)
        self.focusSection()
        #else
        self
        #endif
    }
}

#if os(macOS)
private struct IrisKeyboardButtonTabStopKey: EnvironmentKey {
    static let defaultValue = true
}

extension EnvironmentValues {
    var irisKeyboardButtonIsTabStop: Bool {
        get { self[IrisKeyboardButtonTabStopKey.self] }
        set { self[IrisKeyboardButtonTabStopKey.self] = newValue }
    }
}

/// Keep the existing button visuals while making app controls reachable without
/// changing the system-wide all-controls keyboard navigation preference.
struct IrisKeyboardButtonStyle<Visual: ButtonStyle>: PrimitiveButtonStyle {
    let visual: Visual
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.irisKeyboardButtonIsTabStop) private var isTabStop
    @FocusState private var focused: Bool

    func makeBody(configuration: Configuration) -> some View {
        Button(configuration)
            .buttonStyle(visual)
            .focusable(isEnabled && isTabStop, interactions: .edit)
            .focusEffectDisabled()
            .focused($focused)
            .overlay {
                if focused && isEnabled && isTabStop {
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(Color.accentColor, lineWidth: 2)
                        .allowsHitTesting(false)
                }
            }
            .onKeyPress(keys: [.return, .space], phases: .down) { press in
                guard isEnabled && isTabStop,
                      press.modifiers.intersection([.command, .option, .control, .shift]).isEmpty
                else { return .ignored }
                configuration.trigger()
                return .handled
            }
    }
}

#endif
