import Foundation

// Reuse GATT metadata for this scan; FIPS still owns connection retries.
struct AppleBleBootstrapDiscovery {
    enum Action: Equatable {
        case read
        case cached(Data)
    }

    private var pending: Set<UUID> = []
    private var bootstraps: [UUID: Data] = [:]

    func isPending(_ identifier: UUID) -> Bool {
        pending.contains(identifier)
    }

    mutating func begin(_ identifier: UUID, refresh: Bool = false) -> Action? {
        if refresh {
            bootstraps.removeValue(forKey: identifier)
        }
        guard !pending.contains(identifier) else { return nil }
        if let bootstrap = bootstraps[identifier] {
            return .cached(bootstrap)
        }
        guard pending.count < 64 else { return nil }
        pending.insert(identifier)
        return .read
    }

    @discardableResult
    mutating func complete(_ identifier: UUID, bootstrap: Data) -> Bool {
        guard pending.remove(identifier) != nil else { return false }
        bootstraps[identifier] = bootstrap
        return true
    }

    mutating func cancel(_ identifier: UUID) {
        pending.remove(identifier)
    }

    mutating func reset() {
        pending.removeAll()
        bootstraps.removeAll()
    }
}
