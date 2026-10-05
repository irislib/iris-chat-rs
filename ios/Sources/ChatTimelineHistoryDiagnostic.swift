#if DEBUG && os(iOS)
import UIKit

/// Temporary, anonymous-fixture-only evidence. Touch callbacks only store scalars.
final class ChatTimelineHistoryDiagnostic {
    enum Event: UInt8 {
        case panBegan, panChanged, panEnded, capture, extentWill, extentDid
        case restore, applyBefore, nativeOffset, applyAfter, settlement
    }
    enum Origin: UInt8 { case direct, preference, mainTurn, nativeLayout, layoutTurn }
    struct Sample {
        var time: Double = 0
        var event = Event.nativeOffset
        var origin = Origin.direct
        var offsetY: CGFloat = .nan
        var nativeHeight: CGFloat = .nan
        var viewportHeight: CGFloat = .nan
        var insetTop: CGFloat = .nan
        var insetBottom: CGFloat = .nan
        var panY: CGFloat = .nan
        var velocityY: CGFloat = .nan
        var panState = 0
        var dragging = false
        var tracking = false
        var decelerating = false
        var ownOffsetWrite = false
        var layoutGeneration = 0
        var anchorGeneration = 0
        var originalContentY: CGFloat = .nan
        var contentY: CGFloat = .nan
        var preferenceViewportY: CGFloat = .nan
        var preferenceHeight: CGFloat = .nan
        var clampCorrectionY: CGFloat = 0
        var targetOffsetY: CGFloat = .nan
    }

    private var storage: [Sample]
    private let precedingCapacity: Int
    private let emit: (String) -> Void
    private var next = 0
    private(set) var count = 0
    private var captureTime = Double.nan
    private(set) var hasCapture = false
    private(set) var hasApply = false
    private(set) var panHasEnded = false
    private(set) var exported = false
    var exportScheduled = false
    var nextApplyOrigin = Origin.direct
    var writeOrigin = Origin.direct
    var ownOffsetWrite = false

    static func configured(environment: [String: String]) -> ChatTimelineHistoryDiagnostic? {
        guard environment["IRIS_UI_TEST_RESET"] == "1",
              environment["IRIS_UI_TEST_BYPASS_KEYCHAIN"] == "1",
              environment["IRIS_UI_TEST_SEED_PEER"] == "self",
              environment["IRIS_UI_TEST_SEED_MIXED_HEIGHTS"] == "1" else { return nil }
        return ChatTimelineHistoryDiagnostic()
    }

    init(capacity: Int = 512, precedingCapacity: Int = 128,
         emit: @escaping (String) -> Void = irisEmitAnonymousPaginationDiagnostic) {
        let capacity = max(1, capacity)
        storage = Array(repeating: Sample(), count: capacity)
        self.precedingCapacity = min(max(0, precedingCapacity), capacity - 1)
        self.emit = emit
    }

    // Checked before clocks/native getters. No closures or growing arrays on this path.
    func prepare(_ event: Event) -> Bool {
        guard !exported else { return false }
        if event == .capture, !hasCapture {
            count = min(count, precedingCapacity)
            hasCapture = true
        }
        if event == .panBegan { panHasEnded = false }
        if event == .panEnded { panHasEnded = true }
        if event == .applyAfter { hasApply = true }
        return !hasCapture || count < storage.count
    }

    func store(_ sample: Sample, event: Event, origin: Origin) {
        guard !exported, !hasCapture || count < storage.count else { return }
        var value = sample
        value.event = event
        value.origin = origin
        if event == .capture, captureTime.isNaN { captureTime = value.time }
        storage[next] = value
        next = (next + 1) % storage.count
        count = min(count + 1, storage.count)
    }

    func sample(at index: Int) -> Sample? {
        guard index >= 0, index < count else { return nil }
        return storage[(next - count + storage.count + index) % storage.count]
    }

