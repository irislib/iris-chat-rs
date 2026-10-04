import CoreGraphics
import XCTest
#if os(iOS)
@testable import IrisChat
#elseif os(macOS)
@testable import IrisChatMac
#endif

final class ChatTimelineInitialPlacementTests: XCTestCase {
    func testInitialScrollRealizesLazyLastBubbleThenWaitsForItsVisibleFrame() {
        var placement = ChatTimelineInitialPlacement()
        XCTAssertEqual(placement.update(targetID: "last", frame: nil, viewportMinY: 0, viewportMaxY: 0), .wait)
        XCTAssertTrue(placement.isPending)
        XCTAssertEqual(placement.update(targetID: "last", frame: nil, viewportMinY: 0, viewportMaxY: 600), .scroll)
        XCTAssertTrue(placement.isAwaitingVisibility)
        XCTAssertEqual(placement.update(targetID: "last", frame: nil, viewportMinY: 0, viewportMaxY: 600), .wait)
        let below = CGRect(x: 0, y: 1_000, width: 100, height: 40)
        XCTAssertEqual(placement.update(targetID: "last", frame: below, viewportMinY: 0, viewportMaxY: 0), .wait)
        XCTAssertEqual(placement.update(targetID: "last", frame: below, viewportMinY: 0, viewportMaxY: 600), .scroll)
        XCTAssertTrue(placement.isAwaitingVisibility)
        XCTAssertEqual(placement.update(targetID: "last", frame: below, viewportMinY: 0, viewportMaxY: 600), .wait)
        let landed = CGRect(x: 0, y: 560, width: 100, height: 40)
        XCTAssertEqual(placement.update(targetID: "last", frame: landed, viewportMinY: 0, viewportMaxY: 600), .reveal)
        XCTAssertFalse(placement.isAwaitingVisibility)
        XCTAssertEqual(placement.update(targetID: "last", frame: landed, viewportMinY: 0, viewportMaxY: 600), .wait)
    }

    func testAlreadyVisibleAndTallLastBubblesNeedNoTimerOrAdditionalGeometryChange() {
        for frame in [CGRect(x: 0, y: 560, width: 100, height: 40), CGRect(x: 0, y: -400, width: 100, height: 1_000)] {
            var placement = ChatTimelineInitialPlacement()
            XCTAssertEqual(placement.update(targetID: "last", frame: frame, viewportMinY: 0, viewportMaxY: 600), .scrollAndReveal)
            XCTAssertFalse(placement.isAwaitingVisibility)
        }
    }

    func testSearchOrPrependCancellationAndChatResetDoNotReuseAnOldPlacement() {
        var placement = ChatTimelineInitialPlacement()
        let below = CGRect(x: 0, y: 1_000, width: 100, height: 40)
        XCTAssertEqual(placement.update(targetID: "old", frame: below, viewportMinY: 0, viewportMaxY: 600), .scroll)
        placement.cancel()
        XCTAssertEqual(placement.update(targetID: "old", frame: below, viewportMinY: 0, viewportMaxY: 600), .wait)
        placement.reset()
        XCTAssertTrue(placement.isPending)
        XCTAssertFalse(placement.isAwaitingVisibility)
        XCTAssertEqual(placement.update(targetID: "new", frame: nil, viewportMinY: 0, viewportMaxY: 600), .scroll)
        XCTAssertEqual(placement.update(targetID: "new", frame: below, viewportMinY: 0, viewportMaxY: 600), .scroll)
    }

    func testNewLastBubbleDuringInitialPlacementMustAlsoBeMeasured() {
        var placement = ChatTimelineInitialPlacement()
        let below = CGRect(x: 0, y: 1_000, width: 100, height: 40)
        XCTAssertEqual(placement.update(targetID: "old", frame: below, viewportMinY: 0, viewportMaxY: 600), .scroll)
        XCTAssertEqual(placement.update(targetID: "new", frame: nil, viewportMinY: 0, viewportMaxY: 600), .scroll)
        XCTAssertEqual(placement.update(targetID: "new", frame: below, viewportMinY: 0, viewportMaxY: 600), .scroll)
    }

    func testCallOrSystemMessageWaitsForItsOwnMeasuredContent() {
        var placement = ChatTimelineInitialPlacement()
        XCTAssertEqual(placement.update(targetID: "notice", frame: nil, viewportMinY: 0, viewportMaxY: 600), .scroll)
        XCTAssertEqual(placement.update(targetID: "notice", frame: nil, viewportMinY: 0, viewportMaxY: 600), .wait)
        let below = CGRect(x: 0, y: 1_000, width: 100, height: 40)
        XCTAssertEqual(placement.update(targetID: "notice", frame: below, viewportMinY: 0, viewportMaxY: 600), .scroll)
        let visible = CGRect(x: 0, y: 560, width: 100, height: 40)
        XCTAssertEqual(placement.update(targetID: "notice", frame: visible, viewportMinY: 0, viewportMaxY: 600), .reveal)
    }

    func testChangingEstimatedRowsCorrectsPlacementWithoutRepeatingAnUnchangedScroll() {
        var placement = ChatTimelineInitialPlacement()
        XCTAssertEqual(placement.update(targetID: "last", frame: nil, viewportMinY: 0, viewportMaxY: 600), .scroll)
        for y in [2_000.0, 1_000.0] {
            let frame = CGRect(x: 0, y: y, width: 100, height: 80)
            XCTAssertEqual(placement.update(targetID: "last", frame: frame, viewportMinY: 0, viewportMaxY: 600), .scroll)
            XCTAssertEqual(placement.update(targetID: "last", frame: frame, viewportMinY: 0, viewportMaxY: 600), .wait)
        }
        let landed = CGRect(x: 0, y: 520, width: 100, height: 80)
        XCTAssertEqual(placement.update(targetID: "last", frame: landed, viewportMinY: 0, viewportMaxY: 600), .reveal)
        XCTAssertFalse(placement.isAwaitingVisibility)
    }

    func testPartiallyVisibleLastRowWaitsUntilItsLastLineClearsTheComposer() {
        for frame in [CGRect(x: 0, y: 590, width: 100, height: 80),
                      CGRect(x: 0, y: -400, width: 100, height: 1_100)] {
            var placement = ChatTimelineInitialPlacement()
            XCTAssertEqual(placement.update(targetID: "last", frame: frame,
                                            viewportMinY: 0, viewportMaxY: 600), .scroll)
            XCTAssertTrue(placement.isAwaitingVisibility)
            let landed = frame.offsetBy(dx: 0, dy: 600 - frame.maxY)
            XCTAssertEqual(placement.update(targetID: "last", frame: landed,
                                            viewportMinY: 0, viewportMaxY: 600), .reveal)
        }
    }

}
