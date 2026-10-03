import Foundation

/// Rust retains every loaded row. Project a bounded entry window, then grow it
/// only with live arrivals or explicit pages while preserving raw authority.
struct ChatHistoryWindow {
    static let pageSize = 80
    private var recentIDs = Set<String>()
    private var olderIDs = Set<String>()
    private var removedIDs = Set<String>()
    private var rawIDs = Set<String>()
    private var excludedIDs = Set<String>()
    private var receivedRawSnapshot = false
    private var recentBoundary: UInt64?

    init(recent: [ChatMessageSnapshot] = []) {
        let recent = recent.suffix(Self.pageSize)
        recentIDs = Set(recent.map(\.id))
        recentBoundary = recent.first?.createdAtSecs
    }

    mutating func replaceRecent(
        _ raw: [ChatMessageSnapshot], in displayed: [ChatMessageSnapshot],
        now: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) -> [ChatMessageSnapshot] {
        let nextRawIDs = Set(raw.map(\.id))
        if let boundary = raw.firstIndex(where: { recentIDs.contains($0.id) }) {
            excludedIDs.formUnion(raw[..<boundary].map(\.id))
        } else if !receivedRawSnapshot {
            excludedIDs.formUnion(raw.dropLast(Self.pageSize).map(\.id))
        }
        receivedRawSnapshot = receivedRawSnapshot || !raw.isEmpty
        // Only omissions from the full raw set are deletions. Trimming the
        // hidden prefix must never prevent a later explicit page read.
        removedIDs.formUnion(rawIDs.union(recentIDs).subtracting(nextRawIDs))
        removedIDs.subtract(nextRawIDs)
        rawIDs = nextRawIDs
        if displayed.count == recentIDs.count + olderIDs.count, displayed == raw,
           displayed.allSatisfy({ isLive($0, now: now) }) {
            return displayed
        }
        let recent = raw.filter { !excludedIDs.contains($0.id) && isLive($0, now: now) }
        let fresh = olderIDs.isEmpty ? [:] : Dictionary(raw.map { ($0.id, $0) }, uniquingKeysWith: { _, next in next })
        return update(recent, in: displayed, fresh: fresh, now: now)
    }

    /// A latest-page database read is partial, so it cannot delete raw history.
    mutating func replaceLatestPage(
        _ page: [ChatMessageSnapshot], in displayed: [ChatMessageSnapshot],
        now: UInt64 = UInt64(Date().timeIntervalSince1970)
    ) -> [ChatMessageSnapshot] {
        var recent = page.suffix(Self.pageSize).filter { !removedIDs.contains($0.id) && isLive($0, now: now) }
        if receivedRawSnapshot {
            let current = displayed.filter { recentIDs.contains($0.id) && isLive($0, now: now) }
            let currentIDs = Set(current.map(\.id))
            let additions = recent.filter {
                !currentIDs.contains($0.id) && (current.count < Self.pageSize
                    || (!excludedIDs.contains($0.id)
                        && (recentBoundary == nil || $0.createdAtSecs >= recentBoundary!)))
            }
            recent = merge(additions, with: current)
            if current.count < Self.pageSize { recent = Array(recent.suffix(Self.pageSize)) }
        }
        excludedIDs.subtract(recent.map(\.id))
        return update(recent, in: displayed, fresh: [:], now: now)
    }

    private mutating func update(
        _ recent: [ChatMessageSnapshot], in displayed: [ChatMessageSnapshot],
        fresh: [String: ChatMessageSnapshot], now: UInt64
    ) -> [ChatMessageSnapshot] {
        recentIDs = Set(recent.map(\.id))
        olderIDs.subtract(recentIDs)
        let older = displayed.compactMap { message -> ChatMessageSnapshot? in
            guard olderIDs.contains(message.id), !removedIDs.contains(message.id) else { return nil }
            let current = fresh[message.id] ?? message
            return isLive(current, now: now) ? current : nil
        }
        olderIDs = Set(older.map(\.id))
        if let first = recent.first?.createdAtSecs {
            recentBoundary = min(recentBoundary ?? first, first)
        }
        let result = merge(older, with: recent)
        // Preserve the displayed array's storage on repeated typing/draft updates.
        return result == displayed ? displayed : result
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
