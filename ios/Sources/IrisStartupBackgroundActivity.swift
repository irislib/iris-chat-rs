import Foundation
#if os(iOS)
import UIKit
#endif

/// Protect native construction and the deferred secure-input restore as one startup.
@MainActor
final class IrisStartupBackgroundActivity {
    let initiallyBackgrounded: Bool
    private let backgroundState: () -> Bool
    private var finishAllowance: (() -> Void)?

    init(initiallyBackgrounded: Bool, isBackgrounded: (() -> Bool)? = nil, finish: @escaping () -> Void) {
        self.initiallyBackgrounded = initiallyBackgrounded
        backgroundState = isBackgrounded ?? { initiallyBackgrounded }
        finishAllowance = finish
    }

    static func live() -> IrisStartupBackgroundActivity {
#if os(iOS)
        let allowance = IrisSuspendBackgroundTask()
        return IrisStartupBackgroundActivity(
            initiallyBackgrounded: UIApplication.shared.applicationState == .background,
            isBackgrounded: { UIApplication.shared.applicationState == .background },
            finish: { allowance.finish() }
        )
#else
        return IrisStartupBackgroundActivity(initiallyBackgrounded: false, finish: {})
#endif
    }

    var isBackgrounded: Bool { backgroundState() }

    func finish() {
        let finish = finishAllowance
        finishAllowance = nil
        finish?()
    }
}
