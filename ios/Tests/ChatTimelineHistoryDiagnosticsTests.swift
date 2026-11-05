#if os(iOS)
import UIKit
import XCTest
@testable import IrisChat

final class ChatTimelineHistoryDiagnosticsTests: XCTestCase {
    func testCompactTraceRetainsCorrectionBoundariesAfterOffsetNoise() {
        let trace = ChatTimelineHistoryDiagnostics()
        var sample = ChatTimelineHistoryDiagnostics.Snapshot(
            generation: 38, pageGeneration: 0, panState: 2, dragging: true,
            offsetY: 1004.33, extent: 10016, originalY: 1200,
            baseY: 1004.33, translationY: 40)
        trace.record("capture", fields: "", snapshot: sample)
        sample.pageGeneration = 1
        sample.contentY = 9200
        sample.anchorHeight = 18913.67
        trace.record("restore", fields: "", snapshot: sample)
        sample.targetY = 8884.33
        sample.correctionPanY = 160
        sample.completedTranslationY = 160
        trace.record("apply", fields: "", snapshot: sample)
        let applied = sample.encoded("A", sequence: 3)
        for index in 0..<(ChatTimelineHistoryDiagnostics.capacity + 3) {
            trace.record("offset", fields: "", snapshot: .init(generation: 41, pageGeneration: 1,
                panState: 3, offsetY: CGFloat(index), extent: 18913.67))
        }
        let exposed = String(trace.accessibilityText.prefix(512))
        XCTAssertEqual(exposed, trace.accessibilityText)
        XCTAssertTrue(exposed.contains("N[n,g,p,s,d,i,o,h,ah]=166,41,1,3,0,0,162.00,18913.67,-"))
        XCTAssertTrue(exposed.contains("C[n,g,p,o,h,y,b,t]=1,38,0,1004.33,10016.00,1200.00,1004.33,40.00"))
        XCTAssertTrue(exposed.contains("R[n,g,p,o,h,y,y2,ah]=2,38,1,1004.33,10016.00,1200.00,9200.00,18913.67"))
        XCTAssertTrue(exposed.contains(applied))
        XCTAssertEqual(trace.text.split(separator: "\n").count, ChatTimelineHistoryDiagnostics.capacity)
    }

    func testCompactTraceFitsSnapshotLimitForExceptionalCoordinatesAndCounters() {
        let coordinates: [CGFloat?] = [nil, 0, -0.01, 9999.99, -9999.99, 100000,
                                      CGFloat.greatestFiniteMagnitude, -CGFloat.greatestFiniteMagnitude,
                                      CGFloat.leastNonzeroMagnitude, CGFloat.infinity, -CGFloat.infinity, CGFloat.nan]
        for value in coordinates {
            let trace = ChatTimelineHistoryDiagnostics()
            let sample = ChatTimelineHistoryDiagnostics.Snapshot(
                generation: .max, pageGeneration: .min, panState: .max,
                dragging: true, decelerating: true, offsetY: value ?? 0, extent: value ?? 0,
                originalY: value, contentY: value, anchorHeight: value, clampY: value,
                baseY: value, translationY: value, completedTranslationY: value,
                targetY: value, correctionPanY: value)
            for event in ["capture", "restore", "apply", "pan-3"] {
                trace.record(event, fields: "", snapshot: sample)
            }
            XCTAssertLessThanOrEqual(trace.accessibilityText.utf8.count, 512)
            XCTAssertTrue(trace.accessibilityText.unicodeScalars.allSatisfy(\.isASCII))
            for kind in ["N", "C", "R", "A"] {
                let encoded = sample.encoded(kind, sequence: .max)
                XCTAssertTrue(encoded.contains("ovf"), "Unrepresentable counters must be explicit")
                XCTAssertTrue(encoded.split(separator: "=")[1].split(separator: ",").allSatisfy { $0.count <= 8 })
            }
        }
    }

    @MainActor
    func testCoordinatorRecordsCorrectionBeforeClearingItsAnchor() throws {
        let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        scroll.contentSize.height = 3000
        scroll.contentOffset.y = 100
        let coordinator = ChatTimelineInteractionCoordinator()
        coordinator.scrollView = scroll
        // The recorder is available only for the existing synthetic reset fixture.
        coordinator.historyDiagnostics = ChatTimelineHistoryDiagnostics.configured(environment: [
            "IRIS_UI_TEST_TIMELINE_TRACE": "1", "IRIS_UI_TEST_RESET": "1", "IRIS_UI_TEST_SEED_PEER": "self",
        ])
        coordinator.historyViewportAnchor = ChatTimelineHistoryAnchor(
            chatID: "synthetic", firstMessageID: "private-first-token",
            messageID: "private-message-token", originalContentY: 220)
        coordinator.recordHistoryDiagnostic("capture")
        let page = ChatTimelinePageFrames(chatID: "synthetic", firstMessageID: "1",
            contentFrames: ["private-message-token": CGRect(x: 0, y: 1020, width: 100, height: 80)], contentHeight: 3000)
        XCTAssertTrue(coordinator.restoreHistoryViewportAnchor(page: page))
        XCTAssertEqual(scroll.contentOffset.y, 900)
        coordinator.recordNativeScrollPosition()
        let exposed = try XCTUnwrap(scroll.accessibilityValue)
        XCTAssertLessThanOrEqual(exposed.utf8.count, 512)
        XCTAssertTrue(exposed.contains("220.00,1020.00,0.00,-,-,-,900.00,-"))
        XCTAssertFalse(exposed.contains("synthetic"))
        XCTAssertFalse(exposed.contains("private-message-token"))
        XCTAssertFalse(exposed.contains("private-first-token"))
    }
}
#endif
