import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class IrisProfileStoreTests: XCTestCase {
    private let owner = String(repeating: "a", count: 64)

    func testGroupProfileIsResolvedByIdentityAndPhotoRemovalIsAuthoritative() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 1)
        state.currentChat?.participants = [ChatParticipantSnapshot(socialConnection: nil,
            ownerPubkeyHex: owner, displayName: "Ada", pictureUrl: "https://example.invalid/ada.png", isLocalOwner: false)]
        let profiles = IrisProfileStore()
        profiles.apply(state)
        XCTAssertEqual(profiles[owner]?.name, "Ada")
        XCTAssertEqual(profiles[owner]?.pictureUrl, "https://example.invalid/ada.png")
        state.currentChat?.participants[0].pictureUrl = nil
        state.currentChat?.messages[0].authorOwnerPubkeyHex = owner
        state.currentChat?.messages[0].authorPictureUrl = "https://example.invalid/old.png"
        profiles.apply(state)
        XCTAssertNotNil(profiles[owner])
        XCTAssertNil(profiles[owner]?.pictureUrl, "Old message metadata must not restore a removed profile picture")
    }

    func testLogoutAndAccountSwitchDiscardProfilesAndRejectOldSearchResults() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let account = try XCTUnwrap(state.account?.publicKeyHex)
        let results = searchResult()
        let profiles = IrisProfileStore()
        profiles.apply(state)
        profiles.apply(results, account: account)
        XCTAssertNotNil(profiles[owner])
        state.account = nil
        profiles.apply(state)
        XCTAssertTrue(profiles.profiles.isEmpty)
        profiles.apply(results, account: account)
        XCTAssertTrue(profiles.profiles.isEmpty)
        state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0, messagesInCurrentChat: 0)
        state.account?.publicKeyHex = String(repeating: "b", count: 64)
        profiles.apply(state)
        profiles.apply(results, account: account)
        XCTAssertNil(profiles[owner])
    }

    func testDelayedSearchCanFillUnknownPeopleWithoutReplacingNewerProfiles() throws {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let account = try XCTUnwrap(state.account?.publicKeyHex)
        let profiles = IrisProfileStore()
        profiles.apply(state)
        profiles.apply(searchResult(), account: account, replaceExisting: false)
        XCTAssertNotNil(profiles[owner])
        state.currentChat?.participants = [ChatParticipantSnapshot(socialConnection: nil,
            ownerPubkeyHex: owner, displayName: "Current Ada", pictureUrl: nil, isLocalOwner: false)]
        profiles.apply(state)
        profiles.apply(searchResult(), account: account, replaceExisting: false)
        XCTAssertEqual(profiles[owner]?.name, "Current Ada")
        XCTAssertNil(profiles[owner]?.pictureUrl)
    }

    private func searchResult() -> SearchResultSnapshot {
        SearchResultSnapshot(query: "ada", scopeChatId: nil,
            people: [FollowedUserSearchResult(socialConnection: nil, ownerPubkeyHex: owner,
                displayLabel: "Ada", profileLabel: "Ada", pictureUrl: "https://example.invalid/old.png",
                about: nil, userId: owner)], contacts: [], groups: [], messages: [], shortcut: nil)
    }
}
