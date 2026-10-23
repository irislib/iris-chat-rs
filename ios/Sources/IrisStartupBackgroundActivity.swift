import Foundation
#if os(iOS)
import UIKit
#endif

/// Protect native construction and the deferred secure-input restore as one startup.
@MainActor
final class IrisStartupBackgroundActivity {
    let initiallyBackgrounded: Bool
    private let backgroundState: () -> Bool
    private let isAllowanceActive: (() -> Bool)?
    private var finishAllowance: (() -> Void)?
    private let startupCancellation: IrisStartupRestoreCancellation

    init(initiallyBackgrounded: Bool, isBackgrounded: (() -> Bool)? = nil,
         startupCancellation: IrisStartupRestoreCancellation = IrisStartupRestoreCancellation(),
         isAllowanceActive: (() -> Bool)? = nil,
         finish: @escaping () -> Void) {
        self.initiallyBackgrounded = initiallyBackgrounded
        backgroundState = isBackgrounded ?? { initiallyBackgrounded }
        finishAllowance = finish
        self.startupCancellation = startupCancellation
        self.isAllowanceActive = isAllowanceActive
    }

    static func live() -> IrisStartupBackgroundActivity {
#if os(iOS)
        let cancellation = IrisStartupRestoreCancellation()
        let allowance = IrisSuspendBackgroundTask(startupCancellation: cancellation)
        return IrisStartupBackgroundActivity(
            initiallyBackgrounded: UIApplication.shared.applicationState == .background,
            isBackgrounded: { UIApplication.shared.applicationState == .background },
            startupCancellation: cancellation,
            isAllowanceActive: { allowance.isActive },
            finish: { allowance.finish() }
        )
#else
        return IrisStartupBackgroundActivity(initiallyBackgrounded: false, finish: {})
#endif
    }

    var isBackgrounded: Bool { backgroundState() }

    func bindStartupRestore(to client: RustAppClient) {
        startupCancellation.bind(to: client)
        cancelIfBackgroundAllowanceUnavailable(backgrounded: isBackgrounded)
    }

    func cancelIfBackgroundAllowanceUnavailable(backgrounded: Bool) {
        if backgrounded && isAllowanceActive?() == false { startupCancellation.expire() }
    }

    func finish() {
        startupCancellation.finish()
        let finish = finishAllowance
        finishAllowance = nil
        finish?()
    }
}
