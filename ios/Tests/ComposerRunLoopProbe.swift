import CoreFoundation
import Foundation

/// Main-run-loop wall time during a hosted edit, not CPU time or frame rate.
/// All callbacks run on the main run loop; no work is moved off that loop.
final class ComposerRunLoopProbe {
    private let started = ProcessInfo.processInfo.systemUptime
    private var activeSince: TimeInterval?
    private var activeMs = 0.0
    private var longestActiveMs = 0.0
    private var lastTimer: TimeInterval = 0
    private var longestTimerGapMs = 0.0
    private var timerSamples = 0
    private var phase = "native-edit-entry"
    private var markers: [[String: Any]] = []
    private var longestTurns: [[String: Any]] = []
    private var observer: CFRunLoopObserver?
    private var timer: Timer?

    init() {
        lastTimer = started
        activeSince = started
        observer = CFRunLoopObserverCreateWithHandler(kCFAllocatorDefault, CFRunLoopActivity.allActivities.rawValue, true, 0) { [weak self] _, activity in
            guard let self else { return }
            let now = ProcessInfo.processInfo.systemUptime
            if activity == .beforeWaiting || activity == .exit {
                self.endActiveTurn(at: now)
            } else if self.activeSince == nil {
                self.activeSince = now
            }
        }
        if let observer { CFRunLoopAddObserver(CFRunLoopGetMain(), observer, kCFRunLoopCommonModes) }
        let timer = Timer(timeInterval: 0.01, repeats: true) { [weak self] _ in
            guard let self else { return }
            let now = ProcessInfo.processInfo.systemUptime
            self.longestTimerGapMs = max(self.longestTimerGapMs, (now - self.lastTimer) * 1_000)
            self.lastTimer = now
            self.timerSamples += 1
        }
        self.timer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    func mark(_ name: String) {
        phase = name
        markers.append(["phase": name, "elapsedMs": (ProcessInfo.processInfo.systemUptime - started) * 1_000])
    }

    func finish() -> [String: Any] {
        endActiveTurn(at: ProcessInfo.processInfo.systemUptime)
        invalidate()
        return ["activeWallMs": activeMs, "longestActiveTurnMs": longestActiveMs,
                "longestTimerCallbackGapMs": longestTimerGapMs, "timerSamples": timerSamples,
                "timerIntervalMs": 10, "phaseMarkers": markers, "longestActiveTurns": longestTurns]
    }

    func invalidate() {
        timer?.invalidate()
        timer = nil
        if let observer { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, kCFRunLoopCommonModes) }
        observer = nil
    }

    private func endActiveTurn(at now: TimeInterval) {
        guard let beginning = activeSince else { return }
        activeSince = nil
        let duration = (now - beginning) * 1_000
        activeMs += duration
        longestActiveMs = max(longestActiveMs, duration)
        if duration >= 20 {
            longestTurns.append(["startMs": (beginning - started) * 1_000, "durationMs": duration, "endingPhase": phase])
            longestTurns.sort { ($0["durationMs"] as? Double ?? 0) > ($1["durationMs"] as? Double ?? 0) }
            if longestTurns.count > 8 { longestTurns.removeLast() }
        }
    }

    deinit { invalidate() }
}
