import SwiftUI

// A maximum-width frame expands even a short bubble to the maximum, separating
// its visible edge from the adjacent action dock. Limit the proposal instead.
struct ChatMessageBubbleWidthLimit: ViewModifier {
    let maxWidth: CGFloat

    func body(content: Content) -> some View {
        ChatMessageBubbleWidthLayout(maxWidth: maxWidth) { content }
    }
}

private struct ChatMessageBubbleWidthLayout: Layout {
    let maxWidth: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let content = subviews.first else { return .zero }
        return content.sizeThatFits(limited(proposal))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, proposal: limited(proposal))
    }

    private func limited(_ proposal: ProposedViewSize) -> ProposedViewSize {
        ProposedViewSize(width: min(proposal.width ?? maxWidth, maxWidth), height: proposal.height)
    }
}

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
