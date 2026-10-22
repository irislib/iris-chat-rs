struct ChatTimelineHistoryPanState {
    private(set) var generation = 0
    private(set) var isActive = false

    mutating func update(isActive: Bool) {
        if isActive && !self.isActive { generation += 1 }
        self.isActive = isActive
    }
}

// Native layout may rebase contentOffset during a window swap. Relate that
// swap to the same drag's captured position instead of counting the rebase
// as finger movement. A later gesture and non-dragging movement use the
// native offset and its existing extent-clamp correction.
struct ChatTimelineHistoryPanPosition<Coordinate: BinaryFloatingPoint> {
    let generation: Int
    let offsetY: Coordinate
    let translationY: Coordinate
    var completedTranslationY: Coordinate? = nil
}

func chatTimelineHistoryOffset<Coordinate: BinaryFloatingPoint>(
    currentOffsetY: Coordinate, contentDeltaY: Coordinate, clampCorrectionY: Coordinate,
    panPosition: ChatTimelineHistoryPanPosition<Coordinate>?,
    panGeneration: Int, panTranslationY: Coordinate?,
    minimumOffsetY: Coordinate, maximumOffsetY: Coordinate
) -> Coordinate {
    if let panPosition, panPosition.generation == panGeneration,
       let panTranslationY, panTranslationY.isFinite {
        let offset = panPosition.offsetY + panPosition.translationY - panTranslationY + contentDeltaY
        if offset.isFinite, offset >= minimumOffsetY, offset <= maximumOffsetY { return offset }
    }
    return currentOffsetY + contentDeltaY + clampCorrectionY
}
