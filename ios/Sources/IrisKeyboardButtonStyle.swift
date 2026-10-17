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
import AppKit

/// Mouse focus still works, but only keyboard navigation needs a focus outline.
final class IrisKeyboardFocusVisibility: ObservableObject {
    static let shared = IrisKeyboardFocusVisibility()
    @Published private(set) var visible = false
    private var monitor: Any?

    private init() {
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] event in
            let visible = event.type == .keyDown
            if self?.visible != visible { self?.visible = visible }
            return event
        }
    }

    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }
}

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
    @ObservedObject private var keyboardFocus = IrisKeyboardFocusVisibility.shared
    @FocusState private var focused: Bool

    func makeBody(configuration: Configuration) -> some View {
        Button(configuration)
            .buttonStyle(visual)
            .focusable(isEnabled && isTabStop, interactions: .edit)
            .focusEffectDisabled()
            .focused($focused)
            .overlay {
                if focused && isEnabled && isTabStop && keyboardFocus.visible {
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
