import SwiftUI

// Reserve horizontal room without making short bubbles as tall as the toolbar.
struct ChatMessageActionDockSlot<Content: View>: View {
    let isVisible: Bool
    let size: CGSize
    @ViewBuilder var content: () -> Content

    var body: some View {
        Color.clear
            .frame(width: size.width, height: 0)
            .overlay(alignment: .bottom) {
                if isVisible {
                    content().fixedSize()
                }
            }
            .allowsHitTesting(isVisible)
            .accessibilityHidden(!isVisible)
    }
}
