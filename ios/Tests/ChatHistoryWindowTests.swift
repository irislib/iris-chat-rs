import XCTest
#if IRIS_HISTORY_HARNESS
@testable import IrisHistoryHarness
#elseif os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ChatHistoryWindowTests: XCTestCase {
    private func messages(_ range: ClosedRange<Int>) -> [ChatMessageSnapshot] {
        range.map { makeMessage(chatId: "history", id: String($0)) }
    }

    func testWarmResidentHistoryStartsWithLatestPageAndOnlyExplicitPagesExtendIt() {
        let raw = messages(1...240)
        var history = ChatHistoryWindow(recent: raw)
        var visible = history.replaceRecent(raw, in: raw)
        XCTAssertEqual(visible.map(\.id), messages(161...240).map(\.id))
        visible = history.addPage(messages(81...160), to: visible)
        XCTAssertEqual(visible.map(\.id), messages(81...240).map(\.id))
        visible = history.replaceRecent(raw, in: visible)
        XCTAssertEqual(visible.map(\.id), messages(81...240).map(\.id))
        visible = history.addPage(messages(1...80), to: visible)
        XCTAssertEqual(visible, raw)
        XCTAssertEqual(history.replaceRecent(raw, in: visible), raw)
    }

    func testEmptyOpeningSnapshotDoesNotLetFirstLoadedRawHistoryBypassTheBound() {
        var history = ChatHistoryWindow()
        var visible = history.replaceRecent([], in: [])
        visible = history.replaceRecent(messages(1...240), in: visible)
        XCTAssertEqual(visible.map(\.id), messages(161...240).map(\.id))
    }

    func testInitialLatestPageFillsPartialRawWindowWithoutReplacingFreshOverlap() {
        var history = ChatHistoryWindow()
        var last = messages(240...240)
        last[0].body = "fresh"
        var visible = history.replaceRecent(last, in: [])
        visible = history.replaceLatestPage(messages(161...240), in: visible)
        XCTAssertEqual(visible.count, 80)
        XCTAssertEqual(visible.last?.body, "fresh")
        var raw = messages(1...240)
        raw[239] = last[0]
        XCTAssertEqual(history.replaceRecent(raw, in: visible), visible)
    }

    func testRawUpdatesRefreshPagedRowsAndKnownDeletionsRejectLatePages() {
        var raw = messages(1...240)
        var history = ChatHistoryWindow(recent: messages(161...240))
        var visible = history.replaceRecent(raw, in: messages(161...240))
        visible = history.addPage(messages(81...160), to: visible)
        let stalePage = messages(1...160)
        raw[99].body = "edited"
        raw[99].reactions = [MessageReactionSnapshot(emoji: "❤️", count: 2, reactedByMe: true)]
        raw.removeAll { ["60", "120", "161", "240"].contains($0.id) }
        visible = history.replaceRecent(raw, in: visible)
        XCTAssertEqual(visible.count, 157)
        XCTAssertEqual(visible.first { $0.id == "100" }, raw.first { $0.id == "100" })
        visible = history.addPage(stalePage, to: visible)
        XCTAssertEqual(visible.count, 236)
        XCTAssertFalse(visible.contains { ["60", "120", "161", "240"].contains($0.id) })
        XCTAssertEqual(visible.first { $0.id == "100" }, raw.first { $0.id == "100" })
    }

    func testLiveTailGrowsWithoutImportingNewOlderRawPrefix() {
        var history = ChatHistoryWindow(recent: messages(161...240))
        var visible = history.replaceRecent(messages(161...240), in: messages(161...240))
        visible = history.replaceRecent(messages(1...245), in: visible)
        XCTAssertEqual(visible.map(\.id), messages(161...245).map(\.id))
        let unchanged = history.replaceRecent(messages(1...245), in: visible)
        visible.withUnsafeBufferPointer { original in
            unchanged.withUnsafeBufferPointer { next in XCTAssertEqual(original.baseAddress, next.baseAddress) }
        }
    }

    func testDeletingAllRecentDoesNotRevealHiddenPrefixButRetainsDatabaseOnlyHistory() {
        var history = ChatHistoryWindow(recent: messages(161...240))
        var visible = history.replaceRecent(messages(81...240), in: messages(161...240))
        visible = history.addPage(messages(1...80), to: visible)
        visible = history.replaceRecent(messages(81...160), in: visible)
        XCTAssertEqual(visible.map(\.id), messages(1...80).map(\.id))
        visible = history.replaceRecent([], in: visible)
        XCTAssertEqual(visible.map(\.id), messages(1...80).map(\.id))
        XCTAssertEqual(history.addPage(messages(81...240), to: visible), visible)
        visible = history.replaceRecent(messages(241...242), in: visible)
        XCTAssertEqual(visible.map(\.id), messages(1...80).map(\.id) + ["241", "242"])
    }

    func testNewScopeBoundsAgainWithoutRetainingPreviousScopeTombstones() {
        let raw = messages(1...240)
        var history = ChatHistoryWindow(recent: raw)
        var visible = history.replaceRecent(raw, in: raw)
        visible = history.addPage(messages(1...160), to: visible)
        visible = history.replaceRecent(messages(2...240), in: visible)
        history = ChatHistoryWindow(recent: visible)
        visible = history.replaceRecent(raw, in: visible)
        XCTAssertEqual(visible.map(\.id), messages(161...240).map(\.id))
        XCTAssertEqual(history.addPage(messages(1...160), to: visible), raw)
    }

    func testLateLatestPageFillsUnloadedHistoryButPreservesRawEditsDeletesAndNewArrivals() {
        var history = ChatHistoryWindow()
        var visible = history.replaceRecent([], in: [])
        visible = history.replaceLatestPage(messages(161...240), in: visible)
        XCTAssertEqual(visible, messages(161...240))
        var raw = messages(1...241)
        raw.removeAll { $0.id == "161" }
        raw[238].body = "fresh edit"
        visible = history.replaceRecent(raw, in: visible)
        let fresh = visible
        visible = history.replaceLatestPage(messages(161...240), in: visible)
        XCTAssertEqual(visible, fresh)
        XCTAssertEqual(visible.last?.id, "241")
        XCTAssertFalse(visible.contains { $0.id == "161" })
        XCTAssertEqual(visible.first { $0.id == "240" }?.body, "fresh edit")
    }

    func testSameSecondRowsKeepPageOrderAndExpiredRowsStayHidden() {
        var raw = messages(1...240)
        for index in raw.indices { raw[index].createdAtSecs = 10 }
        raw[89].expiresAtSecs = 20
        var history = ChatHistoryWindow(recent: raw)
        var visible = history.replaceRecent(raw, in: raw, now: 19)
        XCTAssertEqual(visible.map(\.id), messages(161...240).map(\.id))
        visible = history.addPage(Array(raw[80..<160]), to: visible, now: 19)
        XCTAssertEqual(visible.map(\.id), messages(81...240).map(\.id))
        visible = history.replaceRecent(raw, in: visible, now: 20)
        XCTAssertEqual(visible.count, 159)
        XCTAssertFalse(visible.contains { $0.id == "90" })
        XCTAssertEqual(history.addPage(Array(raw[80..<160]), to: visible, now: 20), visible)
        var arrival = makeMessage(chatId: "history", id: "241", createdAtSecs: 10)
        arrival.body = "new same-second arrival"
        raw.append(arrival)
        visible = history.replaceRecent(raw, in: visible, now: 20)
        XCTAssertEqual(visible.count, 160)
        XCTAssertEqual(visible.last, arrival)
    }
}
