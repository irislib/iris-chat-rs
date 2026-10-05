import Foundation

/// Only a bounded history window is laid out. Previously visited rows
/// contribute their measured height to the surrounding spacers.
struct ChatTimelineRenderWindow: Equatable {
    static let preferredCount = 80
    static let capacity = 160
    static let step = 40
    private static let overscan = 16
    private var anchorID: String?
    private var rowsBeforeAnchor = 0
    private var fallbackStart = 0
    private var rowCount = Self.preferredCount
    private var protectedIDs: Set<String> = []

    func range(in ids: [String]) -> Range<Int> {
        let protected = protectedRange(in: ids)
        let count = budget(protecting: protected)
        let maximum = max(0, ids.count - count)
        let desired = anchorID.map { id in
            ids.firstIndex(of: id).map { $0 - rowsBeforeAnchor } ?? fallbackStart
        } ?? maximum
        var start = min(maximum, max(0, desired))
        if let protected, protected.count <= count {
            let margin = min(Self.overscan, (count - protected.count) / 2)
            let minimumStart = min(maximum, max(0, protected.upperBound + margin - count))
            let maximumStart = min(maximum, max(0, protected.lowerBound - margin))
            start = min(maximumStart, max(minimumStart, start))
        }
        return start..<min(ids.count, start + count)
    }

    mutating func showLatest() { self = Self() }

    mutating func show(_ id: String, in ids: [String]) {
        guard let index = ids.firstIndex(of: id) else { return }
        self = Self()
        anchorID = id
        rowsBeforeAnchor = Self.step
        fallbackStart = max(0, index - Self.step)
    }

    mutating func start(at index: Int, in ids: [String]) {
        guard !ids.isEmpty else { self = Self(); return }
        rowCount = budget(protecting: protectedRange(in: ids))
        protectedIDs.removeAll()
        let start = min(max(0, index), max(0, ids.count - rowCount))
        anchorID = ids[start]
        rowsBeforeAnchor = 0
        fallbackStart = start
    }

    /// Keep the measured viewport and any captured anchor inside one exact
    /// window. A prepend changes their indices, while these identities survive.
    @discardableResult
    mutating func preserveVisible(_ visible: Range<Int>, including capturedID: String?,
                                 in ids: [String], startAt: Int? = nil) -> Bool {
        guard !visible.isEmpty, visible.lowerBound >= 0, visible.upperBound <= ids.count else { return false }
        let captured = capturedID.flatMap { ids.firstIndex(of: $0) }
        let lower = min(visible.lowerBound, captured ?? visible.lowerBound)
        let upper = max(visible.upperBound, captured.map { $0 + 1 } ?? visible.upperBound)
        guard upper - lower <= Self.capacity else { return false }
        rowCount = min(Self.capacity, max(Self.preferredCount, upper - lower + Self.overscan * 2))
        anchorID = ids[lower]
        rowsBeforeAnchor = startAt.map { lower - $0 } ?? (rowCount - (upper - lower)) / 2
        fallbackStart = max(0, lower - rowsBeforeAnchor)
        protectedIDs = Set(ids[lower..<upper])
        return true
    }

    private func protectedRange(in ids: [String]) -> Range<Int>? {
        guard !protectedIDs.isEmpty,
              let first = ids.firstIndex(where: { protectedIDs.contains($0) }),
              let last = ids.lastIndex(where: { protectedIDs.contains($0) }) else { return nil }
        return first..<(last + 1)
    }

    private func budget(protecting range: Range<Int>?) -> Int {
        guard let range else { return rowCount }
        return min(Self.capacity, max(Self.preferredCount, range.count + Self.overscan * 2))
    }

    static func spacerHeight(_ ids: ArraySlice<String>, measured: [String: CGFloat]) -> CGFloat {
        // Unvisited live arrivals have no layout yet. This estimate only
        // represents off-screen content; its real row is always measured.
        ids.reduce(0) { $0 + (measured[$1] ?? 100) }
    }
}
