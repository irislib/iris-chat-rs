import SwiftUI

#if os(macOS)
import AppKit

private struct DesktopChatListFocusRequestKey: EnvironmentKey {
    static let defaultValue: Binding<UUID?> = .constant(nil)
}

private struct DesktopComposerFocusRequestKey: EnvironmentKey {
    static let defaultValue: Binding<UUID?> = .constant(nil)
}

extension EnvironmentValues {
    var desktopChatListFocusRequest: Binding<UUID?> {
        get { self[DesktopChatListFocusRequestKey.self] }
        set { self[DesktopChatListFocusRequestKey.self] = newValue }
    }

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

/// Window-scoped section shortcuts also work while the native text editor owns focus.
struct DesktopChatSectionShortcuts: NSViewRepresentable {
    let onChatList: () -> Void
    let onComposer: () -> Void

    func makeNSView(context: Context) -> ShortcutView { ShortcutView() }
    func updateNSView(_ view: ShortcutView, context: Context) {
        view.onChatList = onChatList
        view.onComposer = onComposer
    }

    final class ShortcutView: NSView {
        var onChatList: (() -> Void)?
        var onComposer: (() -> Void)?
        private var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, event.window === self.window,
                      event.modifierFlags.contains(.command),
                      event.modifierFlags.intersection([.control, .option]).isEmpty,
                      event.charactersIgnoringModifiers?.lowercased() == "t" || event.keyCode == 97
                else { return event }
                if event.modifierFlags.contains(.shift) { self.onComposer?() }
                else { self.onChatList?() }
                return nil
            }
        }

        deinit { if let monitor { NSEvent.removeMonitor(monitor) } }
    }
}
#endif
