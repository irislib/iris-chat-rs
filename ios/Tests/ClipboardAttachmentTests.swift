import Foundation
import ImageIO
import UniformTypeIdentifiers
import XCTest
#if os(iOS)
import UIKit
@testable import IrisChat
#elseif os(macOS)
import AppKit
@testable import IrisChatMac
#endif

final class ClipboardAttachmentTests: XCTestCase {
    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    private func provider(_ type: UTType, data: Data, name: String? = nil) -> NSItemProvider {
        let provider = NSItemProvider(item: data as NSData, typeIdentifier: type.identifier)
        provider.suggestedName = name
        return provider
    }

    func testTextAndWebAddressesRemainOrdinaryPaste() {
        for type in [UTType.utf8PlainText, .rtf, .html, .url, .data] {
            XCTAssertNil(IrisClipboardAttachments(providers: [provider(type, data: Data("caption".utf8))]))
        }
    }

    func testMultipleFilesAndImagesStageInOrderAndCleanClipboardCopies() async throws {
        let root = try temporaryDirectory()
        let scratch = root.appendingPathComponent("clipboard")
        let source = root.appendingPathComponent("notes.txt")
        try Data("notes".utf8).write(to: source)
        let file = NSItemProvider(item: source as NSURL, typeIdentifier: UTType.fileURL.identifier)
        file.registerDataRepresentation(forTypeIdentifier: UTType.png.identifier, visibility: .all) { completion in
            XCTFail("A second representation must not duplicate the copied file")
            completion(Data([9]), nil)
            return nil
        }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [
            file, provider(.png, data: Data([1, 2, 3]), name: "picture"),
            provider(.pdf, data: Data("%PDF".utf8), name: "document.pdf")
        ]))
        let staging = IrisAttachmentStaging(dataDir: root.appendingPathComponent("data"), fileManager: .default)
        var staged: [StagedAttachment] = []
        var loaded: [URL] = []
        await clipboard.withURLs(in: scratch) { load in
            loaded = await load()
            do { staged = try await staging.stageAsync(loaded) }
            catch { XCTFail("Staging failed: \(error)") }
        }
        XCTAssertEqual(staged.map(\.filename), ["notes.txt", "picture.png", "document.pdf"])
        XCTAssertEqual(try staged.map { try Data(contentsOf: URL(fileURLWithPath: $0.path)) },
                       [Data("notes".utf8), Data([1, 2, 3]), Data("%PDF".utf8)])
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
        XCTAssertTrue(loaded.dropFirst().allSatisfy { !FileManager.default.fileExists(atPath: $0.path) })
        XCTAssertEqual((try? FileManager.default.contentsOfDirectory(atPath: scratch.path)) ?? [], [])
    }

    func testCopiesProviderFileBeforeItsCallbackExpires() async throws {
        let root = try temporaryDirectory()
        let source = root.appendingPathComponent("ephemeral.pdf")
        try Data("%PDF data".utf8).write(to: source)
        let provider = NSItemProvider()
        provider.registerFileRepresentation(forTypeIdentifier: UTType.pdf.identifier, fileOptions: [], visibility: .all) { completion in
            completion(source, false, nil)
            return nil
        }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [provider]))
        await clipboard.withURLs { load in
            let urls = await load()
            try? FileManager.default.removeItem(at: source)
            XCTAssertEqual(urls.count, 1)
            XCTAssertNotEqual(urls.first, source)
            XCTAssertEqual(urls.first.flatMap { try? Data(contentsOf: $0) }, Data("%PDF data".utf8))
        }
    }

    func testNamedTextDocumentIsAnAttachmentRatherThanCaptionText() async throws {
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [
            provider(.utf8PlainText, data: Data("document contents".utf8), name: "notes.txt")
        ]))
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.map(\.lastPathComponent), ["notes.txt"])
            XCTAssertEqual(urls.first.flatMap { try? Data(contentsOf: $0) }, Data("document contents".utf8))
        }
    }

    func testEmptyNamedDocumentRemainsAnAttachment() async throws {
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [
            provider(.utf8PlainText, data: Data(), name: "empty.txt")
        ]))
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.map(\.lastPathComponent), ["empty.txt"])
            XCTAssertEqual(urls.first.flatMap { try? Data(contentsOf: $0) }, Data())
        }
    }

    func testRawImagePrefersAdvertisedPNGWithoutReencodingIt() async throws {
        let png = try encodedImage(.png)
        let image = NSItemProvider()
        image.registerDataRepresentation(forTypeIdentifier: UTType.tiff.identifier, visibility: .all) { completion in
            XCTFail("Raw clipboard image must prefer its PNG representation")
            completion(nil, CocoaError(.fileReadUnknown))
            return nil
        }
        image.registerDataRepresentation(forTypeIdentifier: UTType.png.identifier, visibility: .all) { completion in
            completion(png, nil)
            return nil
        }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [image]))
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.count, 1)
            XCTAssertEqual(urls.first?.pathExtension, "png")
            XCTAssertEqual(urls.first.flatMap { try? Data(contentsOf: $0) }, png)
        }
    }

    func testRawTIFFBecomesFullSizeOrientedPNG() async throws {
        let tiff = try encodedImage(.tiff, orientation: 6)
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [provider(.tiff, data: tiff)]))
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.count, 1)
            XCTAssertEqual(urls.first?.pathExtension, "png")
            guard let url = urls.first, let source = CGImageSourceCreateWithURL(url as CFURL, nil) else {
                return XCTFail("Missing converted PNG")
            }
            XCTAssertEqual(CGImageSourceGetType(source).map { $0 as String }, UTType.png.identifier)
            let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
            XCTAssertEqual(properties?[kCGImagePropertyPixelWidth] as? Int, 16)
            XCTAssertEqual(properties?[kCGImagePropertyPixelHeight] as? Int, 24)
        }
    }

    func testRawHEICBecomesPNGWhenPlatformCanProvideHEIC() async throws {
        let heic: Data
        do { heic = try encodedImage(.heic) }
        catch { throw XCTSkip("The platform cannot encode the HEIC clipboard fixture") }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [provider(.heic, data: heic)]))
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.count, 1)
            XCTAssertEqual(urls.first?.pathExtension, "png")
            guard let url = urls.first, let source = CGImageSourceCreateWithURL(url as CFURL, nil) else {
                return XCTFail("Missing converted PNG")
            }
            XCTAssertEqual(CGImageSourceGetType(source).map { $0 as String }, UTType.png.identifier)
            let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
            XCTAssertEqual(properties?[kCGImagePropertyPixelWidth] as? Int, 24)
            XCTAssertEqual(properties?[kCGImagePropertyPixelHeight] as? Int, 16)
        }
    }

    func testOriginalFilesAndNamedImageRepresentationsPreserveBytes() async throws {
        let root = try temporaryDirectory()
        let tiff = try encodedImage(.tiff)
        let png = try encodedImage(.png)
        let original = root.appendingPathComponent("original.tiff")
        try tiff.write(to: original)
        let file = NSItemProvider(item: original as NSURL, typeIdentifier: UTType.fileURL.identifier)
        let namedTIFF = provider(.png, data: png, name: "kept.tif")
        namedTIFF.registerDataRepresentation(forTypeIdentifier: UTType.tiff.identifier, visibility: .all) { completion in
            completion(tiff, nil)
            return nil
        }
        let namedPNG = provider(.tiff, data: tiff, name: "kept.png")
        namedPNG.registerDataRepresentation(forTypeIdentifier: UTType.png.identifier, visibility: .all) { completion in
            completion(png, nil)
            return nil
        }
        let mismatchedName = provider(.tiff, data: tiff, name: "mismatch.png")
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [file, namedTIFF, namedPNG, mismatchedName]))
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.map(\.lastPathComponent), ["original.tiff", "kept.tif", "kept.png", "mismatch.tiff"])
            XCTAssertEqual(urls.map { try? Data(contentsOf: $0) }, [tiff, tiff, png, tiff])
        }
        XCTAssertEqual(try Data(contentsOf: original), tiff)
    }

    private func encodedImage(_ type: UTType, orientation: Int = 1) throws -> Data {
        guard let context = CGContext(data: nil, width: 24, height: 16, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            throw CocoaError(.fileWriteUnknown)
        }
        context.setFillColor(CGColor(red: 0.1, green: 0.4, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 24, height: 16))
        let data = NSMutableData()
        guard let image = context.makeImage(),
              let destination = CGImageDestinationCreateWithData(data as CFMutableData, type.identifier as CFString, 1, nil) else {
            throw CocoaError(.fileWriteUnknown)
        }
        CGImageDestinationAddImage(destination, image, [kCGImagePropertyOrientation: orientation] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw CocoaError(.fileWriteUnknown) }
        return data as Data
    }

    @MainActor
    func testNativeImageClipboardProviderProducesRecognizableImageFile() async throws {
        #if os(iOS)
        let image = UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { context in
            UIColor.systemBlue.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 24, height: 24))
        }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [NSItemProvider(object: image)]))
        let png = try XCTUnwrap(image.pngData())
        #else
        let image = NSImage(size: NSSize(width: 24, height: 24))
        image.lockFocus()
        NSColor.systemBlue.setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: 24, height: 24)).fill()
        image.unlockFocus()
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        XCTAssertTrue(pasteboard.writeObjects([image]))
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(pasteboard: pasteboard))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        #endif
        await clipboard.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.count, 1)
            guard let url = urls.first else { return }
            XCTAssertEqual(url.pathExtension, "png")
            XCTAssertEqual(chatAttachmentCategory(from: url.lastPathComponent), .image)
            XCTAssertNotNil(CGImageSourceCreateWithURL(url as CFURL, nil))
        }
        // Some providers advertise only the abstract image type. Infer the
        // extension from the actual bytes, never assume those bytes are PNG.
        let abstract = try XCTUnwrap(IrisClipboardAttachments(providers: [provider(.image, data: png)]))
        await abstract.withURLs { load in
            let urls = await load()
            XCTAssertEqual(urls.first?.pathExtension, "png")
            XCTAssertEqual(urls.first.flatMap { try? Data(contentsOf: $0) }, png)
        }
    }

    #if os(iOS)
    @MainActor
    func testClipboardStagingDoesNotSendAndCanUseDirectSendToSelf() async throws {
        let root = try temporaryDirectory()
        let rust = MockRustApp(state: makeAppState(rev: 1))
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
                                 dataDir: root, environment: [:])
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [
            provider(.png, data: Data([1]), name: "picture.png"),
            provider(.pdf, data: Data([2]), name: "document.pdf")
        ]))
        var staged: [StagedAttachment] = []
        await clipboard.withURLs { load in
            do { staged = try await manager.stageOutgoingAttachmentsAsync(load) }
            catch { XCTFail("Clipboard staging failed: \(error)") }
        }
        XCTAssertEqual(staged.count, 2)
        XCTAssertFalse(rust.dispatchedActions.contains { action in
            switch action {
            case .sendAttachments, .sendDirectFiles: return true
            default: return false
            }
        })
        let action = irisAttachmentSendAction(chatId: "self-chat", attachments: staged, caption: "caption", sendDirectly: true)
        guard case let .sendDirectFiles(chatId, attachments, caption) = action else {
            return XCTFail("Direct mode must retain a direct offer")
        }
        XCTAssertEqual(chatId, "self-chat")
        XCTAssertEqual(attachments.map(\.filename), staged.map(\.filename))
        XCTAssertEqual(attachments.map(\.filePath), staged.map(\.path))
        XCTAssertEqual(caption, "caption")
    }
    #endif

    func testUnreadableItemRejectsWholeBatchAndRemovesTemporaryFiles() async throws {
        let scratch = try temporaryDirectory()
        let missing = NSItemProvider()
        missing.registerDataRepresentation(forTypeIdentifier: UTType.pdf.identifier, visibility: .all) { completion in
            completion(nil, CocoaError(.fileReadNoSuchFile))
            return nil
        }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [
            provider(.png, data: Data([1])), missing
        ]))
        await clipboard.withURLs(in: scratch) { load in
            let urls = await load()
            XCTAssertTrue(urls.isEmpty)
        }
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: scratch.path), [])
    }

    func testCancellationAfterDelayedProviderDiscardsBatchAndTemporaryFiles() async throws {
        let scratch = try temporaryDirectory()
        let started = expectation(description: "provider started")
        let delayed = NSItemProvider()
        delayed.registerDataRepresentation(forTypeIdentifier: UTType.pdf.identifier, visibility: .all) { completion in
            started.fulfill()
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.1) { completion(Data([2]), nil) }
            return nil
        }
        let clipboard = try XCTUnwrap(IrisClipboardAttachments(providers: [provider(.png, data: Data([1])), delayed]))
        let task = Task {
            await clipboard.withURLs(in: scratch) { load in
                let urls = await load()
                XCTAssertTrue(urls.isEmpty)
            }
        }
        await fulfillment(of: [started], timeout: 2)
        task.cancel()
        await task.value
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: scratch.path), [])
    }

    @MainActor
    func testNativeAttachmentPastePreservesCaptionAndSelection() throws {
        var batches: [IrisClipboardAttachments] = []
        #if os(iOS)
        let view = IrisComposerUITextView()
        view.text = "existing caption"
        view.selectedRange = NSRange(location: 3, length: 4)
        view.onPasteAttachments = { batches.append($0) }
        view.paste(itemProviders: [provider(.png, data: Data([1])), provider(.pdf, data: Data([2]))])
        XCTAssertEqual(view.text, "existing caption")
        XCTAssertEqual(view.selectedRange, NSRange(location: 3, length: 4))
        XCTAssertFalse(view.pasteAttachments([provider(.utf8PlainText, data: Data("text".utf8))]))
        #elseif os(macOS)
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let image = NSPasteboardItem()
        image.setData(Data([1]), forType: .png)
        image.setData(Data([9]), forType: .tiff)
        let file = NSPasteboardItem()
        file.setString("file:///tmp/notes.txt", forType: .fileURL)
        pasteboard.writeObjects([image, file])
        let view = IrisComposerNSTextView()
        view.string = "existing caption"
        view.setSelectedRange(NSRange(location: 3, length: 4))
        view.onPasteAttachments = { batches.append($0) }
        XCTAssertTrue(view.pasteAttachments(from: pasteboard))
        XCTAssertEqual(view.string, "existing caption")
        XCTAssertEqual(view.selectedRange(), NSRange(location: 3, length: 4))
        pasteboard.clearContents()
        pasteboard.setString("ordinary text", forType: .string)
        XCTAssertFalse(view.pasteAttachments(from: pasteboard))
        #endif
        XCTAssertEqual(batches.count, 1)
        XCTAssertEqual(batches.first?.providers.count, 2)
    }
}
