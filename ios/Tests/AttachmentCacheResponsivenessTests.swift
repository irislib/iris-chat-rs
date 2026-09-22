import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

private final class CacheProbeFileManager: FileManager, @unchecked Sendable {
    var onTouch: (() -> Void)?
    var onScan: (() -> Void)?

    override func setAttributes(_ attributes: [FileAttributeKey: Any], ofItemAtPath path: String) throws {
        onTouch?()
        try super.setAttributes(attributes, ofItemAtPath: path)
    }

    override func contentsOfDirectory(
        at url: URL,
        includingPropertiesForKeys keys: [URLResourceKey]?,
        options mask: FileManager.DirectoryEnumerationOptions = []
    ) throws -> [URL] {
        onScan?()
        return try super.contentsOfDirectory(at: url, includingPropertiesForKeys: keys, options: mask)
    }
}

final class AttachmentCacheResponsivenessTests: XCTestCase {
    @MainActor
    func testCachedPictureDoesNotBlockMainActorOnDiskIO() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let downloaded = directory.appendingPathComponent("attachments/downloaded", isDirectory: true)
        try FileManager.default.createDirectory(at: downloaded, withIntermediateDirectories: true)
        let expected = Data("cached picture".utf8)
        try expected.write(to: downloaded.appendingPathComponent("picture-test-hash"))
        let started = expectation(description: "cache I/O started off the main thread")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        let files = CacheProbeFileManager()
        files.onTouch = {
            XCTAssertFalse(Thread.isMainThread)
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
        }
        let manager = AppManager(
            rust: MockRustApp(),
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            desktopNotifications: NoopDesktopNotificationPoster(),
            dataDir: directory,
            fileManager: files,
            environment: [:]
        )
        let load = Task { await manager.resolveHashtreePictureBytes(nhash: " test-hash ") }
        await fulfillment(of: [started], timeout: 2)
        // The UI executor can resume even while the cache is stalled.
        gate.signal()
        let actual = await load.value
        XCTAssertEqual(actual, expected)
    }

    @MainActor
    func testStoreAndPruneRunOffMainAndReuseExistingBlob() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let scanned = expectation(description: "only the new blob requires a scan")
        scanned.assertForOverFulfill = true
        let files = CacheProbeFileManager()
        files.onScan = {
            XCTAssertFalse(Thread.isMainThread)
            scanned.fulfill()
        }
        files.onTouch = { XCTAssertFalse(Thread.isMainThread) }
        let cache = IrisAttachmentCache(dataDir: directory, fileManager: files)
        let data = Data("attachment".utf8)
        let key = IrisAttachmentCache.attachmentKey(nhash: "hash", filename: "image.png")
        let firstURL = try await cache.store(data, for: key)
        let firstFileNumber = try FileManager.default.attributesOfItem(atPath: firstURL.path)[.systemFileNumber] as? NSNumber
        let secondURL = try await cache.store(data, for: key)
        let secondFileNumber = try FileManager.default.attributesOfItem(atPath: secondURL.path)[.systemFileNumber] as? NSNumber
        let loaded = await cache.data(for: key)
        XCTAssertEqual(loaded, data)
        XCTAssertEqual(firstURL, secondURL)
        XCTAssertNotNil(firstFileNumber)
        XCTAssertEqual(firstFileNumber, secondFileNumber)
        await fulfillment(of: [scanned], timeout: 1)
    }

    func testEvictionKeepsRecentlyReadAndNewFiles() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let cache = IrisAttachmentCache(dataDir: directory, limitBytes: 8)
        let data = Data(repeating: 1, count: 4)
        let first = try await cache.store(data, for: "first")
        let second = try await cache.store(data, for: "second")
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 1)], ofItemAtPath: first.path)
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 2)], ofItemAtPath: second.path)
        _ = await cache.data(for: "first")
        let newest = try await cache.store(data, for: "third")
        XCTAssertTrue(FileManager.default.fileExists(atPath: first.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: second.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: newest.path))
    }

    func testKeysRetainExistingCacheLayoutAndSanitizePathSeparators() {
        XCTAssertEqual(IrisAttachmentCache.pictureKey(nhash: "hash"), "picture-hash")
        XCTAssertEqual(IrisAttachmentCache.attachmentKey(nhash: "hash", filename: "photo.png"), "hash-photo.png")
        XCTAssertEqual(IrisAttachmentCache.attachmentKey(nhash: "hash", filename: "../path\\file:name"), "hash-..-path-file-name")
    }
}
