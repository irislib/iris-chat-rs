#if os(iOS)
import UIKit

/// Keep the suspension allowance until Rust acknowledges that storage is idle.
@MainActor
final class IrisSuspendBackgroundTask {
    private var identifier: UIBackgroundTaskIdentifier = .invalid
    private let endTask: (UIBackgroundTaskIdentifier) -> Void

    init(
        begin: (@escaping @Sendable () -> Void) -> UIBackgroundTaskIdentifier = {
            UIApplication.shared.beginBackgroundTask(withName: "IrisSuspend", expirationHandler: $0)
        },
        end: @escaping (UIBackgroundTaskIdentifier) -> Void = {
            UIApplication.shared.endBackgroundTask($0)
        }
    ) {
        endTask = end
        identifier = begin { [weak self] in
            Task { @MainActor in self?.finish() }
        }
    }

    var isActive: Bool { identifier != .invalid }

    func finish() {
        guard identifier != .invalid else { return }
        let completed = identifier
        identifier = .invalid
        endTask(completed)
    }
}
#endif
