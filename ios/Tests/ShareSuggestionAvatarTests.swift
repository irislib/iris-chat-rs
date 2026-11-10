import XCTest
@testable import IrisChat

final class ShareSuggestionAvatarTests: XCTestCase {
    @MainActor
    func testFirstNoteSurvivesTheExtensionLimitAndRoutesToTheSignedInAccount() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let state = buildLargeTestAppState(directChatCount: 24, groupChatCount: 0, messagesInCurrentChat: 0)
        let account = try XCTUnwrap(state.account)
        let chats = shareTargetChats(account: account, chats: state.chatList)
        let entries = resolvedShareSuggestionChats(chats, ownChatId: account.publicKeyHex) { _ in nil }
        let exporter = ShareSuggestionsExporter(appGroupIdentifier: "test", containerOverride: directory) { _, _ in nil }
        exporter.export(chats: entries, manager: nil) { _ in }
        let url = directory.appendingPathComponent("share-suggestions.json")
        let published = await waitUntil { FileManager.default.fileExists(atPath: url.path) }
        XCTAssertTrue(published)
        let saved = try JSONDecoder().decode([ShareSuggestionEntry].self, from: Data(contentsOf: url))
        XCTAssertEqual(saved.count, 20)
        let own = try XCTUnwrap(saved.first)
        XCTAssertEqual(own.displayName, "Note to self")
        XCTAssertEqual(own.subtitle, account.displayName)
        XCTAssertEqual(own.chatId, PendingShare.noteToSelfTarget)
        XCTAssertFalse(saved.contains { $0.chatId == account.publicKeyHex })
        let share = PendingShare(id: "note", text: "hello", attachments: [], suggestedChatId: own.chatId,
                                 suggestedChatIds: [own.chatId], autoSend: true, isForward: nil)
        XCTAssertEqual(share.suggestedTargetChatIds(for: account), [account.publicKeyHex])
        XCTAssertEqual(share.suggestedTargetChatIds(for: nil), [])
        var nextAccount = account
        nextAccount.publicKeyHex = "next-account"
        XCTAssertEqual(share.suggestedTargetChatIds(for: nextAccount), [nextAccount.publicKeyHex])
    }

    func testDirectShareUsesCurrentProfileAndRefreshesWithoutANewMessage() {
        let chat = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0).chatList[0]
        var gate = IosStateSideEffectGate()
        let first = resolvedShareSuggestionChats([chat]) { _ in
            IrisUserProfile(name: "Ada", isFallback: false, pictureUrl: "htree://first", socialConnection: nil)
        }
        XCTAssertEqual(first[0].displayName, "Ada")
        XCTAssertEqual(first[0].pictureUrl, "htree://first")
        XCTAssertTrue(gate.shouldSyncShareSuggestions(chatList: first))
        let changed = resolvedShareSuggestionChats([chat]) { _ in
            IrisUserProfile(name: "Ada Lovelace", isFallback: false, pictureUrl: "htree://second", socialConnection: nil)
        }
        XCTAssertTrue(gate.shouldSyncShareSuggestions(chatList: changed))
        let removed = resolvedShareSuggestionChats([chat]) { _ in
            IrisUserProfile(name: "Ada Lovelace", isFallback: false, pictureUrl: nil, socialConnection: nil)
        }
        XCTAssertNil(removed[0].pictureUrl, "Do not resurrect a removed photo from an older chat snapshot")
    }

    func testGroupPhotosStayOnGroupAndRequestsAreNotSuggested() {
        var chats = buildLargeTestAppState(directChatCount: 1, groupChatCount: 1, messagesInCurrentChat: 0).chatList
        let groupIndex = chats.firstIndex { $0.kind == .group }!
        let directIndex = chats.firstIndex { $0.kind == .direct }!
        chats[groupIndex].pictureUrl = "htree://group-photo"
        chats[directIndex].isRequest = true
        let resolved = resolvedShareSuggestionChats(chats) { _ in
            XCTFail("Groups and requests must not resolve a person's profile")
            return nil
        }
        XCTAssertEqual(resolved.count, 1)
        XCTAssertEqual(resolved.first?.pictureUrl, "htree://group-photo")
    }

    @MainActor
    func testChatUpdatesDoNotCancelPhotoExportOrRestoreOldChatNames() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let started = expectation(description: "photo started")
        let published = expectation(description: "photo published")
        var finish: CheckedContinuation<Data?, Never>?
        var loads = 0
        let bytes = Data([1, 2, 3])
        let exporter = ShareSuggestionsExporter(appGroupIdentifier: "share-test", containerOverride: directory) { _, _ in
            loads += 1
            started.fulfill()
            return await withCheckedContinuation { finish = $0 }
        }
        var chat = state.chatList[0]
        chat.pictureUrl = "htree://export-test"
        exporter.export(chats: [chat], manager: nil) { _ in }
        await fulfillment(of: [started], timeout: 2)
        chat.displayName = "Updated name"
        chat.lastMessageAtSecs = 123
        exporter.export(chats: [chat], manager: nil) { avatars in
            if avatars[chat.chatId] == bytes { published.fulfill() }
        }
        finish?.resume(returning: bytes)
        await fulfillment(of: [published], timeout: 2)
        XCTAssertEqual(loads, 1)
        // Observe the asynchronous, atomic disk handoff used by the extension.
        let url = directory.appendingPathComponent("share-suggestions.json")
        var entries: [ShareSuggestionEntry] = []
        for _ in 0..<100 {
            if let data = try? Data(contentsOf: url),
               let saved = try? JSONDecoder().decode([ShareSuggestionEntry].self, from: data),
               saved.first?.avatarData == bytes { entries = saved; break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertEqual(entries.first?.displayName, "Updated name")
        XCTAssertEqual(entries.first?.lastMessageAtSecs, 123)
        XCTAssertEqual(entries.first?.avatarData, bytes)
    }

    func testOlderSharedSnapshotsRemainReadable() throws {
        let data = Data(#"[{"chatId":"alice","displayName":"Alice","isGroup":false}]"#.utf8)
        let entries = try JSONDecoder().decode([ShareSuggestionEntry].self, from: data)
        XCTAssertEqual(entries.first?.displayName, "Alice")
        XCTAssertNil(entries.first?.avatarData)
    }

    func testThumbnailBytesSurviveAppExtensionHandoff() throws {
        let bytes = Data([0, 1, 127, 255])
        let entries = [
            ShareSuggestionEntry(chatId: "alice", displayName: "Alice", subtitle: nil,
                                 pictureUrl: "htree://picture", isGroup: false,
                                 lastMessageAtSecs: 1, avatarData: bytes),
            ShareSuggestionEntry(chatId: "group", displayName: "Friends", subtitle: nil,
                                 pictureUrl: nil, isGroup: true, lastMessageAtSecs: nil)
        ]
        let decoded = try JSONDecoder().decode([ShareSuggestionEntry].self, from: JSONEncoder().encode(entries))
        XCTAssertEqual(decoded[0].avatarData, bytes)
        XCTAssertEqual(decoded[0].pictureUrl, entries[0].pictureUrl)
        XCTAssertNil(decoded[1].avatarData)
        XCTAssertTrue(decoded[1].isGroup)
    }
}
