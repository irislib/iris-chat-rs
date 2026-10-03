import SwiftUI

// Reserve horizontal room without making short bubbles as tall as the toolbar.
struct ChatMessageActionDockSlot<Content: View>: View {
    let isVisible: Bool
    let width: CGFloat
    @ViewBuilder var content: () -> Content

    var body: some View {
        Color.clear
            .frame(width: width, height: 0)
            .overlay(alignment: .bottom) {
                if isVisible {
                    content().fixedSize()
                }
            }
            .allowsHitTesting(isVisible)
            .accessibilityHidden(!isVisible)
    }
}
