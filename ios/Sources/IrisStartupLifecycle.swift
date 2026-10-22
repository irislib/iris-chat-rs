import OSLog

private let irisStartupLogger = Logger(subsystem: "fi.siriusbusiness.irischat", category: "startup")

/// Observes the shell's restoration flow without retaining account or message data.
/// A pending result alone is not evidence of native restoration: state delivery can
/// be delayed or coalesced. An overlap capture also needs a native-restoring state
/// observed at background entry and ordered enqueue/background/result events.
/// These events describe observed state, not the lifetime of a Rust stack frame.
@MainActor
final class IrisStartupLifecycle {
    enum Phase: String, Equatable {
        case restoreEnqueued = "restore_enqueued"
        case restoreDispatchFailed = "restore_dispatch_failed"
        case restoreResultObserved = "restore_result_observed"
        case restoreCancelled = "restore_cancelled"
        case backgrounded
    }

    struct Record: Equatable {
        let phase: Phase
        let restorePendingResult: Bool
        let nativeRestoringObserved: Bool
        let appBackgrounded: Bool
    }

    private var restorePendingResult = false
    private let emit: (Record) -> Void

    init(emit: @escaping (Record) -> Void = {
        irisStartupLogger.notice("iris.startup phase=\($0.phase.rawValue, privacy: .public) restore_pending_result=\($0.restorePendingResult, privacy: .public) native_restoring_observed=\($0.nativeRestoringObserved, privacy: .public) app_backgrounded=\($0.appBackgrounded, privacy: .public)")
    }) {
        self.emit = emit
    }

    func restoreDispatchFinished(dispatched: Bool, backgrounded: Bool, nativeRestoringObserved: Bool) {
        restorePendingResult = dispatched
        record(dispatched ? .restoreEnqueued : .restoreDispatchFailed,
               backgrounded: backgrounded, nativeRestoringObserved: nativeRestoringObserved)
    }

    func restoreResultObserved(backgrounded: Bool, nativeRestoringObserved: Bool) {
        guard restorePendingResult else { return }
        restorePendingResult = false
        record(.restoreResultObserved, backgrounded: backgrounded,
               nativeRestoringObserved: nativeRestoringObserved)
    }

    func backgrounded(nativeRestoringObserved: Bool) {
        record(.backgrounded, backgrounded: true, nativeRestoringObserved: nativeRestoringObserved)
    }

    func restoreCancelled(backgrounded: Bool, nativeRestoringObserved: Bool) {
        guard restorePendingResult else { return }
        restorePendingResult = false
        record(.restoreCancelled, backgrounded: backgrounded,
               nativeRestoringObserved: nativeRestoringObserved)
    }

    private func record(_ phase: Phase, backgrounded: Bool, nativeRestoringObserved: Bool) {
        emit(Record(phase: phase, restorePendingResult: restorePendingResult,
                    nativeRestoringObserved: nativeRestoringObserved, appBackgrounded: backgrounded))
    }
}
