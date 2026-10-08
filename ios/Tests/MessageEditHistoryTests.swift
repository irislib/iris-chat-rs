import XCTest
#if os(macOS)
import AppKit
import SwiftUI
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class MessageEditHistoryTests: XCTestCase {
    private func editedMessage() -> ChatMessageSnapshot {
        var message = makeMessage(chatId: "history-chat", id: "history-message", body: "See you at noon")
        message.editHistory = [
            MessageEditSnapshot(id: "original", body: "See you at ten", createdAtSecs: 100),
            MessageEditSnapshot(id: "edit-1", body: "See you at eleven", createdAtSecs: 110),
            MessageEditSnapshot(id: "edit-2", body: "See you at noon", createdAtSecs: 120)
        ]
        return message
    }

    private func state(_ message: ChatMessageSnapshot) -> AppState {
        makeLargeFixtureState(
            router: Router(defaultScreen: .chatList, screenStack: [.chat(chatId: message.chatId)]),
            account: makeAccount(), chatList: [],
            currentChat: makeCurrentChat(chatId: message.chatId, messages: [message])
        )
    }

    func testHistoryActionCoversReceivedAndSentMessagesButNotDeletedExpiredOrUnedited() {
        var message = editedMessage()
        for outgoing in [true, false] {
            message.isOutgoing = outgoing
            XCTAssertTrue(irisCanViewMessageEditHistory(message, now: 200))
        }
        message.expiresAtSecs = 200
        XCTAssertTrue(irisCanViewMessageEditHistory(message, now: 199))
        XCTAssertFalse(irisCanViewMessageEditHistory(message, now: 200))
        message.expiresAtSecs = nil
        message.deletedForEveryone = true
        XCTAssertFalse(irisCanViewMessageEditHistory(message, now: 200))
        XCTAssertTrue(irisMessageEditHistoryRows(message).isEmpty)
        message.deletedForEveryone = false
        message.editHistory = Array(message.editHistory.prefix(1))
        XCTAssertFalse(irisCanViewMessageEditHistory(message, now: 200))
        message.editHistory = []
        XCTAssertFalse(irisCanViewMessageEditHistory(message, now: 200))
    }

    func testViewerOrdersCurrentThenEarlierEditsThenOriginalWithoutChangingTimestamps() {
        let rows = irisMessageEditHistoryRows(editedMessage())
        XCTAssertEqual(rows.map(\.id), ["edit-2", "edit-1", "original"])
        XCTAssertEqual(rows.map(\.label), ["Current", "Edit 1", "Original"])
        XCTAssertEqual(rows.map(\.body), ["See you at noon", "See you at eleven", "See you at ten"])
        XCTAssertEqual(rows.map(\.createdAtSecs), [120, 110, 100])
    }

    func testHistoryUsesLiveEditsAndRejectsDeletedExpiredMissingAndOtherAccountContent() {
        let initial = editedMessage()
        let target = MessageEditHistoryTarget(message: initial, accountID: "owner")
        let original = state(initial)
        XCTAssertEqual(target.resolve(in: original, now: 200), initial)
        var updated = original
        updated.currentChat?.messages[0].body = "See you at one"
        updated.currentChat?.messages[0].editHistory.append(
            MessageEditSnapshot(id: "edit-3", body: "See you at one", createdAtSecs: 150))
        XCTAssertEqual(target.resolve(in: updated, now: 200)?.editHistory.count, 4)
        XCTAssertEqual(target.resolve(in: updated, now: 200)?.body, "See you at one")

        updated.currentChat?.messages[0].deletedForEveryone = true
        XCTAssertNil(target.resolve(in: updated, now: 200))
        updated = original
        updated.currentChat?.messages[0].expiresAtSecs = 200
        XCTAssertNil(target.resolve(in: updated, now: 200))
        updated = original
        updated.currentChat?.messages.removeAll()
        XCTAssertNil(target.resolve(in: updated, now: 200), "local delete must not restore the sheet snapshot")
        updated = original
        updated.currentChat = nil
        XCTAssertNil(target.resolve(in: updated, now: 200))
        updated = original
        updated.currentChat?.chatId = "another-chat"
        XCTAssertNil(target.resolve(in: updated, now: 200))
        updated = original
        updated.router.screenStack = [.settings]
        XCTAssertNil(target.resolve(in: updated, now: 200), "leaving the chat dismisses history")
        updated = original
        updated.account = nil
        XCTAssertNil(target.resolve(in: updated, now: 200))
        updated = original
        updated.account?.publicKeyHex = "another-owner"
        XCTAssertNil(target.resolve(in: updated, now: 200))
    }

#if os(macOS)
    @MainActor
    func testHistoryViewerRendersReadableVersionsAtDesktopAndPhoneWidths() throws {
        for width in [390.0, 640.0] {
            let content = MessageEditHistoryVersions(message: editedMessage())
                .frame(width: width)
                .background(Color.white)
                .environment(\.irisPalette, .light)
                .environment(\.colorScheme, .light)
            let renderer = ImageRenderer(content: content)
            renderer.scale = 2
            let image = try XCTUnwrap(renderer.cgImage)
            XCTAssertGreaterThan(image.height, 150)
            let attachment = XCTAttachment(image: NSImage(cgImage: image, size: .zero))
            attachment.name = "edit-history-\(Int(width))"
            attachment.lifetime = .keepAlways
            add(attachment)
            if let directory = ProcessInfo.processInfo.environment["IRIS_EDIT_HISTORY_ARTIFACT_DIR"] {
                let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
                try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("edit-history-\(Int(width)).png"))
            }
        }
    }
#endif
}
