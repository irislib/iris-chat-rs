import Foundation
import CoreGraphics
import OSLog

private let irisInteractionLogger = Logger(subsystem: "fi.siriusbusiness.irischat", category: "interaction")

#if os(iOS)
/// Temporary, main-thread diagnostics for anonymous pagination UI fixtures.
/// Geometry stays outside Equatable preferences, so tracing cannot cause delivery.
final class IrisTimelineAnchorTrace {
    enum Event: String {
        case ready, capture, restore, awaitExtent = "await_extent", apply
        case extentWill = "extent_will", extentDid = "extent_did"
        case postApply = "post_apply", nativeOffset = "native_offset", panEnded = "pan_ended"
    }
    enum Origin: String { case direct, preference, nativeLayout = "native_layout", mainTurn = "main_turn" }
    struct Sample {
        var offsetY: CGFloat = .nan
        var nativeContentHeight: CGFloat = .nan
        var viewportHeight: CGFloat = .nan
        var insetTop: CGFloat = .nan
        var insetBottom: CGFloat = .nan
        var panY: CGFloat = .nan
        var velocityY: CGFloat = .nan
        var panState = -1
        var dragging = false
        var decelerating = false
        var anchorViewportY: CGFloat = .nan
        var originalContentY: CGFloat = .nan
        var contentY: CGFloat = .nan
        var preferenceContentHeight: CGFloat = .nan
        var clampCorrectionY: CGFloat = 0
        var offsetBeforeExtent: CGFloat = .nan
        var candidateOffsetY: CGFloat = .nan
        var extentCommitted = false
        var firstChanged = false
        var generationChanged = false
    }
    struct Record {
        let event: Event
        let origin: Origin
        let elapsedMilliseconds: Double
        let preferenceAgeMilliseconds: Double
        let preferenceOffsetY: CGFloat
        let sample: Sample

        var line: String {
            let s = sample
            return "iris.timeline_anchor event=\(event.rawValue) origin=\(origin.rawValue) elapsed_ms=\(elapsedMilliseconds)"
                + " preference_age_ms=\(preferenceAgeMilliseconds) preference_offset_y=\(preferenceOffsetY) offset_y=\(s.offsetY)"
                + " native_content_height=\(s.nativeContentHeight) viewport_height=\(s.viewportHeight) inset_top=\(s.insetTop) inset_bottom=\(s.insetBottom)"
                + " pan_y=\(s.panY) velocity_y=\(s.velocityY) pan_state=\(s.panState) dragging=\(s.dragging) decelerating=\(s.decelerating)"
                + " anchor_viewport_y=\(s.anchorViewportY) original_content_y=\(s.originalContentY) content_y=\(s.contentY)"
                + " preference_content_height=\(s.preferenceContentHeight) clamp_correction_y=\(s.clampCorrectionY) offset_before_extent=\(s.offsetBeforeExtent)"
                + " candidate_offset_y=\(s.candidateOffsetY) extent_committed=\(s.extentCommitted) first_changed=\(s.firstChanged) generation_changed=\(s.generationChanged)"
        }
    }

    private let clock: () -> TimeInterval
    private let emit: (Record) -> Void
    private var remaining = 128
    private var started: TimeInterval?
    private var preferenceTime: TimeInterval?
    private var preferenceOffsetY: CGFloat = .nan
    private var lastMotionTime: TimeInterval?
    private var motionRemaining = 0
    private var recordedAwaitExtent = false
    private(set) var captureGeneration = 0

    static func configured(environment: [String: String]) -> IrisTimelineAnchorTrace? {
        guard environment["IRIS_UI_TEST_TRACE_PAGINATION"] == "1",
              environment["IRIS_UI_TEST_RESET"] == "1",
              environment["IRIS_UI_TEST_SEED_PEER"] == "self",
              environment["IRIS_UI_TEST_SEED_COUNT"].flatMap(Int.init).map({ $0 > 0 }) == true else { return nil }
        let trace = IrisTimelineAnchorTrace()
        trace.emit(Record(event: .ready, origin: .direct, elapsedMilliseconds: 0,
                          preferenceAgeMilliseconds: .nan, preferenceOffsetY: .nan, sample: Sample()))
        return trace
    }

    init(clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
         emit: @escaping (Record) -> Void = {
             let line = $0.line
             irisInteractionLogger.notice("\(line, privacy: .public)")
             // XCTest's app StandardOutputAndStandardError attachment preserves
             // this bounded synthetic-only mirror even without a log archive.
             FileHandle.standardError.write(Data((line + "\n").utf8))
         }) {
        self.clock = clock
        self.emit = emit
    }

    func preferenceDelivered(offsetY: CGFloat) {
        preferenceTime = clock()
        preferenceOffsetY = offsetY
    }

    func begin(_ sample: () -> Sample) {
        guard remaining > 0 else { return }
        started = clock()
        captureGeneration += 1
        motionRemaining = 12
        lastMotionTime = nil
        recordedAwaitExtent = false
        record(.capture, sample: sample)
    }

    @discardableResult
    func record(_ event: Event, origin: Origin = .direct, sample: () -> Sample) -> Bool {
        guard remaining > 0, let started else { return false }
        let now = clock()
        guard now - started <= 5 else { return false }
        if event == .awaitExtent {
            guard !recordedAwaitExtent else { return false }
            recordedAwaitExtent = true
        }
        if event == .nativeOffset {
            guard motionRemaining > 0, lastMotionTime.map({ now - $0 >= 0.1 }) ?? true else { return false }
            motionRemaining -= 1
            lastMotionTime = now
        }
        remaining -= 1
        emit(Record(event: event, origin: origin, elapsedMilliseconds: max(0, now - started) * 1_000,
                    preferenceAgeMilliseconds: preferenceTime.map { max(0, now - $0) * 1_000 } ?? .nan,
                    preferenceOffsetY: preferenceOffsetY, sample: sample()))
        return true
    }
}
#endif

