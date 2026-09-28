#if os(iOS)
import AVFoundation
import UIKit
import XCTest
@testable import IrisChat

private final class StagingProbeFileManager: FileManager, @unchecked Sendable {
    var onCopy: (() -> Void)?
    var onCopySource: ((URL) -> Void)?

    override func copyItem(at srcURL: URL, to dstURL: URL) throws {
        onCopy?()
        onCopySource?(srcURL)
        try super.copyItem(at: srcURL, to: dstURL)
    }
}

final class AttachmentStagingTests: XCTestCase {
    @MainActor
    func testVoiceRecordingStagesOffMainSurvivesSourceDisposalAndDispatchesOnce() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let sourceDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer {
            try? FileManager.default.removeItem(at: dataDir)
            try? FileManager.default.removeItem(at: sourceDir)
        }
        let sourceURL = sourceDir.appendingPathComponent("Voice message.m4a")
        try await Task.detached {
            try FileManager.default.createDirectory(at: sourceDir, withIntermediateDirectories: true)
            let file = try AVAudioFile(forWriting: sourceURL, settings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 44_100,
                AVNumberOfChannelsKey: 1,
                AVEncoderBitRateKey: 64_000,
            ])
            let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 44_100))
            let channel = try XCTUnwrap(buffer.floatChannelData?[0])
            channel.initialize(repeating: 0, count: 44_100)
            buffer.frameLength = 44_100
            try file.write(from: buffer)
        }.value
        let recordedBytes = try Data(contentsOf: sourceURL)
        XCTAssertFalse(recordedBytes.isEmpty)

        let rust = MockRustApp(state: makeAppState(rev: 1))
        let files = StagingProbeFileManager()
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: dataDir,
            fileManager: files,
            environment: [:]
        )
        let copyStarted = expectation(description: "copy started off the UI thread")
        let copyGate = DispatchSemaphore(value: 0)
        defer { copyGate.signal() }
        files.onCopy = {
            XCTAssertFalse(Thread.isMainThread)
            copyStarted.fulfill()
            XCTAssertEqual(copyGate.wait(timeout: .now() + 3), .success)
        }
        let staging = Task { try await manager.stageOutgoingAttachmentsAsync([sourceURL]) }
        await fulfillment(of: [copyStarted], timeout: 2)
        // The UI actor resumes while the real production copy is blocked.
        copyGate.signal()
        let attachments = try await staging.value
        let staged = try XCTUnwrap(attachments.first)
        XCTAssertEqual(chatAttachmentCategory(from: staged.filename), .audio)
        XCTAssertFalse(rust.dispatchedActions.contains {
            if case .sendAttachments = $0 { return true }
            return false
        }, "Staging a preview must not send it")

        // The composer disposes its temporary recording after the attachment
        // pipeline accepts the copied file. Uploads must not retain that source.
        try FileManager.default.removeItem(at: sourceDir)
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: staged.path)), recordedBytes)

        manager.sendAttachments(chatId: "chat-voice-test", attachments: [staged], caption: "")
        let sends = rust.dispatchedActions.filter { action in
            if case let .sendAttachments(chatId, attachments, caption) = action {
                return chatId == "chat-voice-test" && caption.isEmpty && attachments.count == 1
                    && attachments[0].filename == "Voice message.m4a"
                    && attachments[0].filePath == staged.path
            }
            return false
        }
        XCTAssertEqual(sends.count, 1)
    }

    @MainActor
    func testCancellingAsyncStagingRemovesTheUnsentCopy() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let sourceURL = directory.appendingPathComponent("Voice message.m4a")
        let sourceBytes = Data("recording bytes".utf8)
        try sourceBytes.write(to: sourceURL)
        let files = StagingProbeFileManager()
        let manager = AppManager(
            rust: MockRustApp(state: makeAppState(rev: 1)),
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: directory,
            fileManager: files,
            environment: [:]
        )
        let copyStarted = expectation(description: "copy started")
        let copyGate = DispatchSemaphore(value: 0)
        defer { copyGate.signal() }
        files.onCopy = {
            copyStarted.fulfill()
            XCTAssertEqual(copyGate.wait(timeout: .now() + 3), .success)
        }
        let staging = Task { try await manager.stageOutgoingAttachmentsAsync([sourceURL]) }
        await fulfillment(of: [copyStarted], timeout: 2)
        staging.cancel()
        copyGate.signal()
        do {
            _ = try await staging.value
            XCTFail("A cancelled recording must not leave an upload copy")
        } catch is CancellationError {
            // Expected after the in-progress copy is removed.
        }

        let outgoing = directory.appendingPathComponent("attachments/outgoing", isDirectory: true)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: outgoing.path), [])
        XCTAssertEqual(try Data(contentsOf: sourceURL), sourceBytes)
    }

    @MainActor
    func testBundledIrisLogoIsStagedAndDispatchedAsAnAttachment() async throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let sourceDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer {
            try? FileManager.default.removeItem(at: dataDir)
            try? FileManager.default.removeItem(at: sourceDir)
        }
        try FileManager.default.createDirectory(at: sourceDir, withIntermediateDirectories: true)
        let sourceURL = sourceDir.appendingPathComponent("iris-logo.png")
        let logo = try XCTUnwrap(UIImage(named: "IrisLogo")?.pngData())
        try logo.write(to: sourceURL)

        let rust = MockRustApp(state: makeAppState(rev: 1))
        let manager = AppManager(
            rust: rust,
            secretStore: InMemorySecretStore(),
            pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(),
            dataDir: dataDir,
            environment: [:]
        )
        let attachments = try await manager.stageOutgoingAttachmentsAsync([sourceURL])
        let staged = try XCTUnwrap(attachments.first)

        XCTAssertTrue(staged.path.contains("/attachments/outgoing/"))
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: staged.path)), logo)

        manager.sendAttachments(
            chatId: "chat-logo-test",
            attachments: [staged],
            caption: "Iris logo"
        )
        XCTAssertTrue(rust.dispatchedActions.contains { action in
            if case let .sendAttachments(chatId, attachments, caption) = action {
                return chatId == "chat-logo-test"
                    && caption == "Iris logo"
                    && attachments.count == 1
                    && attachments[0].filename == "iris-logo.png"
                    && attachments[0].filePath == staged.path
            }
            return false
        })
    }

    @MainActor
    func testProfilePictureStagesOffMainBeforeDispatching() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let source = directory.appendingPathComponent("profile.png")
        let bytes = Data("profile picture".utf8)
        try bytes.write(to: source)
        let rust = MockRustApp(state: makeAppState(rev: 1))
        let files = StagingProbeFileManager()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory,
                                 fileManager: files, environment: [:])
        let started = expectation(description: "profile copy started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        files.onCopy = {
            XCTAssertFalse(Thread.isMainThread)
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
        }
        let upload = Task { await manager.uploadProfilePicture { source } }
        await fulfillment(of: [started], timeout: 2)
        XCTAssertFalse(rust.dispatchedActions.contains {
            if case .uploadProfilePicture = $0 { return true }
            return false
        })
        gate.signal()
        await upload.value
        let paths = rust.dispatchedActions.compactMap { action -> String? in
            if case let .uploadProfilePicture(path) = action { return path }
            return nil
        }
        XCTAssertEqual(paths.count, 1)
        let path = try XCTUnwrap(paths.first)
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: path)), bytes)
    }

    @MainActor
    func testCancelledOlderGroupPictureCannotReplaceNewerSelection() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let oldSource = directory.appendingPathComponent("old.png")
        let newSource = directory.appendingPathComponent("new.png")
        try Data("old picture".utf8).write(to: oldSource)
        let newBytes = Data("new picture".utf8)
        try newBytes.write(to: newSource)
        let rust = MockRustApp(state: makeAppState(rev: 1))
        let files = StagingProbeFileManager()
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory,
                                 fileManager: files, environment: [:])
        let started = expectation(description: "old picture copy started")
        let gate = DispatchSemaphore(value: 0)
        defer { gate.signal() }
        files.onCopySource = { source in
            XCTAssertFalse(Thread.isMainThread)
            if source == oldSource {
                started.fulfill()
                XCTAssertEqual(gate.wait(timeout: .now() + 3), .success)
            }
        }
        let oldSelection = Task { await manager.updateGroupPicture(groupId: "group-test") { oldSource } }
        await fulfillment(of: [started], timeout: 2)
        oldSelection.cancel()
        await manager.updateGroupPicture(groupId: "group-test") { newSource }
        gate.signal()
        await oldSelection.value

        let paths = rust.dispatchedActions.compactMap { action -> String? in
            if case let .updateGroupPicture(groupId, path, filename) = action {
                XCTAssertEqual(groupId, "group-test")
                XCTAssertEqual(filename, "new.png")
                return path
            }
            return nil
        }
        XCTAssertEqual(paths.count, 1)
        let path = try XCTUnwrap(paths.first)
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: path)), newBytes)
        let outgoing = directory.appendingPathComponent("attachments/outgoing")
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: outgoing.path).count, 1)
    }

    @MainActor
    func testLogoutDuringPhotoExportPreventsCopyAndUploadIntoNextSession() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let rust = MockRustApp(state: makeAppState(rev: 1))
        let freshRust = MockRustApp(state: makeAppState(rev: 0))
        let files = StagingProbeFileManager()
        files.onCopy = { XCTFail("An old session's photo must not be staged") }
        let manager = AppManager(rust: rust, secretStore: InMemorySecretStore(),
                                 pendingDeviceLinkSecretStore: InMemoryPendingDeviceLinkSecretStore(), dataDir: directory,
                                 fileManager: files, environment: [:], rustFactory: { freshRust })
        let marker = directory.appendingPathComponent("old-core-data")
        try Data("must survive until shutdown completes".utf8).write(to: marker)
        let shutdownStarted = expectation(description: "old core shutting down off the UI thread")
        let shutdownGate = DispatchSemaphore(value: 0)
        defer { shutdownGate.signal() }
        rust.onShutdown = {
            XCTAssertFalse(Thread.isMainThread)
            shutdownStarted.fulfill()
            XCTAssertEqual(shutdownGate.wait(timeout: .now() + 3), .success)
        }
        let started = expectation(description: "photo export started")
        var completion: CheckedContinuation<URL?, Never>?
        let upload = Task {
            await manager.uploadProfilePicture {
                await withCheckedContinuation { continuation in
                    completion = continuation
                    started.fulfill()
                }
            }
        }
        await fulfillment(of: [started], timeout: 2)
        manager.logout()
        await fulfillment(of: [shutdownStarted], timeout: 2)
        // The UI actor can resume while shutdown is blocked, but the old
        // core's files cannot be removed and its pending export is invalid.
        XCTAssertTrue(manager.bootstrapInFlight)
        XCTAssertTrue(FileManager.default.fileExists(atPath: marker.path))
        completion?.resume(returning: directory.appendingPathComponent("old-session.png"))
        await upload.value
        XCTAssertTrue(FileManager.default.fileExists(atPath: marker.path))
        shutdownGate.signal()
        let resetCompleted = await waitUntil { !manager.bootstrapInFlight && manager.state.rev == 0 }
        XCTAssertTrue(resetCompleted)
        XCTAssertEqual(rust.shutdownCallCount, 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: marker.path))
        for client in [rust, freshRust] {
            XCTAssertFalse(client.dispatchedActions.contains {
                if case .uploadProfilePicture = $0 { return true }
                return false
            })
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.appendingPathComponent("attachments/outgoing").path))
    }
}
#endif
