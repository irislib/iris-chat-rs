import SwiftUI

// Keep bubble positioning stable while the hover controls are absent.
struct ChatMessageActionDockSlot<Content: View>: View {
    let isVisible: Bool
    let size: CGSize
    @ViewBuilder var content: () -> Content

    var body: some View {
        Color.clear
            .frame(width: size.width, height: size.height)
            .overlay {
                if isVisible {
                    content().fixedSize()
                }
            }
            .allowsHitTesting(isVisible)
            .accessibilityHidden(!isVisible)
    }
}