    func exportOnce() {
        guard hasCapture, hasApply, panHasEnded, !exported else { return }
        // Stop first: subsequent callbacks cannot mutate/copy this array while exporting.
        exported = true
        var text = ""
        text.reserveCapacity(count * 480)
        for index in 0..<count {
            guard let s = sample(at: index) else { continue }
            text += "iris.timeline_anchor_memory event=\(s.event.rawValue) origin=\(s.origin.rawValue) t_ms=\((s.time - captureTime) * 1000) offset=\(s.offsetY) native_height=\(s.nativeHeight) viewport=\(s.viewportHeight) inset_top=\(s.insetTop) inset_bottom=\(s.insetBottom) pan_y=\(s.panY) velocity=\(s.velocityY) pan_state=\(s.panState) dragging=\(s.dragging) tracking=\(s.tracking) decelerating=\(s.decelerating) own_write=\(s.ownOffsetWrite) generation=\(s.layoutGeneration) anchor_generation=\(s.anchorGeneration) original_y=\(s.originalContentY) content_y=\(s.contentY) preference_y=\(s.preferenceViewportY) preference_height=\(s.preferenceHeight) clamp=\(s.clampCorrectionY) target=\(s.targetOffsetY)\n"
        }
        emit(text)
    }
}

extension ChatTimelineInteractionCoordinator {
    func recordHistoryDiagnostic(_ event: ChatTimelineHistoryDiagnostic.Event,
                                 origin: ChatTimelineHistoryDiagnostic.Origin = .direct,
                                 anchor explicitAnchor: ChatTimelineHistoryAnchor? = nil,
                                 targetOffsetY: CGFloat = .nan) {
        guard let diagnostic = historyDiagnostic, diagnostic.prepare(event), let scrollView else { return }
        let pan = scrollView.panGestureRecognizer
        let anchor = explicitAnchor ?? historyViewportAnchor
        var s = ChatTimelineHistoryDiagnostic.Sample()
        s.time = CACurrentMediaTime()
        s.offsetY = scrollView.contentOffset.y
        s.nativeHeight = scrollView.contentSize.height
        s.viewportHeight = scrollView.bounds.height
        s.insetTop = scrollView.adjustedContentInset.top
        s.insetBottom = scrollView.adjustedContentInset.bottom
        s.panY = pan.translation(in: scrollView).y
        s.velocityY = pan.velocity(in: scrollView).y
        s.panState = pan.state.rawValue
        s.dragging = scrollView.isDragging
        s.tracking = scrollView.isTracking
        s.decelerating = scrollView.isDecelerating
        s.ownOffsetWrite = diagnostic.ownOffsetWrite
        s.layoutGeneration = latestPage.layoutGeneration
        s.anchorGeneration = anchor?.layoutGeneration ?? 0
        s.originalContentY = anchor?.originalContentY ?? .nan
        s.contentY = anchor?.contentY ?? .nan
        if let anchor { s.preferenceViewportY = messageContentFrames[anchor.messageID]?.minY ?? .nan }
        s.preferenceHeight = latestPage.contentHeight
        s.clampCorrectionY = anchor?.clampCorrectionY ?? 0
        s.targetOffsetY = targetOffsetY
        diagnostic.store(s, event: event, origin: origin)
    }

    func scheduleHistoryDiagnosticExport(atPanEnd: Bool = false) {
        guard let diagnostic = historyDiagnostic, diagnostic.hasCapture, diagnostic.hasApply,
              diagnostic.panHasEnded, !diagnostic.exported, !diagnostic.exportScheduled else { return }
        guard let scrollView, atPanEnd || !scrollView.isTracking else { return }
        diagnostic.exportScheduled = true
        // Only after touch end; observe the next main turn without delaying/settling the scroll.
        DispatchQueue.main.async { [weak self, weak diagnostic] in
            guard let self, let diagnostic else { return }
            diagnostic.exportScheduled = false
            guard diagnostic.panHasEnded, let scroll = self.scrollView else { return }
            let state = scroll.panGestureRecognizer.state
            guard state != .began, state != .changed,
                  !scroll.isTracking || state == .ended || state == .cancelled || state == .failed else { return }
            self.recordHistoryDiagnostic(.settlement)
            diagnostic.exportOnce()
        }
    }
}
#endif
