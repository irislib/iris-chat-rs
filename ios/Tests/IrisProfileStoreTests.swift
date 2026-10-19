import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class IrisProfileStoreTests: XCTestCase {
    private let owner = String(repeating: "a", count: 64)

    func testCoreIsPrimaryAndViewSnapshotsCannotResurrectRemovedPhotos() {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 1)
        state.currentChat?.participants = [ChatParticipantSnapshot(socialConnection: nil,
            ownerPubkeyHex: owner, displayName: "Stale view", pictureUrl: "https://example.invalid/old.png", isLocalOwner: false)]
        let client = MockRustApp(state: state)
        client.userProfiles[owner] = UserProfileSnapshot(ownerPubkeyHex: owner,
            name: PersonNamePresentation(name: "Current Ada", isFallback: false), pictureUrl: nil, socialConnection: nil)
        let profiles = IrisProfileStore()
        profiles.bind(client)
        profiles.apply(state)
        XCTAssertEqual(profiles[owner]?.name, "Current Ada")
        XCTAssertNil(profiles[owner]?.pictureUrl)
        client.userProfiles[owner]?.name.name = "New name"
        state.userDiscoveryRevision += 1
        profiles.apply(state)
        XCTAssertEqual(profiles[owner]?.name, "New name")
    }

    func testLogoutAndClientReplacementDoNotKeepLocalProfileCopies() {
        var state = buildLargeTestAppState(directChatCount: 1, groupChatCount: 0, messagesInCurrentChat: 0)
        let client = MockRustApp(state: state)
        client.userProfiles[owner] = UserProfileSnapshot(ownerPubkeyHex: owner,
            name: PersonNamePresentation(name: "Private nickname", isFallback: false), pictureUrl: nil, socialConnection: nil)
        let profiles = IrisProfileStore()
        profiles.bind(client)
        profiles.apply(state)
        XCTAssertEqual(profiles[owner]?.name, "Private nickname")
        state.account = nil
        profiles.apply(state)
        XCTAssertNil(profiles[owner])
        let replacement = MockRustApp(state: state)
        profiles.bind(replacement)
        state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0, messagesInCurrentChat: 0)
        profiles.apply(state)
        XCTAssertNotEqual(profiles[owner]?.name, "Private nickname")
    }
}
