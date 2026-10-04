#if os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class ChatTimelineAnchorTraceTests: XCTestCase {
    func testConfigurationRequiresExplicitAnonymousResetFixture() {
        let environment = ["IRIS_UI_TEST_TRACE_PAGINATION": "1", "IRIS_UI_TEST_RESET": "1",
                           "IRIS_UI_TEST_SEED_PEER": "self", "IRIS_UI_TEST_SEED_COUNT": "160"]
        for key in environment.keys {
            var missing = environment
            missing.removeValue(forKey: key)
            XCTAssertNil(IrisTimelineAnchorTrace.configured(environment: missing), key)
        }
        var invalid = environment
        invalid["IRIS_UI_TEST_SEED_PEER"] = "other"
        XCTAssertNil(IrisTimelineAnchorTrace.configured(environment: invalid))
        invalid = environment
        invalid["IRIS_UI_TEST_TRACE_PAGINATION"] = "true"
        XCTAssertNil(IrisTimelineAnchorTrace.configured(environment: invalid))
        invalid = environment
        invalid["IRIS_UI_TEST_SEED_COUNT"] = "0"
        XCTAssertNil(IrisTimelineAnchorTrace.configured(environment: invalid))
        XCTAssertNotNil(IrisTimelineAnchorTrace.configured(environment: environment))
    }

    func testDisabledInactiveExpiredAndExhaustedTracingDoesNotEvaluateSupplier() {
        var now = 0.0
        var evaluations = 0
        let supplier = { evaluations += 1; return IrisTimelineAnchorTrace.Sample() }
        let disabled: IrisTimelineAnchorTrace? = nil
        disabled?.begin(supplier)
        disabled?.record(.apply, sample: supplier)
        let inactive = IrisTimelineAnchorTrace(clock: { now }, emit: { _ in })
        inactive.record(.apply, sample: supplier)
        XCTAssertEqual(evaluations, 0)
        inactive.begin(supplier)
        now = 5.01
        inactive.record(.apply, sample: supplier)
        XCTAssertEqual(evaluations, 1)
        now = 6
        let bounded = IrisTimelineAnchorTrace(clock: { now }, emit: { _ in })
        bounded.begin(supplier)
        for _ in 0..<127 { bounded.record(.apply, sample: supplier) }
        let before = evaluations
        bounded.record(.apply, sample: supplier)
        bounded.begin(supplier)
        XCTAssertEqual(evaluations, before)
    }

    func testMotionIsThrottledAndBoundedWithoutSuppressingTouchEnd() {
        var now = 0.0
        var records: [IrisTimelineAnchorTrace.Record] = []
        let trace = IrisTimelineAnchorTrace(clock: { now }, emit: { records.append($0) })
        trace.begin { .init() }
        for _ in 0..<30 {
            trace.record(.nativeOffset) { .init() }
            trace.record(.nativeOffset) { XCTFail("Repeated motion must stay lazy"); return .init() }
            now += 0.11
        }
        trace.record(.panEnded) { .init(panY: 227.7, velocityY: 250, panState: 3) }
        XCTAssertEqual(records.filter { $0.event == .nativeOffset }.count, 12)
        XCTAssertEqual(records.last?.event, .panEnded)
        XCTAssertEqual(records.last?.sample.panY, 227.7)
        trace.record(.awaitExtent) { .init() }
        trace.record(.awaitExtent) { XCTFail("Repeated extent waits must stay lazy"); return .init() }
        XCTAssertEqual(records.filter { $0.event == .awaitExtent }.count, 1)
    }

    @MainActor
    func testProductionCaptureAndApplyRecordCachedGeometryAndLiveOffsetWithoutChangingMath() throws {
        var now = 10.0
        var records: [IrisTimelineAnchorTrace.Record] = []
        let trace = IrisTimelineAnchorTrace(clock: { now }, emit: { records.append($0) })
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentInsetAdjustmentBehavior = .never
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.anchorTrace = trace
        coordinator.latestPage = ChatTimelinePageFrames(chatID: "anonymous", firstMessageID: "81",
            frames: ["83": CGRect(x: 0, y: 120, width: 100, height: 80)],
            contentFrames: ["83": CGRect(x: 0, y: 220, width: 100, height: 80)], contentHeight: 3_000)
        coordinator.messageContentFrames = coordinator.latestPage.frames
        trace.preferenceDelivered(offsetY: scroll.contentOffset.y)
        scroll.contentOffset.y = 160
        now = 10.4
        coordinator.captureHistoryViewportAnchor(chatID: "anonymous", firstMessageID: "81",
                                                  viewportMinY: 0, viewportMaxY: 600)
        let captured = try XCTUnwrap(records.first)
        XCTAssertEqual(captured.preferenceAgeMilliseconds, 400, accuracy: 0.001)
        XCTAssertEqual(captured.preferenceOffsetY, 100)
        XCTAssertEqual(captured.sample.offsetY, 160)
        XCTAssertEqual(captured.sample.anchorViewportY, 120)
        XCTAssertEqual(captured.sample.originalContentY, 220)
        XCTAssertTrue(captured.sample.extentCommitted)
        let page = ChatTimelinePageFrames(chatID: "anonymous", firstMessageID: "1",
            contentFrames: ["83": CGRect(x: 0, y: 1_020, width: 100, height: 80)], contentHeight: 3_000)
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 960)
        let applied = try XCTUnwrap(records.first { $0.event == .apply })
        XCTAssertEqual(applied.origin, .preference)
        XCTAssertEqual(applied.sample.offsetY, 160)
        XCTAssertEqual(applied.sample.candidateOffsetY, 960)
        XCTAssertEqual(applied.sample.contentY - applied.sample.originalContentY, 800)
        XCTAssertTrue(records.first { $0.event == .restore }?.sample.firstChanged == true)
        XCTAssertFalse(applied.line.contains("anonymous"))
        XCTAssertFalse(applied.line.contains("message_count"))
        XCTAssertFalse(applied.line.contains("chat_id"))
        // Tracing does not retain or modify the preference value or anchor.
        XCTAssertEqual(coordinator.latestPage.firstMessageID, "81")
        XCTAssertNil(coordinator.historyViewportAnchor)
    }

    @MainActor
    func testPostApplySamplesTheLaterNativeOffsetWithoutWritingIt() async throws {
        var records: [IrisTimelineAnchorTrace.Record] = []
        let postApply = expectation(description: "next main turn sample")
        let trace = IrisTimelineAnchorTrace(emit: {
            records.append($0)
            if $0.event == .postApply { postApply.fulfill() }
        })
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 3_000
        scroll.contentOffset.y = 160
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        coordinator.anchorTrace = trace
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            messageID: "83", originalContentY: 220, contentY: 1_020, contentHeight: 3_000)
        trace.begin { .init() }
        XCTAssertTrue(coordinator.applyPendingHistoryViewportAnchor(origin: .nativeLayout))
        XCTAssertEqual(scroll.contentOffset.y, 960)
        // Model a subsequent UIKit mutation; diagnostics must only observe it.
        scroll.contentOffset.y = 1_000
        await fulfillment(of: [postApply], timeout: 2)
        let sample = try XCTUnwrap(records.first { $0.event == .postApply })
        XCTAssertEqual(sample.origin, .mainTurn)
        XCTAssertEqual(sample.sample.candidateOffsetY, 960)
        XCTAssertEqual(sample.sample.offsetY, 1_000)
        XCTAssertEqual(scroll.contentOffset.y, 1_000)
    }
}
#endif
