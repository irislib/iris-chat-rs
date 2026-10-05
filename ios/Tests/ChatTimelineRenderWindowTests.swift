import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ChatTimelineRenderWindowTests: XCTestCase {
    private func ids(_ range: ClosedRange<Int>) -> [String] { range.map { "row-\($0)" } }

    func testPrependKeepsTheActualVisibleAnchorAfterTheReaderMoved() {
        let old = (81...400).map(String.init)
        let loaded = (1...400).map(String.init)
        var window = ChatTimelineRenderWindow()
        window.show("163", in: old)
        let range = window.range(in: loaded)
        XCTAssertEqual(range.count, 80)
        XCTAssertTrue(range.contains(162), "A late page must not evict the captured visible row")
    }

    func testInitialAndDeepWindowsStayBounded() {
        let window = ChatTimelineRenderWindow()
        for count in [0, 80, 160, 240, 10_000] {
            let messages = (0..<count).map(String.init)
            let range = window.range(in: messages)
            XCTAssertEqual(range.count, min(count, 80))
            XCTAssertEqual(range.upperBound, count)
        }
    }

    func testPrependRetainsTheVisibleOverlapAndReusesMeasuredSpacers() {
        var window = ChatTimelineRenderWindow()
        let old = ids(161...320)
        window.show(old[0], in: old)
        let all = ids(81...320)
        XCTAssertEqual(window.range(in: all), 40..<120)
        let heights = Dictionary(uniqueKeysWithValues: all.enumerated().map { ($0.element, CGFloat(30 + $0.offset % 3 * 70)) })
        let before = ChatTimelineRenderWindow.spacerHeight(all[..<80], measured: heights)
        window.start(at: 80, in: all)
        let after = window.range(in: all)
        XCTAssertEqual(after, 80..<160)
        XCTAssertEqual(ChatTimelineRenderWindow.spacerHeight(all[..<after.lowerBound], measured: heights), before)
        XCTAssertEqual(all[after.lowerBound], old[0])
    }

    func testDeletingRowsKeepsSurvivingWindowIdentity() {
        var window = ChatTimelineRenderWindow()
        var all = ids(1...400)
        window.start(at: 80, in: all)
        all.removeFirst(12)
        XCTAssertEqual(all[window.range(in: all).lowerBound], "row-81")
        all.removeAll { $0 == "row-81" }
        let range = window.range(in: all)
        XCTAssertLessThanOrEqual(range.count, 160)
        XCTAssertFalse(all[range].contains("row-81"))
        all = ids(1...400)
        XCTAssertTrue(window.preserveVisible(200..<210, including: all[200], in: all))
        let surviving = Set(all[201..<209])
        all.removeFirst(120)
        all.removeAll { $0 == "row-201" || $0 == "row-210" }
        XCTAssertTrue(surviving.isSubset(of: Set(all[window.range(in: all)])),
                      "Deleting both span endpoints must retain every surviving visible row")
    }

    func testSearchLatestAndLiveArrivalsChooseTheIntendedWindow() {
        var window = ChatTimelineRenderWindow()
        var all = ids(1...400)
        window.show("row-40", in: all)
        XCTAssertTrue(window.range(in: all).contains(39))
        let before = window.range(in: all)
        all += ids(401...480)
        XCTAssertEqual(window.range(in: all), before)
        window.showLatest()
        XCTAssertEqual(window.range(in: all), 400..<480)
        window.show("deleted", in: all)
        XCTAssertEqual(window.range(in: all), 400..<480)
    }

    func testFirstPrependRealizesOnlyTheOlderOverscanAndKeepsEveryVisibleRow() {
        let old = ids(81...160)
        let loaded = ids(1...160)
        var window = ChatTimelineRenderWindow()
        XCTAssertTrue(window.preserveVisible(5..<12, including: old[5], in: old))
        let range = window.range(in: loaded)
        XCTAssertEqual(range.count, 80)
        XCTAssertTrue(Set(old[5..<12]).isSubset(of: Set(loaded[range])))
        XCTAssertLessThanOrEqual(loaded[range].filter { !old.contains($0) }.count, 40,
                                 "Do not synchronously realize an entire fetched older page")
        let measured = Dictionary(uniqueKeysWithValues: old.enumerated().map { ($0.element, CGFloat(40 + $0.offset % 3 * 70)) })
        XCTAssertEqual(ChatTimelineRenderWindow.spacerHeight(loaded[range.upperBound...], measured: measured),
                       loaded[range.upperBound...].reduce(0) { $0 + measured[$1]! },
                       "Omitted previously realized rows retain their actual mixed heights")
    }

    func testLargeMeasuredViewportGrowsOnlyAsNeededAndRejectsAnUnprotectableSpan() {
        let all = ids(1...1_000)
        var window = ChatTimelineRenderWindow()
        XCTAssertTrue(window.preserveVisible(300..<370, including: all[300], in: all))
        let range = window.range(in: all)
        XCTAssertEqual(range, 284..<386, "Retain 16 measured rows of overscan on either side")
        XCTAssertTrue(window.preserveVisible(300..<450, including: all[300], in: all))
        XCTAssertEqual(window.range(in: all), 295..<455)
        XCTAssertTrue(window.preserveVisible(300..<450, including: all[300], in: all, startAt: 255))
        XCTAssertEqual(window.range(in: all), 295..<455, "A protected no-op must not swap rows")
        let retained = window
        XCTAssertFalse(window.preserveVisible(300..<461, including: all[300], in: all))
        XCTAssertEqual(window, retained, "The hard cap must never silently discard a protected row")
    }

    func testShiftProtectsTheWholeViewportAndACapturedAnchorOutsideIt() {
        let all = ids(1...400)
        var window = ChatTimelineRenderWindow()
        XCTAssertTrue(window.preserveVisible(100..<110, including: all[95], in: all, startAt: 0))
        let range = window.range(in: all)
        XCTAssertEqual(range, 46..<126)
        XCTAssertTrue((95..<110).allSatisfy(range.contains))
        XCTAssertGreaterThanOrEqual(range.upperBound - 110, 16)
        XCTAssertTrue(window.preserveVisible(100..<110, including: all[114], in: all, startAt: 300))
        XCTAssertEqual(window.range(in: all), 84..<164)
        XCTAssertTrue((100..<115).allSatisfy(window.range(in: all).contains))
    }

    func testRetainingAWindowWithoutGeometryKeepsItsIdentityThroughPrepend() {
        let old = ids(81...160)
        var window = ChatTimelineRenderWindow()
        window.start(at: window.range(in: old).lowerBound, in: old)
        let all = ids(1...160)
        XCTAssertEqual(Array(all[window.range(in: all)]), old)
        window.show("row-30", in: all)
        XCTAssertTrue(window.range(in: all).contains(29))
        window.showLatest()
        XCTAssertEqual(window.range(in: all), 80..<160)
    }

    func testSpacerMeasurementsCanBeReplacedAfterEditsAndSizeChanges() {
        let all = ids(1...4)
        var heights: [String: CGFloat] = [all[0]: 40, all[1]: 210, all[2]: 80, all[3]: 140]
        XCTAssertEqual(ChatTimelineRenderWindow.spacerHeight(all[...], measured: heights), 470)
        heights[all[1]] = 330
        XCTAssertEqual(ChatTimelineRenderWindow.spacerHeight(all[...], measured: heights), 590)
        XCTAssertEqual(ChatTimelineRenderWindow.spacerHeight(all[0..<0], measured: heights), 0)
    }
}
