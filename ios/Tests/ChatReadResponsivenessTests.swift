import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ChatReadResponsivenessTests: XCTestCase {
    @MainActor
    func testReopeningPagedChatUsesLatestPageWithoutTrimmingActiveHistory() async throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 1_200)
        state.rev = 1
        let history = try XCTUnwrap(state.currentChat)
        state.router.screenStack = [.chat(chatId: history.chatId)]
        let rust = MockRustApp(state: state)
        var latest = history
        latest.messages = Array(history.messages.suffix(80))
        rust.chatSnapshotOverride = latest
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])

        // Applying a snapshot refreshes the navigation cache, but must leave
        // the currently browsed history and its scroll anchors available.
        state.rev = 2
        rust.emit(.fullState(state))
        let updated = await waitUntil { manager.state.rev == 2 }
        XCTAssertTrue(updated)
        XCTAssertEqual(manager.state.currentChat?.messages.count, 1_200)

        manager.navigateBack()
        XCTAssertNil(manager.state.currentChat)
        manager.dispatch(.openChat(chatId: history.chatId))

        // Check the immediate cached first paint before the async page read
        // can return; reopening must not eagerly lay out all browsed pages.
        XCTAssertEqual(manager.state.currentChat?.messages.map(\.id), latest.messages.map(\.id))
        XCTAssertEqual(manager.state.currentChat?.messages.count, 80)
    }

    @MainActor
    func testMetadataUpdatesReuseLoadedHistoryAndMessageEditsStillMerge() async throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 1_200)
        state.rev = 1
        let history = try XCTUnwrap(state.currentChat)
        state.router.screenStack = [.chat(chatId: history.chatId)]
        let rust = MockRustApp(state: state)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        var latest = history
        latest.messages = Array(history.messages.suffix(80))
        latest.draft = "still typing"
        latest.displayName = "Updated name"
        state.currentChat = latest
        state.rev = 2
        rust.emit(.fullState(state))
        let updated = await waitUntil { manager.state.rev == 2 }
        XCTAssertTrue(updated)
        let afterTyping = try XCTUnwrap(manager.state.currentChat)

        XCTAssertEqual(afterTyping.messages, history.messages)
        XCTAssertEqual(afterTyping.draft, "still typing")
        XCTAssertEqual(afterTyping.displayName, "Updated name")
        // Keeping the original array alive makes this a deterministic
        // allocation regression: a metadata update must not rebuild and sort
        // every retained message just to produce the same history again.
        let reusedHistory = history.messages.withUnsafeBufferPointer { original in
            afterTyping.messages.withUnsafeBufferPointer { updated in
                original.baseAddress == updated.baseAddress
            }
        }
        XCTAssertTrue(reusedHistory)

        latest.messages[latest.messages.count - 1].body = "Edited message"
        latest.draft = ""
        state.currentChat = latest
        state.rev = 3
        rust.emit(.fullState(state))
        let editApplied = await waitUntil { manager.state.rev == 3 }
        XCTAssertTrue(editApplied)
        XCTAssertEqual(manager.state.currentChat?.messages.count, history.messages.count)
        XCTAssertEqual(manager.state.currentChat?.messages.map(\.id), history.messages.map(\.id))
        XCTAssertEqual(manager.state.currentChat?.messages.last?.body, "Edited message")
        XCTAssertEqual(manager.state.currentChat?.draft, "")
    }

    @MainActor
    func testBackToChatListDoesNotWaitForBusyNavigationDispatch() async {
        var state = buildLargeTestAppState(directChatCount: 100, groupChatCount: 0, messagesInCurrentChat: 80)
        state.router.screenStack = []
        let rust = MockRustApp(state: state)
        let opening = expectation(description: "chat dispatch started")
        let returned = expectation(description: "back dispatch completed")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onDispatch = { action in
            switch action {
            case .openChat:
                XCTAssertFalse(Thread.isMainThread)
                opening.fulfill()
                XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            case .updateScreenStack(let stack) where stack.isEmpty:
                XCTAssertFalse(Thread.isMainThread)
                returned.fulfill()
            default:
                break
            }
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        manager.dispatch(.openChat(chatId: state.chatList[0].chatId))
        await fulfillment(of: [opening], timeout: 2)

        manager.navigateBack()
        XCTAssertEqual(manager.activeScreen, .chatList)
        XCTAssertNil(manager.state.currentChat)
        await Task.yield()
        XCTAssertEqual(manager.activeScreen, .chatList)

        gate.signal()
        await fulfillment(of: [returned], timeout: 2)
    }

    @MainActor
    func testCreateChatRoutesImmediatelyAndDispatchesOffMainThread() async {
        let peer = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        state.router.screenStack = []
        let rust = MockRustApp(state: state)
        let dispatched = expectation(description: "create chat dispatched")
        rust.onDispatch = { action in
            guard case .createChat = action else { return }
            XCTAssertFalse(Thread.isMainThread)
            dispatched.fulfill()
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])

        manager.dispatch(.createChat(peerInput: peer))

        XCTAssertEqual(manager.activeScreen, .chat(chatId: peer))
        await fulfillment(of: [dispatched], timeout: 3)
    }

    @MainActor
    func testBlockedSearchDoesNotBlockChatNavigation() async {
        let state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let rust = MockRustApp(state: state)
        let started = expectation(description: "search started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        rust.onSearch = {
            XCTAssertFalse(Thread.isMainThread)
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        let search = Task { await manager.search("friend") }
        await fulfillment(of: [started], timeout: 2)

        let chatId = state.chatList[0].chatId
        manager.dispatch(.openChat(chatId: chatId))
        XCTAssertEqual(manager.activeScreen, .chat(chatId: chatId))
        gate.signal()
        _ = await search.value
    }

    @MainActor
    func testLateChatPageCannotRestoreCheckingAfterCoreUnlockedComposer() async {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 1)
        let chatId = state.currentChat!.chatId
        var stalePage = state.currentChat!
        stalePage.directChatCapability = .checking
        state.router.screenStack = []
        state.currentChat = nil
        let rust = MockRustApp(state: state)
        rust.chatSnapshotOverride = stalePage
        let gate = DispatchSemaphore(value: 0)
        rust.chatSnapshotGate = gate
        defer { gate.signal() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), desktopNotifications: NoopDesktopNotificationPoster(), dataDir: directory, environment: [:])
        manager.dispatch(.openChat(chatId: chatId))
        var fresh = state
        fresh.rev += 1
        fresh.router.screenStack = [.chat(chatId: chatId)]
        fresh.currentChat = stalePage
        fresh.currentChat?.messages = []
        fresh.currentChat?.directChatCapability = .available
        rust.emit(.fullState(fresh))
        for _ in 0..<100 where manager.state.currentChat?.directChatCapability != .available {
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertEqual(manager.state.currentChat?.directChatCapability, .available)

        gate.signal()
        for _ in 0..<100 where manager.state.currentChat?.messages.isEmpty != false {
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertFalse(manager.state.currentChat?.messages.isEmpty ?? true)
        XCTAssertEqual(manager.state.currentChat?.directChatCapability, .available)
    }
}
