import XCTest
#if os(iOS)
@testable import IrisChat

final class MobilePushSyncInputTests: XCTestCase {
    func testSilentAuthorChangeRefreshesSubscription() {
        var state = buildLargeTestAppState(directChatCount: 0, groupChatCount: 0, messagesInCurrentChat: 0)
        state.mobilePush = MobilePushSyncSnapshot(
            ownerPubkeyHex: "owner", messageAuthorPubkeys: ["author"],
            backgroundMessageAuthorPubkeys: [], inviteResponsePubkeys: [], sessions: []
        )
        var gate = IosStateSideEffectGate()
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "device-secret"))
        XCTAssertFalse(gate.shouldSyncMobilePush(state: state, ownerNsec: "device-secret"))
        state.mobilePush.backgroundMessageAuthorPubkeys = ["author"]
        XCTAssertTrue(gate.shouldSyncMobilePush(state: state, ownerNsec: "device-secret"))
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
