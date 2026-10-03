import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class AppManagerHistoryTests: XCTestCase {
    private let chatID = "chat-1"
    private var directory: URL!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: directory) }

    private func snapshot(_ ids: [String], rev: UInt64 = 1) -> AppState {
        makeLargeFixtureState(
            rev: rev, router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: chatID)]),
            account: makeAccount(),
            currentChat: makeCurrentChat(chatId: chatID, messages: ids.map { makeMessage(chatId: chatID, id: $0) })
        )
    }

    private func manager(_ rust: MockRustApp, replacement: MockRustApp? = nil) -> AppManager {
        AppManager(rust: rust, secretStore: InMemorySecretStore(),
                   pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory,
                   environment: ["IRIS_UI_TEST_RUN_ID": "history-unit"],
                   rustFactory: replacement.map { next in { next } })
    }

    private func page(_ ids: [String]) -> CurrentChatSnapshot {
        makeCurrentChat(chatId: chatID, messages: ids.map { makeMessage(chatId: chatID, id: $0) })
    }

    private func apply(_ state: AppState, via rust: MockRustApp, to manager: AppManager) async {
        rust.currentState = state
        rust.emit(.fullState(state))
        let applied = await waitUntil { manager.state.rev == state.rev }
        XCTAssertTrue(applied)
    }

    func testWarmInitialRawHistoryStaysBoundedAndAccountSwitchStartsAnotherWindow() async {
        let rust = MockRustApp(state: snapshot((1...240).map(String.init)))
        rust.pagesBefore["\(chatID)|161"] = page((81...160).map(String.init))
        let app = manager(rust)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), (161...240).map(String.init))
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID))
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "81" }
        XCTAssertTrue(loaded)
        await apply(snapshot((1...240).filter { $0 != 100 }.map(String.init), rev: 2), via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.count, 159)
        XCTAssertFalse(app.state.currentChat?.messages.contains { $0.id == "100" } ?? true)
        var next = snapshot((1001...1240).map(String.init), rev: 3)
        next.account?.publicKeyHex = "another-synthetic-account"
        await apply(next, via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), (1161...1240).map(String.init))
    }

    func testFullStateKeepsLoadedSearchHitContextForVisibleChat() async {
        let rust = MockRustApp(state: snapshot((121...200).map(String.init)))
        rust.pagesAround["\(chatID)|25"] = page((15...35).map(String.init))
        let app = manager(rust)
        app.loadChatAroundMessage(chatId: chatID, messageId: "25")
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "15" }
        XCTAssertTrue(loaded)
        await apply(snapshot((121...201).map(String.init), rev: 2), via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id),
                       (15...35).map(String.init) + (121...201).map(String.init))
    }

    func testFullStateClearsSentDraftWhileKeepingLoadedHistory() async {
        var state = snapshot(["2"])
        state.currentChat?.draft = "sent message"
        let rust = MockRustApp(state: state)
        rust.pagesBefore["\(chatID)|2"] = page(["1"])
        let app = manager(rust)
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID))
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "1" }
        XCTAssertTrue(loaded)
        await apply(snapshot(["2", "3"], rev: 2), via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["1", "2", "3"])
        XCTAssertEqual(app.state.currentChat?.draft, "")
    }

    func testFullStatePreservesPageOrderForSameSecondVisibleMessages() async {
        var state = snapshot([])
        let recent = ["z-first", "a-second", "m-last"].map {
            makeMessage(chatId: chatID, id: $0, createdAtSecs: 10)
        }
        state.currentChat?.messages = Array(recent.prefix(2))
        let rust = MockRustApp(state: state)
        rust.pagesBefore["\(chatID)|z-first"] = page(["9"])
        let app = manager(rust)
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID))
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "9" }
        XCTAssertTrue(loaded)
        state.rev = 2
        state.currentChat?.messages = recent
        await apply(state, via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["9", "z-first", "a-second", "m-last"])
    }

    func testAuthoritativeSnapshotsRemoveFirstLastAndAllRecentRows() async {
        let rust = MockRustApp(state: snapshot(["10", "11", "12"]))
        rust.pagesBefore["\(chatID)|10"] = page(["9"])
        let app = manager(rust)
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID))
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "9" }
        XCTAssertTrue(loaded)
        await apply(snapshot(["11", "12"], rev: 2), via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["9", "11", "12"])
        await apply(snapshot(["11"], rev: 3), via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["9", "11"])
        await apply(snapshot([], rev: 4), via: rust, to: app)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["9"])
    }

    func testLateOlderPageKeepsFreshReactionAndCannotResurrectRemovedRecentRow() async {
        let rust = MockRustApp(state: snapshot(["10", "11"]))
        rust.pagesBefore["\(chatID)|10"] = page(["9", "10", "11"])
        let read = expectation(description: "read captured old rows")
        let gate = DispatchSemaphore(value: 0)
        rust.onChatPageRead = { read.fulfill(); gate.wait() }
        defer { gate.signal() }
        let app = manager(rust)
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID))
        await fulfillment(of: [read], timeout: 5)
        var fresh = snapshot(["11"], rev: 2)
        fresh.currentChat?.messages[0].body = "edited while page read was pending"
        fresh.currentChat?.messages[0].reactions = [MessageReactionSnapshot(emoji: "❤️", count: 1, reactedByMe: true)]
        await apply(fresh, via: rust, to: app)
        gate.signal()
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "9" }
        XCTAssertTrue(loaded)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["9", "11"])
        XCTAssertEqual(app.state.currentChat?.messages.last, fresh.currentChat?.messages.last)
    }

    func testHistoryWindowDropsExpiredOlderRowsAndPagesPreserveEqualSecondOrder() {
        let recent = makeMessage(chatId: chatID, id: "recent", createdAtSecs: 10)
        var older = makeMessage(chatId: chatID, id: "older", createdAtSecs: 10)
        older.expiresAtSecs = 20
        var history = ChatHistoryWindow(recent: [recent])
        let loaded = history.addPage([older, recent], to: [recent], now: 19)
        XCTAssertEqual(loaded.map(\.id), ["older", "recent"])
        XCTAssertEqual(history.replaceRecent([recent], in: loaded, now: 20), [recent])
        XCTAssertEqual(history.addPage([older], to: [recent], now: 20), [recent])
    }

    func testUnchangedRecentWindowReusesStorageAndOrderUntilOlderHistoryExpires() {
        let recent = [makeMessage(chatId: chatID, id: "recent", createdAtSecs: 10)]
        var older = ["z-first", "a-second"].map { makeMessage(chatId: chatID, id: $0, createdAtSecs: 1) }
        older[1].expiresAtSecs = 20
        var history = ChatHistoryWindow(recent: recent)
        let displayed = history.addPage(older, to: recent, now: 19)
        let unchanged = history.replaceRecent(recent, in: displayed, now: 19)
        XCTAssertEqual(unchanged.map(\.id), ["z-first", "a-second", "recent"])
        displayed.withUnsafeBufferPointer { original in
            unchanged.withUnsafeBufferPointer { next in
                XCTAssertEqual(original.baseAddress, next.baseAddress, "Unchanged updates retain loaded array storage")
            }
        }
        XCTAssertEqual(history.replaceRecent(recent, in: unchanged, now: 20).map(\.id), ["z-first", "recent"])
    }

    func testLateOlderPageAfterLeavingAndReopeningCannotClearNewFlight() async {
        let recentIDs = (10...89).map(String.init)
        let rust = MockRustApp(state: snapshot(recentIDs))
        rust.pagesBefore["\(chatID)|10"] = page(["9"])
        let barrier = PageReadBarrier(test: self)
        rust.onChatPageRead = { barrier.arrive() }
        defer { barrier.releaseAll() }
        let app = manager(rust)
        await apply(snapshot(recentIDs, rev: 2), via: rust, to: app)
        var oldCompleted = false
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID) { _ in oldCompleted = true })
        await fulfillment(of: [barrier.first], timeout: 5)
        app.navigateBack()
        app.dispatch(.openChat(chatId: chatID))
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID))
        barrier.releaseFirst.signal()
        await fulfillment(of: [barrier.second], timeout: 5)
        let oldFinished = await waitUntil { oldCompleted }
        XCTAssertTrue(oldFinished)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), recentIDs)
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID), "Duplicate call uses the new in-flight request")
        barrier.releaseSecond.signal()
        let loaded = await waitUntil { app.state.currentChat?.messages.first?.id == "9" }
        XCTAssertTrue(loaded)
        XCTAssertEqual(barrier.count, 2, "Old completion cannot clear the new request's flight marker")
    }

    func testLatePageAfterAccountSwitchIsIgnoredAndHistoryLoadingRequiresChatRoute() async {
        let rust = MockRustApp(state: snapshot(["10"]))
        rust.pagesBefore["\(chatID)|10"] = page(["9"])
        let read = expectation(description: "old account page captured")
        let gate = DispatchSemaphore(value: 0)
        rust.onChatPageRead = { read.fulfill(); gate.wait() }
        defer { gate.signal() }
        let app = manager(rust)
        var finished = false
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID) { _ in finished = true })
        await fulfillment(of: [read], timeout: 5)
        var next = snapshot(["20"], rev: 2)
        next.account?.publicKeyHex = "another-synthetic-account"
        await apply(next, via: rust, to: app)
        gate.signal()
        let completed = await waitUntil { finished }
        XCTAssertTrue(completed)
        app.dispatch(.pushScreen(screen: .directChatInfo(chatId: chatID)))
        XCTAssertFalse(app.loadOlderMessages(chatId: chatID))
        app.loadChatAroundMessage(chatId: chatID, messageId: "9")
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["20"])
    }

    func testLatePageAfterCoreResetCannotEnterReplacementAccount() async {
        let rust = MockRustApp(state: snapshot(["10"]))
        let replacement = MockRustApp(state: snapshot(["20"]))
        rust.pagesBefore["\(chatID)|10"] = page(["9"])
        let read = expectation(description: "previous core page captured")
        let gate = DispatchSemaphore(value: 0)
        rust.onChatPageRead = { read.fulfill(); gate.wait() }
        defer { gate.signal() }
        let app = manager(rust, replacement: replacement)
        var finished = false
        XCTAssertTrue(app.loadOlderMessages(chatId: chatID) { _ in finished = true })
        await fulfillment(of: [read], timeout: 5)
        app.logout()
        let replaced = await waitUntil { app.state.currentChat?.messages.first?.id == "20" }
        XCTAssertTrue(replaced)
        gate.signal()
        let completed = await waitUntil { finished }
        XCTAssertTrue(completed)
        XCTAssertEqual(app.state.currentChat?.messages.map(\.id), ["20"])
    }
}

private final class PageReadBarrier: @unchecked Sendable {
    let first: XCTestExpectation
    let second: XCTestExpectation
    let releaseFirst = DispatchSemaphore(value: 0)
    let releaseSecond = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var reads = 0
    var count: Int { lock.lock(); defer { lock.unlock() }; return reads }
    init(test: XCTestCase) {
        first = test.expectation(description: "first read")
        second = test.expectation(description: "second read")
    }
    func arrive() {
        lock.lock(); reads += 1; let current = reads; lock.unlock()
        if current == 1 { first.fulfill(); releaseFirst.wait() }
        if current == 2 { second.fulfill(); releaseSecond.wait() }
    }
    func releaseAll() { releaseFirst.signal(); releaseSecond.signal() }
}
