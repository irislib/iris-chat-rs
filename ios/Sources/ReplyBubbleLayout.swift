import SwiftUI

struct ReplyBubbleTrailingRow: LayoutValueKey {
    static let defaultValue = false
}

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
            // Only the outgoing timestamp belongs on the trailing edge.
            // A wider quote must not push the reply text away from its left inset.
            let trailing = isOutgoing && row[ReplyBubbleTrailingRow.self]
            row.place(
                at: CGPoint(x: trailing ? bounds.maxX : bounds.minX, y: y),
                anchor: trailing ? .topTrailing : .topLeading,
                proposal: rowProposal
            )
            y += row.sizeThatFits(rowProposal).height + spacing
        }
    }
}
