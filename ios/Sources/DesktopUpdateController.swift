#if os(macOS)
import AppKit
import Foundation
import SwiftUI

/// Shows what the Rust updater is doing. The core owns every step, message
/// and time limit; this class keeps the user's two preferences, schedules
/// automatic checks and quits when the core says the update needs it.
@MainActor
final class DesktopUpdateController: ObservableObject {
    @Published private(set) var view: DesktopUpdateView
    @Published var autoCheck: Bool {
        didSet {
            defaults.set(autoCheck, forKey: "updates.autoCheck")
            if autoCheck {
                startAutomaticChecks()
            } else {
                stopAutomaticChecks()
            }
        }
    }
    @Published var autoInstall: Bool {
        didSet {
            defaults.set(autoInstall, forKey: "updates.autoInstall")
            if autoInstall, view.step == .available {
                updater.install()
            }
        }
    }

    private let updater: FfiDesktopUpdater
    private let defaults: UserDefaults
    private let quit: () -> Void
    private var automaticCheckTask: Task<Void, Never>?
    private var startupCheckDone = false
    private var quitRequested = false
    private(set) var diagnostics: [ClientDebugLogEntry] = []

    init(
        defaults: UserDefaults = .standard,
        stateDir: URL = FileManager.default.temporaryDirectory.appendingPathComponent("IrisChatUpdates"),
        appPath: String? = Bundle.main.bundleURL.path,
        quit: @escaping () -> Void = { NSApp.terminate(nil) }
    ) {
        self.defaults = defaults
        self.quit = quit
        self.autoCheck = defaults.object(forKey: "updates.autoCheck") as? Bool ?? true
        self.autoInstall = defaults.bool(forKey: "updates.autoInstall")
        let relay = DesktopUpdateRelay()
        self.updater = FfiDesktopUpdater(observer: relay, stateDir: stateDir.path, appPath: appPath)
        self.view = updater.view()
        relay.controller = self
        record(view)
    }

    deinit {
        automaticCheckTask?.cancel()
    }

    func runStartupCheckIfNeeded() {
        startAutomaticChecks()
    }

    func startAutomaticChecks() {
        guard autoCheck else {
            stopAutomaticChecks()
            return
        }
        if !startupCheckDone {
            startupCheckDone = true
            check(manual: false)
        }
        guard automaticCheckTask == nil else { return }
        automaticCheckTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.automaticCheckIntervalNanoseconds)
                guard let self, !Task.isCancelled else { return }
                guard self.autoCheck else {
                    self.stopAutomaticChecks()
                    return
                }
                self.check(manual: false)
            }
        }
    }

    func stopAutomaticChecks() {
        automaticCheckTask?.cancel()
        automaticCheckTask = nil
    }

    func check(manual: Bool = true) {
        updater.check(manual: manual, installWhenFound: autoInstall)
    }

    /// The button on the strip and in Settings: Install, or Retry.
    func performAction() {
        updater.performAction()
    }

    /// Show a view from the core. Tests also call this to preview steps.
    func apply(_ next: DesktopUpdateView) {
        view = next
        record(next)
        if next.quitToFinish, !quitRequested {
            quitRequested = true
            quit()
        }
    }

    private func record(_ view: DesktopUpdateView) {
        guard view.step == .failed, let detail = view.diagnostic,
              diagnostics.last?.detail != detail else { return }
        diagnostics.append(ClientDebugLogEntry(
            timestampSecs: UInt64(Date().timeIntervalSince1970),
            category: "updates.failed",
            detail: detail
        ))
        if diagnostics.count > 10 { diagnostics.removeFirst(diagnostics.count - 10) }
        NSLog("Iris Chat update failed: %@", detail)
    }

    private static var automaticCheckIntervalNanoseconds: UInt64 {
        if let raw = ProcessInfo.processInfo.environment["IRIS_UPDATE_POLL_SECONDS"],
           let seconds = Double(raw),
           seconds > 0 {
            return UInt64(seconds * 1_000_000_000)
        }
        return 6 * 60 * 60 * 1_000_000_000
    }
}

/// Hops core callbacks from the updater's thread to the main actor.
private final class DesktopUpdateRelay: DesktopUpdateObserver, @unchecked Sendable {
    weak var controller: DesktopUpdateController?

    func desktopUpdateChanged(view: DesktopUpdateView) {
        // The main queue keeps the steps in the order the core sent them.
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                self?.controller?.apply(view)
            }
        }
    }
}

extension DesktopUpdateView {
    var lastChecked: Date? {
        lastCheckedSecs.map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }
}
#endif
