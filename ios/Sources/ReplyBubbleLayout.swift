import SwiftUI

/// Measure the bubble at its natural width, then give each row that width.
/// This lets the quote fill a longer reply without stretching short replies.
struct ReplyBubbleLayout: Layout {
    let isOutgoing: Bool
    private let spacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let idealWidth = subviews.map { $0.sizeThatFits(.unspecified).width }.max() ?? 0
        let width = min(proposal.width ?? idealWidth, idealWidth)
        let rowProposal = ProposedViewSize(width: width, height: nil)
        let height = subviews.reduce(CGFloat.zero) { $0 + $1.sizeThatFits(rowProposal).height }
        return CGSize(width: width, height: height + spacing * CGFloat(max(0, subviews.count - 1)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let rowProposal = ProposedViewSize(width: bounds.width, height: nil)
        var y = bounds.minY
        for row in subviews {
            row.place(
                at: CGPoint(x: isOutgoing ? bounds.maxX : bounds.minX, y: y),
                anchor: isOutgoing ? .topTrailing : .topLeading,
                proposal: rowProposal
            )
            y += row.sizeThatFits(rowProposal).height + spacing
        }
    }
}
