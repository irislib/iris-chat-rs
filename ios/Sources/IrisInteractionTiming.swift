import Foundation
import CoreGraphics
import OSLog

private let irisInteractionLogger = Logger(subsystem: "fi.siriusbusiness.irischat", category: "interaction")

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
