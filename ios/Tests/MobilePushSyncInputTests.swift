import XCTest
#if os(iOS)
@testable import IrisChat

final class MobilePushSyncInputTests: XCTestCase {
    func testSilentAuthorChangeRefreshesSubscription() {
        var state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0, messagesInCurrentChat: 0)
        state.mobilePush = MobilePushSyncSnapshot(
            callDevicePubkeyHex: nil, callAuthorPubkeys: [],
            ownerPubkeyHex: "owner", messageAuthorPubkeys: ["author"],
            delayedMessageAuthors: [],
            backgroundMessageAuthorPubkeys: [], inviteResponsePubkeys: [], sessions: []
        )
        var gate = IosStateSideEffectGate()
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "device-secret"))
        XCTAssertFalse(gate.shouldSyncMobilePush(state: state, ownerNsec: "device-secret"))
        state.mobilePush.backgroundMessageAuthorPubkeys = ["author"]
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "device-secret"))
    }

    func testMuteDeadlineChangeRefreshesSubscription() {
        var state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0, messagesInCurrentChat: 0)
        state.mobilePush.delayedMessageAuthors = [MobilePushDelayedAuthor(authorPubkey: "author", sinceSecs: 100)]
        var gate = IosStateSideEffectGate()
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "secret"))
        XCTAssertFalse(gate.shouldSyncMobilePush(state: state, ownerNsec: "secret"))
        state.mobilePush.delayedMessageAuthors[0].sinceSecs = 200
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "secret"))
    }

    func testLinkedDevicePushCredentialsSurvivePersistenceWithoutAccountSecret() throws {
        let linked = StoredAccountBundle(ownerNsec: nil, ownerPubkeyHex: "account", deviceNsec: "device-secret")
        let restored = try JSONDecoder().decode(StoredAccountBundle.self, from: JSONEncoder().encode(linked))
        XCTAssertNil(restored.ownerNsec)
        XCTAssertEqual(restored.mobilePushAuthNsec, "device-secret")
        XCTAssertEqual(StoredAccountBundle(ownerNsec: "account-secret", ownerPubkeyHex: "account", deviceNsec: "device-secret").mobilePushAuthNsec, "account-secret")
    }
}
#endif
