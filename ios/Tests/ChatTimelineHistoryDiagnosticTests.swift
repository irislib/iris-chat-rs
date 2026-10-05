#if DEBUG && os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class ChatTimelineHistoryDiagnosticTests: XCTestCase {
    func testOnlyTheExistingAnonymousResetMixedHeightFixtureEnablesRecording() {
        let fixture = ["IRIS_UI_TEST_RESET": "1", "IRIS_UI_TEST_BYPASS_KEYCHAIN": "1",
                       "IRIS_UI_TEST_SEED_PEER": "self", "IRIS_UI_TEST_SEED_MIXED_HEIGHTS": "1"]
        XCTAssertNotNil(ChatTimelineHistoryDiagnostic.configured(environment: fixture))
        for key in fixture.keys {
            var unsafe = fixture
            unsafe.removeValue(forKey: key)
            XCTAssertNil(ChatTimelineHistoryDiagnostic.configured(environment: unsafe))
        }
        var otherPeer = fixture
        otherPeer["IRIS_UI_TEST_SEED_PEER"] = "another-person"
        XCTAssertNil(ChatTimelineHistoryDiagnostic.configured(environment: otherPeer))
        XCTAssertNil(ChatTimelineHistoryDiagnostic.configured(environment: [:]))
    }

    func testCapacityPreservesCaptureAndRecentPanHistoryAndStopsBeforeSingleExport() {
        var exports: [String] = []
        let diagnostic = ChatTimelineHistoryDiagnostic(capacity: 6, precedingCapacity: 2) { exports.append($0) }
        for time in 0..<12 { add(.nativeOffset, at: Double(time), to: diagnostic) }
        add(.capture, at: 12, to: diagnostic)
        add(.applyAfter, at: 13, to: diagnostic)
        add(.panEnded, at: 14, to: diagnostic)
        add(.settlement, at: 15, to: diagnostic)
        XCTAssertEqual(diagnostic.count, 6)
        XCTAssertEqual(diagnostic.sample(at: 0)?.time, 10)
        XCTAssertEqual(diagnostic.sample(at: 1)?.time, 11)
        XCTAssertEqual(diagnostic.sample(at: 2)?.event, .capture)
        XCTAssertFalse(diagnostic.prepare(.nativeOffset), "Post-capture overflow must not overwrite the evidence")
        diagnostic.exportOnce()
        XCTAssertTrue(diagnostic.exported)
        XCTAssertEqual(exports.count, 1)
        XCTAssertEqual(exports[0].split(separator: "\n").count, 6)
        XCTAssertFalse(diagnostic.prepare(.panBegan), "No later pan may evaluate native getters or mutate the exported buffer")
        diagnostic.exportOnce()
        XCTAssertEqual(exports.count, 1)
        XCTAssertEqual(diagnostic.sample(at: 2)?.event, .capture)
    }

    @MainActor
    func testDisabledAndExhaustedRecorderDoesNotReadNativeGeometry() {
        let scroll = DiagnosticCountingScrollView()
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyDiagnostic = nil
        scroll.offsetReads = 0
        coordinator.recordHistoryDiagnostic(.nativeOffset)
        XCTAssertEqual(scroll.offsetReads, 0)
        let full = ChatTimelineHistoryDiagnostic(capacity: 1, precedingCapacity: 0, emit: { _ in })
        add(.capture, at: 1, to: full)
        coordinator.historyDiagnostic = full
        coordinator.recordHistoryDiagnostic(.nativeOffset)
        XCTAssertEqual(scroll.offsetReads, 0, "Capacity must be checked before evaluating a sample")
        _ = full.prepare(.applyAfter)
        _ = full.prepare(.panEnded)
        full.exportOnce()
        XCTAssertTrue(full.exported)
        coordinator.recordHistoryDiagnostic(.panChanged)
        XCTAssertEqual(scroll.offsetReads, 0, "Exported storage must never be touched by a later pan")
    }

    @MainActor
    func testProductionExportRunsOnceOnTheNextTurnAndDoesNotMoveTheScroll() async {
        let emitted = expectation(description: "anonymous pagination export")
        var exports: [String] = []
        let diagnostic = ChatTimelineHistoryDiagnostic { text in
            exports.append(text)
            emitted.fulfill()
        }
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 1000
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyDiagnostic = diagnostic
        add(.capture, at: 1, to: diagnostic)
        add(.applyAfter, at: 2, to: diagnostic)
        coordinator.recordHistoryDiagnostic(.panEnded)
        coordinator.scheduleHistoryDiagnosticExport()
        coordinator.scheduleHistoryDiagnosticExport()
        XCTAssertTrue(exports.isEmpty, "Pan end only schedules an export; it must not format records inline")
        await fulfillment(of: [emitted], timeout: 2)
        XCTAssertEqual(exports.count, 1)
        XCTAssertTrue(diagnostic.exported)
        XCTAssertEqual(diagnostic.sample(at: diagnostic.count - 1)?.event, .settlement)
        XCTAssertEqual(scroll.contentOffset.y, 100)
    }

    @MainActor
    func testProductionApplyRecordsOriginAndBothOffsetsWithoutExportingMessageIdentifiers() {
        var exports: [String] = []
        let diagnostic = ChatTimelineHistoryDiagnostic(emit: { exports.append($0) })
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize = CGSize(width: 390, height: 1000)
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.historyDiagnostic = diagnostic
        coordinator.messageContentFrames = ["secret-row": CGRect(x: 0, y: 120, width: 100, height: 40)]
        coordinator.latestPage = ChatTimelinePageFrames(chatID: "secret-chat", firstMessageID: "secret-first",
            contentFrames: ["secret-row": CGRect(x: 0, y: 220, width: 100, height: 40)])
        coordinator.captureHistoryViewportAnchor(chatID: "secret-chat", firstMessageID: "secret-first",
                                                viewportMinY: 0, viewportMaxY: 600)
        scroll.contentSize.height = 1800
        let page = ChatTimelinePageFrames(chatID: "secret-chat", firstMessageID: "secret-older-first",
            layoutGeneration: 1, contentFrames: ["secret-row": CGRect(x: 0, y: 1020, width: 100, height: 40)],
            contentHeight: 1800)
        coordinator.latestPage = page
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 900, "Diagnostic hooks must preserve the original correction")
        let before = (0..<diagnostic.count).compactMap { diagnostic.sample(at: $0) }.first { $0.event == .applyBefore }
        let after = (0..<diagnostic.count).compactMap { diagnostic.sample(at: $0) }.first { $0.event == .applyAfter }
        XCTAssertEqual(before?.origin, .preference)
        XCTAssertEqual(before?.offsetY, 100)
        XCTAssertEqual(before?.targetOffsetY, 900)
        XCTAssertEqual(after?.origin, .preference)
        XCTAssertEqual(after?.offsetY, 900)
        diagnostic.exportOnce()
        XCTAssertTrue(exports.isEmpty, "An applied correction cannot export while the touch is active")
        coordinator.recordHistoryDiagnostic(.panEnded)
        XCTAssertTrue(exports.isEmpty, "The touch callbacks must never format/export records")
        diagnostic.exportOnce()
        XCTAssertEqual(exports.count, 1)
        XCTAssertFalse(exports[0].contains("secret"))
    }

    private func add(_ event: ChatTimelineHistoryDiagnostic.Event, at time: Double,
                     to diagnostic: ChatTimelineHistoryDiagnostic) {
        guard diagnostic.prepare(event) else { return }
        var sample = ChatTimelineHistoryDiagnostic.Sample()
        sample.time = time
        diagnostic.store(sample, event: event, origin: .direct)
    }
}

private final class DiagnosticCountingScrollView: UIScrollView {
    var offsetReads = 0
    override var contentOffset: CGPoint {
        get { offsetReads += 1; return super.contentOffset }
        set { super.contentOffset = newValue }
    }
}
#endif
