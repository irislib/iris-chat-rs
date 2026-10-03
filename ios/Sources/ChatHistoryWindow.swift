import Foundation

/// Rust snapshots own the recent window. Only explicit page reads may extend it.
struct ChatHistoryWindow {
    private var recentIDs = Set<String>()
    private var olderIDs = Set<String>()
    private var removedIDs = Set<String>()
    private var recentBoundary: UInt64?

    init(recent: [ChatMessageSnapshot] = []) {
        recentIDs = Set(recent.map(\.id))
        recentBoundary = recent.first?.createdAtSecs
    }

    mutating func replaceRecent(
        _ recent: [ChatMessageSnapshot], in displayed: [ChatMessageSnapshot],
        now: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) -> [ChatMessageSnapshot] {
        let nextIDs = Set(recent.map(\.id))
        // Typing/draft updates repeat the same recent window. Keep the browsed
        // array's storage, while still pruning history when its expiry passes.
        if recentIDs == nextIDs, displayed.count >= recent.count,
           displayed.suffix(recent.count).elementsEqual(recent),
           displayed.dropLast(recent.count).allSatisfy({ olderIDs.contains($0.id) && isLive($0, now: now) }),
           recent.allSatisfy({ isLive($0, now: now) }) {
            return displayed
        }
        removedIDs.formUnion(recentIDs.subtracting(nextIDs))
        removedIDs.subtract(nextIDs)
        olderIDs.subtract(recentIDs)
        olderIDs.subtract(nextIDs)
        recentIDs = nextIDs
        if let first = recent.first?.createdAtSecs {
            recentBoundary = min(recentBoundary ?? first, first)
        }
        let older = displayed.filter { olderIDs.contains($0.id) && isLive($0, now: now) }
        olderIDs = Set(older.map(\.id))
        return merge(older, with: recent.filter { isLive($0, now: now) })
    }

    mutating func addPage(
        _ page: [ChatMessageSnapshot], to displayed: [ChatMessageSnapshot],
        now: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) -> [ChatMessageSnapshot] {
        let current = displayed.filter { isLive($0, now: now) }
        let currentIDs = Set(current.map(\.id))
        let additions = page.filter {
            !currentIDs.contains($0.id) && !removedIDs.contains($0.id)
                && isLive($0, now: now)
                && (recentBoundary == nil || $0.createdAtSecs <= recentBoundary!)
        }
        olderIDs.formUnion(additions.map(\.id))
        olderIDs.formIntersection(Set(current.map(\.id)).union(additions.map(\.id)))
        // A read may finish after a reaction/edit/delivery update. Existing rows win.
        return merge(additions, with: current)
    }

    private func isLive(_ message: ChatMessageSnapshot, now: UInt64) -> Bool {
        message.expiresAtSecs.map { $0 > now } ?? true
    }

    private func merge(_ older: [ChatMessageSnapshot], with recent: [ChatMessageSnapshot]) -> [ChatMessageSnapshot] {
        guard !older.isEmpty else { return recent }
        guard !recent.isEmpty else { return older }
        if older.last!.createdAtSecs <= recent.first!.createdAtSecs { return older + recent }
        // Preserve database order for equal-second messages, including the page boundary.
        var result: [ChatMessageSnapshot] = []
        result.reserveCapacity(older.count + recent.count)
        var left = 0, right = 0
        while left < older.count && right < recent.count {
            if older[left].createdAtSecs <= recent[right].createdAtSecs {
                result.append(older[left]); left += 1
            } else {
                result.append(recent[right]); right += 1
            }
        }
        result.append(contentsOf: older[left...])
        result.append(contentsOf: recent[right...])
        return result
    }
}
