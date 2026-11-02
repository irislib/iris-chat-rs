import XCTest
#if canImport(IrisShareCore)
@testable import IrisShareCore
#elseif canImport(IrisShareHarness)
@testable import IrisShareHarness
#elseif os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class ShareSuggestionsStoreTests: XCTestCase {
    private func entry(_ id: String = "ada", name: String = "Ada", picture: String? = "photo", time: UInt64 = 1) -> ShareSuggestionEntry {
        ShareSuggestionEntry(chatId: id, displayName: name, subtitle: nil, pictureUrl: picture,
            isGroup: false, lastMessageAtSecs: time)
    }

    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func saved(_ directory: URL, matching: ([ShareSuggestionEntry]) -> Bool) async throws -> [ShareSuggestionEntry] {
        let url = directory.appendingPathComponent("share-suggestions.json")
        for _ in 0..<200 {
            if let data = try? Data(contentsOf: url),
               let entries = try? JSONDecoder().decode([ShareSuggestionEntry].self, from: data), matching(entries) {
                return entries
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("Timed out waiting for the extension's atomic snapshot")
        return []
    }

    func testMessageUpdatesKeepPhotoLoadAndLatestName() async throws {
        let dir = try directory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = ShareSuggestionsStore(container: dir)
        let started = expectation(description: "photo load started")
        let published = expectation(description: "thumbnail published")
        let bytes = Data([1, 2, 3])
        var finish: CheckedContinuation<Data?, Never>?
        var loads = 0
        store.export(entries: [entry()], thumbnailKeys: ["ada": "photo"], loadThumbnail: { _ in
            loads += 1
            started.fulfill()
            return await withCheckedContinuation { finish = $0 }
        }, onAvatars: { _ in })
        await fulfillment(of: [started], timeout: 2)
        store.export(entries: [entry(name: "Ada Lovelace", time: 42)], thumbnailKeys: ["ada": "photo"],
            loadThumbnail: { _ in XCTFail("Must retain the running download"); return nil },
            onAvatars: { if $0["ada"] == bytes { published.fulfill() } })
        finish?.resume(returning: bytes)
        await fulfillment(of: [published], timeout: 2)
        let entries = try await saved(dir) { $0.first?.avatarData == bytes }
        XCTAssertEqual(loads, 1)
        XCTAssertEqual(entries.first?.displayName, "Ada Lovelace")
        XCTAssertEqual(entries.first?.lastMessageAtSecs, 42)
    }

    func testRemovedPhotoCannotReturnFromAnOldDownload() async throws {
        let dir = try directory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = ShareSuggestionsStore(container: dir)
        let started = expectation(description: "old photo started")
        var finish: CheckedContinuation<Data?, Never>?
        store.export(entries: [entry()], thumbnailKeys: ["ada": "old-photo"], loadThumbnail: { _ in
            started.fulfill()
            return await withCheckedContinuation { finish = $0 }
        }, onAvatars: { _ in })
        await fulfillment(of: [started], timeout: 2)
        store.export(entries: [entry(picture: nil)], thumbnailKeys: [:], loadThumbnail: { _ in nil },
            onAvatars: { XCTAssertTrue($0.isEmpty) })
        finish?.resume(returning: Data([9]))
        let entries = try await saved(dir) { $0.first?.pictureUrl == nil }
        XCTAssertNil(entries.first?.avatarData)
        await Task.yield()
    }

    func testCachedPhotosSurviveRestartWithoutFetching() async throws {
        let dir = try directory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let bytes = Data([4, 5, 6])
        let first = ShareSuggestionsStore(container: dir)
        first.export(entries: [entry()], thumbnailKeys: ["ada": "photo"], cachedThumbnails: ["photo": bytes],
            loadThumbnail: { _ in XCTFail("Use the app's existing thumbnail"); return nil }, onAvatars: { _ in })
        _ = try await saved(dir) { $0.first?.avatarData == bytes }
        let restored = ShareSuggestionsStore(container: dir)
        restored.export(entries: [entry(name: "Ada restored")], thumbnailKeys: ["ada": "photo"],
            loadThumbnail: { _ in XCTFail("Use the persisted photo"); return nil },
            onAvatars: { XCTAssertEqual($0["ada"], bytes) })
        let entries = try await saved(dir) { $0.first?.displayName == "Ada restored" }
        XCTAssertEqual(entries.first?.avatarData, bytes)
    }

    func testMostRecentRecipientsAreFirstAndSnapshotIsBounded() async throws {
        let dir = try directory()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = ShareSuggestionsStore(container: dir)
        let entries = (0..<30).map { entry("person-\($0)", picture: nil, time: UInt64($0)) }
        store.export(entries: entries, thumbnailKeys: [:], loadThumbnail: { _ in nil }, onAvatars: { _ in })
        let snapshot = try await saved(dir) { $0.count == 20 }
        XCTAssertEqual(snapshot.map(\.chatId), (10..<30).reversed().map { "person-\($0)" })
    }

    func testOlderExtensionSnapshotIsReadable() async throws {
        let bytes = Data(#"[{"chatId":"ada","displayName":"Ada","isGroup":false}]"#.utf8)
        let entries = try JSONDecoder().decode([ShareSuggestionEntry].self, from: bytes)
        XCTAssertEqual(entries.first?.displayName, "Ada")
        XCTAssertNil(entries.first?.avatarData)
    }
}
