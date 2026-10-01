import Foundation
import SwiftUI
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
import UIKit
@testable import IrisChat
#endif

@MainActor
final class DirectFileTransferTests: XCTestCase {
    func testSelectedFilesUseDirectOfferActionWithoutUpload() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sources = [directory.appendingPathComponent("one.txt"), directory.appendingPathComponent("two.pdf")]
        for source in sources { try Data(source.lastPathComponent.utf8).write(to: source) }
        let files = try IrisAttachmentStaging(dataDir: directory, fileManager: .default).stage(sources)
        let action = irisAttachmentSendAction(chatId: "self-chat", attachments: files, caption: "For my laptop", sendDirectly: true)
        guard case let .sendDirectFiles(chatId, attachments, caption) = action else {
            return XCTFail("Direct files must use the explicit offer action")
        }
        XCTAssertEqual(chatId, "self-chat")
        XCTAssertEqual(attachments.map(\.filename), ["one.txt", "two.pdf"])
        XCTAssertEqual(attachments.map(\.filePath), files.map(\.path))
        XCTAssertEqual(caption, "For my laptop")
        for file in files { XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: file.path)), Data(file.filename.utf8)) }
    }

    func testCompletedFileExportPreservesOriginalNameAndBytes() async throws {
        let source = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + "-0-original.txt")
        let content = Data("Direct transfer".utf8)
        try content.write(to: source)
        defer { try? FileManager.default.removeItem(at: source) }
        let output = try await irisDirectFileExportURL(path: source.path, filename: "original.txt")
        defer { try? FileManager.default.removeItem(at: output.deletingLastPathComponent()) }
        XCTAssertEqual(output.lastPathComponent, "original.txt")
        XCTAssertEqual(try Data(contentsOf: output), content)
    }

    func testAddingFilesNeverChangesADirectSelectionIntoAnUpload() {
        XCTAssertTrue(irisDirectFileSendMode(current: true, hasFiles: true, selectedDirectly: false))
        XCTAssertTrue(irisDirectFileSendMode(current: false, hasFiles: true, selectedDirectly: true))
        XCTAssertFalse(irisDirectFileSendMode(current: true, hasFiles: false, selectedDirectly: false))
    }

    func testOtherDeviceCanAcceptASelfChatOffer() {
        let recipient = fixture(isSender: false)
        XCTAssertTrue(recipient.canAcceptOnThisDevice)
        XCTAssertFalse(recipient.canCancelOnThisDevice)
        let sender = fixture(isSender: true)
        XCTAssertFalse(sender.canAcceptOnThisDevice)
        XCTAssertTrue(sender.canCancelOnThisDevice)
        for status: DirectFileTransferStatus in [.completed, .cancelled, .declined, .failed, .unavailable] {
            XCTAssertFalse(fixture(isSender: false, status: status).canAcceptOnThisDevice)
            XCTAssertFalse(fixture(isSender: false, status: status).canCancelOnThisDevice)
        }
    }

    func testDirectFileOfferAndProgressRenderNatively() throws {
        let view = VStack(alignment: .leading, spacing: 24) {
            ChatDirectFileTransferView(transfer: fixture(isSender: false), chatId: "self-chat", dispatch: { _ in })
            ChatDirectFileTransferView(transfer: fixture(isSender: true, status: .transferring), chatId: "self-chat", dispatch: { _ in })
        }
        .padding(20)
        .frame(width: 320)
        .background(Color.white)
        .foregroundStyle(Color.black)
        .environment(\.irisPalette, .light)
        .preferredColorScheme(.light)
        let png: Data
#if os(macOS)
        let host = NSHostingView(rootView: view)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 640), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host
        window.orderFront(nil)
        defer { window.orderOut(nil) }
        host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
#else
        let host = UIHostingController(rootView: view)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 640))
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        host.view.layoutIfNeeded()
        png = try XCTUnwrap(UIGraphicsImageRenderer(bounds: host.view.bounds).image { _ in
            host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
        }.pngData())
#endif
        XCTAssertGreaterThan(png.count, 1_000)
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = "direct-file-offer-and-progress"
        attachment.lifetime = .keepAlways
        add(attachment)
        if let output = ProcessInfo.processInfo.environment["IRIS_UI_EVIDENCE_DIR"] {
            try png.write(to: URL(fileURLWithPath: output).appendingPathComponent("direct-file-offer-and-progress.png"))
        }
    }

    private func fixture(isSender: Bool, status: DirectFileTransferStatus = .offered) -> DirectFileTransferSnapshot {
        DirectFileTransferSnapshot(id: "test-transfer", files: [
            DirectFileSnapshot(filename: "Weekend photos.zip", sizeBytes: 1_024_000, localPath: nil),
            DirectFileSnapshot(filename: "Packing list.txt", sizeBytes: 512, localPath: nil),
        ], status: status, isSender: isSender, transferredBytes: 512_000, totalBytes: 1_024_512, error: nil)
    }
}
