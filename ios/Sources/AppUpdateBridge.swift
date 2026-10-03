import Foundation

final class UpdateBridge: NSObject, AppReconciler, @unchecked Sendable {
    weak var owner: AppManager?
    private let generation: UInt64
    private let pendingLock = NSLock()
    private var pendingUpdates: [AppUpdate] = []
    private var deliveryScheduled = false

    init(owner: AppManager, generation: UInt64) {
        self.owner = owner
        self.generation = generation
    }

    func reconcile(update: AppUpdate) {
        pendingLock.lock()
        // Rust can publish faster than the main actor can draw. Coalesce again
        // at this boundary; never discard or reorder side effects between states.
        if case .fullState(let next) = update,
           case .fullState(let previous)? = pendingUpdates.last {
            if next.rev > previous.rev {
                pendingUpdates[pendingUpdates.count - 1] = update
            }
        } else {
            pendingUpdates.append(update)
        }
        let shouldSchedule = !deliveryScheduled
        deliveryScheduled = true
        pendingLock.unlock()
        guard shouldSchedule else { return }
        Task { @MainActor [weak self] in
            self?.deliverPendingUpdates()
        }
    }

    @MainActor
    private func deliverPendingUpdates() {
        pendingLock.lock()
        let updates = pendingUpdates
        pendingUpdates = []
        deliveryScheduled = false
        pendingLock.unlock()
        for update in updates {
            owner?.apply(update: update, generation: generation)
        }
    }
}
