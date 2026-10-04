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
        XCTAssertEqual(range.count, 160)
        XCTAssertTrue(range.contains(162), "A late page must not evict the captured visible row")
    }

    func testInitialAndDeepWindowsStayBounded() {
        let window = ChatTimelineRenderWindow()
        for count in [0, 80, 160, 240, 10_000] {
            let messages = (0..<count).map(String.init)
            let range = window.range(in: messages)
            XCTAssertEqual(range.count, min(count, 160))
            XCTAssertEqual(range.upperBound, count)
        }
    }

    func testPrependRetainsTheVisibleOverlapAndReusesMeasuredSpacers() {
        var window = ChatTimelineRenderWindow()
        let old = ids(161...320)
        window.show(old[0], in: old)
        let all = ids(81...320)
        XCTAssertEqual(window.range(in: all), 0..<160)
        let heights = Dictionary(uniqueKeysWithValues: all.enumerated().map { ($0.element, CGFloat(30 + $0.offset % 3 * 70)) })
        let before = ChatTimelineRenderWindow.spacerHeight(all[..<80], measured: heights)
        window.start(at: 80, in: all)
        let after = window.range(in: all)
        XCTAssertEqual(after, 80..<240)
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
        XCTAssertEqual(window.range(in: all), 320..<480)
        window.show("deleted", in: all)
        XCTAssertEqual(window.range(in: all), 320..<480)
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
