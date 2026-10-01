import Foundation
import UniformTypeIdentifiers
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class AttachmentDropTests: XCTestCase {
    func testDropStagesMultipleFilesInOrderWithoutRetainingSource() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let sources = [directory.appendingPathComponent("first.txt"), directory.appendingPathComponent("second.pdf")]
        for (index, url) in sources.enumerated() { try Data([UInt8(index)]).write(to: url) }
        // Exercise the file-URL contract requested by IrisAttachmentDropModifier.
        // contentsOf: advertises file contents rather than a URL on iOS.
        let providers = sources.map { NSItemProvider(item: $0 as NSURL, typeIdentifier: UTType.fileURL.identifier) }
        XCTAssertTrue(providers.allSatisfy { $0.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) })
        let loaded = await IrisDroppedFiles.load(providers)
        XCTAssertEqual(loaded, sources)
        let staging = IrisAttachmentStaging(dataDir: directory.appendingPathComponent("cache"), fileManager: .default)
        let staged = try await staging.stageAsync(loaded)
        XCTAssertEqual(staged.map(\.filename), ["first.txt", "second.pdf"])
        for url in sources { try FileManager.default.removeItem(at: url) }
        for (index, attachment) in staged.enumerated() {
            XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: attachment.path)), Data([UInt8(index)]))
        }
    }

    func testDropRejectsRemoteURLsAndUnsupportedProviders() async {
        XCTAssertNil(droppedFileURL(from: NSURL(string: "https://example.com/file.pdf")))
        XCTAssertNil(droppedFileURL(from: "hello" as NSString))
        let urls = await IrisDroppedFiles.load([NSItemProvider(object: "hello" as NSString)])
        XCTAssertTrue(urls.isEmpty)
    }

    func testFailedSelectionDiscardsEarlierCopiesAndRejectsDirectories() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let source = directory.appendingPathComponent("valid.txt")
        try Data("hello".utf8).write(to: source)
        let cache = directory.appendingPathComponent("cache")
        let staging = IrisAttachmentStaging(dataDir: cache, fileManager: .default)
        do {
            _ = try await staging.stageAsync([source, directory])
            XCTFail("Directories must not be copied into a draft")
        } catch { }
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: cache.appendingPathComponent("attachments/outgoing").path), [])
        XCTAssertThrowsError(try staging.stage(URL(string: "https://example.com/file.pdf")!))
    }

    func testCancelledProviderSelectionDoesNotStageFiles() async throws {
        let source = URL(fileURLWithPath: "/tmp/drop-cancelled.txt")
        let started = expectation(description: "provider started")
        let provider = NSItemProvider()
        provider.registerItem(forTypeIdentifier: UTType.fileURL.identifier) { completion, _, _ in
            started.fulfill()
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.1) { completion?(source as NSURL, nil) }
        }
        let load = Task { await IrisDroppedFiles.load([provider]) }
        await fulfillment(of: [started], timeout: 2)
        load.cancel()
        let urls = await load.value
        XCTAssertTrue(urls.isEmpty)
    }
}
