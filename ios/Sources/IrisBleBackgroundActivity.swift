#if os(iOS)
import UIKit

/// An actual Bluetooth callback may earn a short processing window. Outbound
/// writes and polling cannot renew it. Keep the OS allowance through Rust's
/// final storage acknowledgement, including when the system expires it early.
@MainActor
final class IrisBleBackgroundActivity {
    typealias Transition = (UInt64, Bool, @escaping @Sendable () -> Void) -> Void
    private let transition: Transition
    private let isBackground: () -> Bool
    private let beginTask: (@escaping @Sendable () -> Void) -> UIBackgroundTaskIdentifier
    private let endTask: (UIBackgroundTaskIdentifier) -> Void
    private let budget: TimeInterval
    private var generation: UInt64 = 0
    private var current: (generation: UInt64, task: UIBackgroundTaskIdentifier)?
    private var deadline: Task<Void, Never>?
    private var foregroundObserver: NSObjectProtocol?
    private var closed = false

    init(
        budget: TimeInterval = 5,
        isBackground: @escaping () -> Bool = { UIApplication.shared.applicationState == .background },
        begin: @escaping (@escaping @Sendable () -> Void) -> UIBackgroundTaskIdentifier = {
            UIApplication.shared.beginBackgroundTask(withName: "IrisBluetooth", expirationHandler: $0)
        },
        end: @escaping (UIBackgroundTaskIdentifier) -> Void = { UIApplication.shared.endBackgroundTask($0) },
        transition: @escaping Transition
    ) {
        self.budget = budget
        self.isBackground = isBackground
        self.beginTask = begin
        self.endTask = end
        self.transition = transition
        foregroundObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.finish() }
        }
    }

    func receive(wakesApp: Bool, deliver: () -> Void) {
        guard !closed else { return }
        if wakesApp, isBackground(), current == nil {
            generation &+= 1
            let id = generation
            let task = beginTask { [weak self] in
                Task { @MainActor in self?.finish(generation: id) }
            }
            if task != .invalid {
                current = (id, task)
                // The bridge serializes this transition before event delivery.
                transition(id, true, {})
                deadline = Task { [weak self, budget] in
                    do { try await Task.sleep(nanoseconds: UInt64(budget * 1_000_000_000)) }
                    catch { return }
                    self?.finish(generation: id)
                }
            }
        }
        deliver()
    }

    func finish(generation: UInt64? = nil) {
        guard let current, generation == nil || current.generation == generation else { return }
        self.current = nil
        deadline?.cancel()
        deadline = nil
        let endTask = endTask
        transition(current.generation, false) {
            Task { @MainActor in endTask(current.task) }
        }
    }

    func close() {
        closed = true
        finish()
        if let foregroundObserver {
            NotificationCenter.default.removeObserver(foregroundObserver)
            self.foregroundObserver = nil
        }
    }

    deinit {
        deadline?.cancel()
        if let foregroundObserver { NotificationCenter.default.removeObserver(foregroundObserver) }
    }
}
#endif
