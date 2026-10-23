#if os(iOS)
import UIKit

/// Normal completion follows the storage acknowledgement. Expiration requests
/// cold-restore cancellation directly before releasing the finite OS allowance.
@MainActor
final class IrisSuspendBackgroundTask {
    private var identifier: UIBackgroundTaskIdentifier = .invalid
    private let endTask: (UIBackgroundTaskIdentifier) -> Void
    private let startupCancellation: IrisStartupRestoreCancellation

    init(
        startupCancellation: IrisStartupRestoreCancellation = IrisStartupRestoreCancellation(),
        begin: (@escaping @Sendable () -> Void) -> UIBackgroundTaskIdentifier = {
            UIApplication.shared.beginBackgroundTask(withName: "IrisSuspend", expirationHandler: $0)
        },
        end: @escaping (UIBackgroundTaskIdentifier) -> Void = {
            UIApplication.shared.endBackgroundTask($0)
        }
    ) {
        endTask = end
        self.startupCancellation = startupCancellation
        identifier = begin { [weak self] in
            startupCancellation.expire()
            Task { @MainActor in self?.finish() }
        }
    }

    var isActive: Bool { identifier != .invalid }

    func finish() {
        startupCancellation.finish()
        guard identifier != .invalid else { return }
        let completed = identifier
        identifier = .invalid
        endTask(completed)
    }
}
#endif
