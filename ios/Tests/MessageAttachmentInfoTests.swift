import XCTest
import SwiftUI
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
import UIKit
@testable import IrisChat
#endif

final class MessageAttachmentInfoTests: XCTestCase {
    private let upload = "https://files.example/" + String(repeating: "a", count: 64) + ".bin"

    private func attachment(uploads: [String]) -> MessageAttachmentSnapshot {
        MessageAttachmentSnapshot(nhash: "fixture", filename: "photo.jpg", filenameEncoded: "photo.jpg",
                                  htreeUrl: "htree://fixture/photo.jpg", isImage: true, isVideo: false,
                                  isAudio: false, uploadedUrls: uploads)
    }

    func testLocalReceiptsAreCopiedInDetailsButNeverForwardedOrPutInRumor() {
        var message = makeMessage(chatId: "chat", id: "message", body: "A photo")
        message.attachments = [attachment(uploads: [])]
        XCTAssertFalse(messageInfoText(message).contains("Encrypted uploads"))
        message.attachments = [attachment(uploads: [upload])]
        XCTAssertTrue(messageInfoText(message).contains(upload))
        XCTAssertEqual(forwardableMessageText(message), "A photo\nhtree://fixture/photo.jpg")
        XCTAssertFalse(synthesizeMessageRumorJson(message: message, chat: nil, account: nil).contains(upload))
        XCTAssertEqual(attachmentUploadURL(upload)?.absoluteString, upload)
        for unsafe in ["file:///tmp/photo.jpg", "htree://secret/photo.jpg", "javascript:alert(1)",
                       "https://user:secret@files.example/blob", "https://files.example/blob#key",
                       "https://files.example/blob?secret=key"] {
            XCTAssertNil(attachmentUploadURL(unsafe))
        }
    }

    @MainActor
    func testProductionAttachmentDetailsRenderFullUploadURLsAtPhoneAndDesktopWidths() throws {
        for width in [390.0, 640.0] {
            let content = MessageInfoAttachmentSection(attachments: [attachment(uploads: [upload])])
                .padding(20).frame(width: width)
                .background(Color.white)
                .environment(\.irisPalette, .light)
                .environment(\.colorScheme, .light)
            let renderer = ImageRenderer(content: content)
            renderer.scale = 2
            let image = try XCTUnwrap(renderer.cgImage)
            XCTAssertGreaterThan(image.height, 100)
#if os(macOS)
            let screenshot = XCTAttachment(image: NSImage(cgImage: image, size: .zero))
#else
            let screenshot = XCTAttachment(image: UIImage(cgImage: image))
#endif
            screenshot.name = "encrypted-attachment-uploads-\(Int(width))"
            screenshot.lifetime = .keepAlways
            add(screenshot)
        }
    }
}
