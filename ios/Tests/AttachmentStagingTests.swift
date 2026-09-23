#if os(iOS)
import AVFoundation
import UIKit
import XCTest
@testable import IrisChat

private final class StagingProbeFileManager: FileManager, @unchecked Sendable {
    var onCopy: (() -> Void)?

    override func copyItem(at srcURL: URL, to dstURL: URL) throws {
        onCopy?()
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
    func testBundledIrisLogoIsStagedAndDispatchedAsAnAttachment() throws {
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
            dataDir: dataDir,
            environment: [:]
        )
        let staged = try XCTUnwrap(manager.stageOutgoingAttachments([sourceURL]).first)

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
}
#endif
