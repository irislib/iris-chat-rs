import CoreGraphics

// Initial scrolling needs measured rows, then a visible target. A delayed
// follow-up scroll is still useful for attachments, but is not a readiness clock.
struct ChatTimelineInitialPlacement {
    enum Step: Equatable { case wait, scroll, reveal, scrollAndReveal }
    private(set) var isPending = true
    private var awaitingTarget: String?
    var isAwaitingVisibility: Bool { awaitingTarget != nil }

    mutating func reset() { self = Self() }
    mutating func cancel() { isPending = false; awaitingTarget = nil }

    mutating func update(
        targetID: String, frame: CGRect?, measuredEndY: CGFloat? = nil,
        viewportMinY: CGFloat, viewportMaxY: CGFloat
    ) -> Step {
        guard isPending || awaitingTarget != nil else { return .wait }
        // Calls and system notices use the existing timeline-end marker;
        // ordinary messages use their own measured bubble frame.
        let frame = frame ?? measuredEndY.map { CGRect(x: 0, y: $0 - 1, width: 1, height: 1) }
        guard let frame, !frame.isEmpty, !frame.isInfinite, !frame.isNull,
              frame.minY.isFinite, frame.maxY.isFinite,
              viewportMinY.isFinite, viewportMaxY.isFinite,
              viewportMaxY > viewportMinY else { return .wait }

        let visible = frame.maxY > viewportMinY && frame.minY < viewportMaxY
        if isPending || awaitingTarget != targetID {
            isPending = false
            awaitingTarget = visible ? nil : targetID
            return visible ? .scrollAndReveal : .scroll
        }
        guard visible else { return .wait }
        awaitingTarget = nil
        return .reveal
    }
}
