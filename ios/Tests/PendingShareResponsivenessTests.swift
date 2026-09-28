#if os(iOS)
import XCTest
@testable import IrisChat

private final class ShareProbeFileManager: FileManager, @unchecked Sendable {
    var onScan: (() -> Void)?
    var onCopy: (() -> Void)?
    var onRead: (() -> Void)?

    override func contentsOfDirectory(at url: URL, includingPropertiesForKeys keys: [URLResourceKey]?, options mask: FileManager.DirectoryEnumerationOptions = []) throws -> [URL] {
        if url.lastPathComponent == "pending-shares" { onScan?() }
        return try super.contentsOfDirectory(at: url, includingPropertiesForKeys: keys, options: mask)
    }

    override func copyItem(at srcURL: URL, to dstURL: URL) throws {
        onCopy?()
        try super.copyItem(at: srcURL, to: dstURL)
    }

    override func contents(atPath path: String) -> Data? {
        let data = super.contents(atPath: path)
        onRead?()
        return data
    }
}

final class PendingShareResponsivenessTests: XCTestCase {
    @MainActor
    func testShareScanDoesNotBlockUIOrRepeatForEveryStateUpdate() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory.appendingPathComponent("pending-shares"), withIntermediateDirectories: true)
        let files = ShareProbeFileManager()
        let scanStarted = expectation(description: "share scan started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        files.onScan = {
            XCTAssertFalse(Thread.isMainThread, "Share inbox I/O must not block UI updates")
            scanStarted.fulfill()
            if !Thread.isMainThread { XCTAssertEqual(gate.wait(timeout: .now() + 3), .success) }
        }
        let rust = MockRustApp()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory.appendingPathComponent("app"), fileManager: files, environment: ["IRIS_SHARE_CONTAINER_DIR": directory.path])
        await fulfillment(of: [scanStarted], timeout: 2)
        for revision in 1...20 {
            var snapshot = rust.currentState
            snapshot.rev = UInt64(revision)
            manager.apply(update: .fullState(snapshot), generation: 0)
        }
        XCTAssertEqual(manager.state.rev, 20)
        gate.signal()
        await Task.yield()
    }

    @MainActor
    func testSharedAttachmentCopyDoesNotBlockUIOrSendTwice() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let inbox = directory.appendingPathComponent("pending-shares")
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        let source = directory.appendingPathComponent("photo.txt")
        try Data("shared bytes".utf8).write(to: source)
        let share = PendingShare(id: "share-test", text: "caption", attachments: [PendingShareAttachment(path: source.path, filename: "photo.txt")], suggestedChatId: nil, suggestedChatIds: nil, autoSend: false, isForward: nil)
        try JSONEncoder().encode(share).write(to: inbox.appendingPathComponent("share-test.json"))
        let files = ShareProbeFileManager()
        let copyStarted = expectation(description: "copy started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        files.onCopy = {
            XCTAssertFalse(Thread.isMainThread, "Shared attachment copies must not block the UI")
            copyStarted.fulfill()
            if !Thread.isMainThread { XCTAssertEqual(gate.wait(timeout: .now() + 3), .success) }
        }
        let rust = MockRustApp()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory.appendingPathComponent("app"), fileManager: files, environment: ["IRIS_SHARE_CONTAINER_DIR": directory.path])
        let loaded = await waitUntil { manager.pendingShare != nil }
        XCTAssertTrue(loaded)
        manager.sendPendingShare(to: "recipient")
        await fulfillment(of: [copyStarted], timeout: 2)
        manager.sendPendingShare(to: "recipient")
        gate.signal()
        let sent = await waitUntil { manager.pendingShare == nil }
        XCTAssertTrue(sent)
        let sends = rust.dispatchedActions.filter {
            if case .sendAttachments = $0 { return true }
            return false
        }
        XCTAssertEqual(sends.count, 1)
    }

    @MainActor
    func testCancellingShareDuringCopyDoesNotSendOrReplaceNewForward() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let inbox = directory.appendingPathComponent("pending-shares")
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        let source = directory.appendingPathComponent("photo.txt")
        try Data("shared bytes".utf8).write(to: source)
        let share = PendingShare(id: "cancel-share", text: "cancel me", attachments: [PendingShareAttachment(path: source.path, filename: "photo.txt")], suggestedChatId: nil, suggestedChatIds: nil, autoSend: false, isForward: nil)
        try JSONEncoder().encode(share).write(to: inbox.appendingPathComponent("cancel-share.json"))
        let files = ShareProbeFileManager()
        let started = expectation(description: "copy started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        files.onCopy = {
            XCTAssertFalse(Thread.isMainThread)
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
        }
        let rust = MockRustApp()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory.appendingPathComponent("app"), fileManager: files, environment: ["IRIS_SHARE_CONTAINER_DIR": directory.path])
        let loaded = await waitUntil { manager.pendingShare != nil }
        XCTAssertTrue(loaded)
        manager.sendPendingShare(to: "recipient")
        await fulfillment(of: [started], timeout: 2)
        XCTAssertTrue(manager.isSendingPendingShare)
        manager.startForward(text: "keep this")
        XCTAssertFalse(manager.isSendingPendingShare)
        gate.signal()
        let outgoing = directory.appendingPathComponent("app/attachments/outgoing")
        let discarded = await waitUntil {
            (try? FileManager.default.contentsOfDirectory(atPath: outgoing.path).isEmpty) == true
                && !FileManager.default.fileExists(atPath: inbox.appendingPathComponent("cancel-share.json").path)
        }
        XCTAssertTrue(discarded)
        XCTAssertEqual(manager.pendingShare?.text, "Forwarded:\n\nkeep this")
        XCTAssertFalse(rust.dispatchedActions.contains {
            if case .sendAttachments = $0 { return true }
            return false
        })
    }

    @MainActor
    func testLateDuplicateShareURLDoesNotResendCompletedAttachment() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let inbox = directory.appendingPathComponent("pending-shares")
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        let source = directory.appendingPathComponent("photo.txt")
        try Data("shared bytes".utf8).write(to: source)
        let share = PendingShare(id: "duplicate", text: "send once", attachments: [PendingShareAttachment(path: source.path, filename: "photo.txt")], suggestedChatId: "recipient", suggestedChatIds: nil, autoSend: false, isForward: nil)
        let payload = inbox.appendingPathComponent("duplicate.json")
        try JSONEncoder().encode(share).write(to: payload)
        let files = ShareProbeFileManager()
        let copyStarted = expectation(description: "copy started")
        let readStarted = expectation(description: "duplicate read started")
        let copyGate = DispatchSemaphore(value: 0)
        let readGate = DispatchSemaphore(value: 0)
        defer { copyGate.signal(); readGate.signal() }
        files.onCopy = {
            copyStarted.fulfill()
            XCTAssertEqual(copyGate.wait(timeout: .now() + 3), .success)
        }
        let rust = MockRustApp()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(), pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory.appendingPathComponent("app"), fileManager: files, environment: ["IRIS_SHARE_CONTAINER_DIR": directory.path])
        let loaded = await waitUntil { manager.pendingShare != nil }
        XCTAssertTrue(loaded)
        manager.sendPendingShare(to: "recipient")
        await fulfillment(of: [copyStarted], timeout: 2)
        files.onRead = {
            readStarted.fulfill()
            XCTAssertEqual(readGate.wait(timeout: .now() + 3), .success)
        }
        XCTAssertTrue(manager.handleShareURL(URL(string: "irischat://share/duplicate?send=1")!))
        await fulfillment(of: [readStarted], timeout: 2)
        copyGate.signal()
        let sent = await waitUntil { manager.pendingShare == nil }
        XCTAssertTrue(sent)
        readGate.signal()
        let removed = await waitUntil { !FileManager.default.fileExists(atPath: payload.path) }
        XCTAssertTrue(removed)
        XCTAssertNil(manager.pendingShare)
        XCTAssertEqual(rust.dispatchedActions.filter {
            if case .sendAttachments = $0 { return true }
            return false
        }.count, 1)
    }
}
#endif
