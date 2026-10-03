import SwiftUI

#if os(macOS)
private struct DesktopComposerFocusRequestKey: EnvironmentKey {
    static let defaultValue: Binding<UUID?> = .constant(nil)
}

extension EnvironmentValues {
    var desktopComposerFocusRequest: Binding<UUID?> {
        get { self[DesktopComposerFocusRequestKey.self] }
        set { self[DesktopComposerFocusRequestKey.self] = newValue }
    }
}

struct DesktopComposerKeyboardFocus: ViewModifier {
    @Environment(\.desktopComposerFocusRequest) private var request
    @Binding var isFocused: Bool

    func body(content: Content) -> some View {
        content
            .onAppear(perform: applyRequest)
            .onChange(of: request.wrappedValue) { _, _ in applyRequest() }
    }

    private func applyRequest() {
        guard request.wrappedValue != nil else { return }
        isFocused = true
        request.wrappedValue = nil
    }
}

#endif
