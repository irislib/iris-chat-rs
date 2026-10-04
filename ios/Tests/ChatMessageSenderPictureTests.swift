import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ChatMessageSenderPictureTests: XCTestCase {
    func testCurrentParticipantPictureOverridesTheMessageSnapshot() {
        XCTAssertEqual(irisGroupSenderPictureURL(messagePictureURL: "htree://old",
            participant: participant(picture: "htree://current")), "htree://current")
    }

    func testRemovedParticipantPictureDoesNotResurrectTheMessagePhoto() {
        XCTAssertNil(irisGroupSenderPictureURL(messagePictureURL: "htree://old",
                                              participant: participant(picture: nil)))
    }

    func testHistoricalSenderWithoutAParticipantUsesTheDecoratedMessagePicture() {
        XCTAssertEqual(irisGroupSenderPictureURL(messagePictureURL: "htree://former-member",
                                                participant: nil), "htree://former-member")
        XCTAssertNil(irisGroupSenderPictureURL(messagePictureURL: nil, participant: nil))
    }

    private func participant(picture: String?) -> ChatParticipantSnapshot {
        ChatParticipantSnapshot(socialConnection: nil, ownerPubkeyHex: "alice",
            displayName: "Alice", pictureUrl: picture, isLocalOwner: false)
    }
}
