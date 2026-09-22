import Foundation

// Discovery can fail before FIPS receives a peer candidate and can apply its
// connection backoff. Bound those GATT retries separately and cache successes.
struct AppleBleBootstrapDiscovery {
    enum Action: Equatable {
        case read
        case cached(Data)
    }

    private var pending: Set<UUID> = []
    private var bootstraps: [UUID: Data] = [:]
    private struct Failure {
        let delay: TimeInterval
        let retryAt: TimeInterval
    }
    private var failures: [UUID: Failure] = [:]

    func nextRetryAt(allowing canRead: (UUID) -> Bool = { _ in true }) -> TimeInterval? {
        guard pending.count < 64 else { return nil }
        return failures.filter { !pending.contains($0.key) && canRead($0.key) }.values.map(\.retryAt).min()
    }

    func dueRetries(now: TimeInterval, allowing canRead: (UUID) -> Bool = { _ in true }) -> [UUID] {
        guard pending.count < 64 else { return [] }
        return failures.filter {
            !pending.contains($0.key) && $0.value.retryAt <= now && canRead($0.key)
        }.map(\.key)
    }

    func isPending(_ identifier: UUID) -> Bool {
        pending.contains(identifier)
    }

    mutating func begin(
        _ identifier: UUID,
        refresh: Bool = false,
        canRead: Bool = true,
        now: TimeInterval = ProcessInfo.processInfo.systemUptime
    ) -> Action? {
        if refresh {
            bootstraps.removeValue(forKey: identifier)
        }
        guard !pending.contains(identifier) else { return nil }
        if let bootstrap = bootstraps[identifier] {
            return .cached(bootstrap)
        }
        guard canRead else {
            if failures[identifier] == nil {
                failures[identifier] = Failure(delay: 0, retryAt: 0)
            }
            return nil
        }
        if let failure = failures[identifier], now < failure.retryAt { return nil }
        guard pending.count < 64 else { return nil }
        pending.insert(identifier)
        return .read
    }

    @discardableResult
    mutating func complete(_ identifier: UUID, bootstrap: Data) -> Bool {
        guard pending.remove(identifier) != nil else { return false }
        failures.removeValue(forKey: identifier)
        bootstraps[identifier] = bootstrap
        return true
    }

    mutating func fail(_ identifier: UUID, now: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        guard pending.remove(identifier) != nil else { return }
        let delay = min(max(5, (failures[identifier]?.delay ?? 0) * 2), 60)
        failures[identifier] = Failure(delay: delay, retryAt: now + delay)
    }

    mutating func reset() {
        pending.removeAll()
        bootstraps.removeAll()
        failures.removeAll()
    }
}
