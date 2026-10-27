import XCTest
@testable import IrisChat

final class ShareSuggestionAvatarTests: XCTestCase {
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
