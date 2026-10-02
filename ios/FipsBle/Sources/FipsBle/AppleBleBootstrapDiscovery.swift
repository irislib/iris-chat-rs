import Foundation

// Discovery can fail before FIPS receives a peer candidate and can apply its
// connection backoff. Bound those GATT retries separately and report each
// successful discovery once. Re-emitting unchanged ads makes FIPS reopen links
// it deliberately closed after selecting another transport.
struct AppleBleBootstrapDiscovery {
    private var pending: [UUID: TimeInterval] = [:]
    private var resolved: Set<UUID> = []
    private struct Failure {
        let delay: TimeInterval
        let retryAt: TimeInterval
    }
    private var failures: [UUID: Failure] = [:]

    func nextWakeupAt(allowing canRead: (UUID) -> Bool = { _ in true }) -> TimeInterval? {
        [pending.values.min(), nextRetryAt(allowing: canRead)].compactMap { $0 }.min()
    }

    mutating func expirePending(now: TimeInterval) -> [UUID] {
        let expired = pending.filter { $0.value <= now }.map(\.key)
        for identifier in expired { fail(identifier, now: now) }
        return expired
    }

    func nextRetryAt(allowing canRead: (UUID) -> Bool = { _ in true }) -> TimeInterval? {
        guard pending.count < 64 else { return nil }
        return failures.filter { pending[$0.key] == nil && canRead($0.key) }.values.map(\.retryAt).min()
    }

    func dueRetries(now: TimeInterval, allowing canRead: (UUID) -> Bool = { _ in true }) -> [UUID] {
        guard pending.count < 64 else { return [] }
        return failures.filter {
            pending[$0.key] == nil && $0.value.retryAt <= now && canRead($0.key)
        }.map(\.key)
    }

    func isPending(_ identifier: UUID) -> Bool {
        pending[identifier] != nil
    }

    mutating func begin(
        _ identifier: UUID,
        refresh: Bool = false,
        canRead: Bool = true,
        now: TimeInterval = ProcessInfo.processInfo.systemUptime
    ) -> Bool {
        if refresh {
            resolved.remove(identifier)
        }
        guard pending[identifier] == nil else { return false }
        guard !resolved.contains(identifier) else { return false }
        guard canRead else {
            if failures[identifier] == nil {
                failures[identifier] = Failure(delay: 0, retryAt: 0)
            }
            return false
        }
        if let failure = failures[identifier], now < failure.retryAt { return false }
        guard pending.count < 64 else { return false }
        // CoreBluetooth does not guarantee a connect/service/read callback when
        // the peripheral disappears. Keep a deadline even when all slots fill.
        pending[identifier] = now + 15
        return true
    }

    @discardableResult
    mutating func complete(_ identifier: UUID) -> Bool {
        guard pending.removeValue(forKey: identifier) != nil else { return false }
        failures.removeValue(forKey: identifier)
        resolved.insert(identifier)
        return true
    }

    mutating func fail(_ identifier: UUID, now: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        guard pending.removeValue(forKey: identifier) != nil else { return }
        let delay = min(max(5, (failures[identifier]?.delay ?? 0) * 2), 60)
        failures[identifier] = Failure(delay: delay, retryAt: now + delay)
    }

    mutating func reset() {
        pending.removeAll()
        resolved.removeAll()
        failures.removeAll()
    }
}
