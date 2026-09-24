#if os(macOS)
import SwiftUI

struct IrisDesktopZoom: ViewModifier {
    let level: Int

    func body(content: Content) -> some View {
        let scale = pow(1.2, Double(min(4, max(-3, level))))
        GeometryReader { geometry in
            content
                .frame(width: geometry.size.width / scale, height: geometry.size.height / scale)
                .scaleEffect(scale, anchor: .topLeading)
                .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
        }
    }
}

struct IrisDesktopZoomCommands: Commands {
    @Binding var level: Int

    var body: some Commands {
        CommandGroup(after: .toolbar) {
            Divider()
            Button("Actual Size") { level = 0 }
                .keyboardShortcut("0", modifiers: .command)
            Button("Zoom In") { level = min(4, level + 1) }
                .keyboardShortcut("=", modifiers: .command)
                .disabled(level >= 4)
            Button("Zoom Out") { level = max(-3, level - 1) }
                .keyboardShortcut("-", modifiers: .command)
                .disabled(level <= -3)
        }
    }
}
#endif