protocol IrisInteractionMessage {
    var id: String { get }
    var body: String { get }
    var isOutgoing: Bool { get }
    var createdAtSecs: UInt64 { get }
}

@MainActor
final class IrisInteractionTimingController {
    private let explicitlyEnabled: Bool
    private(set) var timing: IrisInteractionTiming?

    init(environment: [String: String], enabledInBundle: Bool =
         Bundle.main.object(forInfoDictionaryKey: "IrisPerformanceTracing") as? Bool == true) {
        timing = IrisInteractionTiming.configured(environment: environment, enabledInBundle: enabledInBundle)
        explicitlyEnabled = timing != nil
    }

    func update(debugLoggingEnabled: Bool) {
        // Explicit test/performance launches remain enabled across ordinary
        // state updates with the preference off. Otherwise off releases all
        // pending message IDs and bodies with the helper itself.
        guard !explicitlyEnabled else { return }
        timing = debugLoggingEnabled ? timing ?? IrisInteractionTiming() : nil
    }
}

/// Opt-in measurements of state and visible layout, not display/paint completion.
@MainActor
final class IrisInteractionTiming {
    enum Action: String { case open, send }
    enum Stage: String { case state, visibleLayout = "visible_layout" }
    struct Record {
        let action: Action
        let stage: Stage
        let durationMilliseconds: Double
        let messageCount: Int
    }
    private struct Pending {
        let action: Action
        let started: TimeInterval
        let chatID: String
        var targetID: String?
        var body: String? = nil
        var previousIDs: Set<String> = []
        var hasState = false
        var earliestCreatedAtSecs: UInt64 = 0
    }
    private var pending: [Pending] = []
    private let clock: () -> TimeInterval
    private let wallClock: () -> TimeInterval
    private let emit: (Record) -> Void

    static func configured(environment: [String: String], enabledInBundle: Bool) -> IrisInteractionTiming? {
        guard environment["IRIS_PERF_LOG"] == "1" || enabledInBundle else { return nil }
        return IrisInteractionTiming()
    }

    init(clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
         wallClock: @escaping () -> TimeInterval = { Date().timeIntervalSince1970 },
         emit: @escaping (Record) -> Void = {
             irisInteractionLogger.notice("iris.interaction action=\($0.action.rawValue, privacy: .public) stage=\($0.stage.rawValue, privacy: .public) duration_ms=\($0.durationMilliseconds, privacy: .public) message_count=\($0.messageCount, privacy: .public)")
         }) {
        self.clock = clock
        self.wallClock = wallClock
        self.emit = emit
    }

    func beginOpen(chatID: String, targetID: String?) {
        pending.removeAll()
        pending.append(Pending(action: .open, started: clock(), chatID: chatID, targetID: targetID))
    }

    func beginSend<M: IrisInteractionMessage>(chatID: String, body: String, messages: [M]) {
        if pending.count >= 8 { pending.removeFirst() }
        pending.append(Pending(action: .send, started: clock(), chatID: chatID, body: body,
                               previousIDs: Set(messages.map(\.id)),
                               earliestCreatedAtSecs: UInt64(max(0, wallClock()))))
    }

    func stateAvailable<M: IrisInteractionMessage>(chatID: String, messages: [M], historyLoaded: Bool) {
        for index in pending.indices where pending[index].chatID == chatID && !pending[index].hasState {
            if pending[index].action == .send {
                guard let message = messages.first(where: {
                    $0.isOutgoing && $0.body == pending[index].body
                        && $0.createdAtSecs >= pending[index].earliestCreatedAtSecs
                        && !pending[index].previousIDs.contains($0.id)
                }) else { continue }
                for other in pending.indices where other != index && pending[other].action == .send {
                    pending[other].previousIDs.insert(message.id)
                }
                pending[index].targetID = message.id
                pending[index].body = nil
                pending[index].previousIDs.removeAll()
            } else if let target = pending[index].targetID {
                guard messages.contains(where: { $0.id == target }) else { continue }
            } else {
                guard !messages.isEmpty || historyLoaded else { continue }
                pending[index].targetID = messages.last?.id
            }
            pending[index].hasState = true
            record(pending[index], stage: .state, count: messages.count)
        }
    }

    func layout(chatID: String, frames: [String: CGRect], viewportMinY: CGFloat,
                viewportMaxY: CGFloat, ready: Bool, messageCount: Int) {
        guard ready, viewportMaxY > viewportMinY else { return }
        pending.removeAll { trace in
            guard trace.chatID == chatID, trace.hasState else { return false }
            if let id = trace.targetID {
                guard let frame = frames[id], frame.width > 0, frame.height > 0,
                      frame.maxY > viewportMinY, frame.minY < viewportMaxY else { return false }
            } else if messageCount != 0 { return false }
            record(trace, stage: .visibleLayout, count: messageCount)
            return true
        }
    }

    private func record(_ trace: Pending, stage: Stage, count: Int) {
        emit(Record(action: trace.action, stage: stage,
                    durationMilliseconds: max(0, clock() - trace.started) * 1_000, messageCount: count))
    }
}
