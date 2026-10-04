import Foundation

/// Only a bounded history window is laid out. Previously visited rows
/// contribute their measured height to the surrounding spacers.
struct ChatTimelineRenderWindow: Equatable {
    static let capacity = 160
    static let step = 80
    private var anchorID: String?
    private var rowsBeforeAnchor = 0
    private var fallbackStart = 0

    func range(in ids: [String]) -> Range<Int> {
        let maximum = max(0, ids.count - Self.capacity)
        let desired = anchorID.map { id in
            ids.firstIndex(of: id).map { $0 - rowsBeforeAnchor } ?? fallbackStart
        } ?? maximum
        let start = min(maximum, max(0, desired))
        return start..<min(ids.count, start + Self.capacity)
    }

    mutating func showLatest() { self = Self() }

    mutating func show(_ id: String, in ids: [String]) {
        guard let index = ids.firstIndex(of: id) else { return }
        anchorID = id
        rowsBeforeAnchor = Self.step
        fallbackStart = max(0, index - Self.step)
    }

    mutating func start(at index: Int, in ids: [String]) {
        guard !ids.isEmpty else { self = Self(); return }
        let start = min(max(0, index), max(0, ids.count - Self.capacity))
        anchorID = ids[start]
        rowsBeforeAnchor = 0
        fallbackStart = start
    }

    static func spacerHeight(_ ids: ArraySlice<String>, measured: [String: CGFloat]) -> CGFloat {
        // Unvisited live arrivals have no layout yet. This estimate only
        // represents off-screen content; its real row is always measured.
        ids.reduce(0) { $0 + (measured[$1] ?? 100) }
    }
}
