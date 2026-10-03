import CoreGraphics

// Realize the last lazy row, then reveal only when its measured frame is visible.
// Geometry changes can correct estimated heights without a readiness timer.
struct ChatTimelineInitialPlacement {
    enum Step: Equatable { case wait, scroll, reveal, scrollAndReveal }
    private(set) var isPending = true
    private var awaitingTarget: String?
    private var lastScrollFrame: CGRect?
    var isAwaitingVisibility: Bool { awaitingTarget != nil }

    mutating func reset() { self = Self() }
    mutating func cancel() { isPending = false; awaitingTarget = nil }

    mutating func update(
        targetID: String, frame: CGRect?,
        viewportMinY: CGFloat, viewportMaxY: CGFloat
    ) -> Step {
        guard isPending || awaitingTarget != nil else { return .wait }
        guard viewportMinY.isFinite, viewportMaxY.isFinite,
              viewportMaxY > viewportMinY else { return .wait }
        if let frame {
            guard !frame.isEmpty, !frame.isInfinite, !frame.isNull,
                  frame.minY.isFinite, frame.maxY.isFinite else { return .wait }
        }

        let visible = frame.map { $0.maxY > viewportMinY && $0.minY < viewportMaxY } ?? false
        if isPending || awaitingTarget != targetID {
            isPending = false
            awaitingTarget = visible ? nil : targetID
            lastScrollFrame = frame
            return visible ? .scrollAndReveal : .scroll
        }
        if visible {
            awaitingTarget = nil
            return .reveal
        }
        guard let frame, frame != lastScrollFrame else { return .wait }
        lastScrollFrame = frame
        return .scroll
    }
}
