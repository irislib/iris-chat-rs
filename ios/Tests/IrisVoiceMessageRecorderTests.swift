#if os(iOS)
import AVFoundation
import UIKit
import XCTest
@testable import IrisChat

@MainActor
final class IrisVoiceMessageRecorderTests: XCTestCase {
    func testReleaseBeforePermissionPreventsLateRecording() async throws {
        let permission = VoicePermissionGate()
        let backend = VoiceTestBackend()
        let recorder = makeRecorder(backend, permission: { await permission.request() })
        recorder.begin()
        try await eventually { await permission.isWaiting }

        let result = await recorder.finish()
        XCTAssertNil(result)
        XCTAssertEqual(recorder.phase, .idle)
        await permission.resolve(true)
        try await eventually { await permission.hasReturned }
        await Task.yield()
        let starts = await backend.startCount
        XCTAssertEqual(starts, 0)
        XCTAssertEqual(recorder.phase, .idle)
    }

    func testDeniedPermissionLeavesHelpfulErrorWithoutStartingRecorder() async throws {
        let backend = VoiceTestBackend()
        let recorder = makeRecorder(backend, permission: { false })
        recorder.begin()
        try await eventually { recorder.phase == .idle }
        XCTAssertNotNil(recorder.errorMessage)
        let starts = await backend.startCount
        XCTAssertEqual(starts, 0)
    }

    func testCancellationWhileRecorderStartsDiscardsLateCompletion() async throws {
        let backend = VoiceTestBackend(delayStart: true)
        let recorder = makeRecorder(backend)
        recorder.begin()
        try await eventually { await backend.isWaitingToStart }
        recorder.cancel()
        await backend.resumeStart()
        try await eventually { await backend.startReturned }
        try await eventually { await backend.discardCount > 0 }
        XCTAssertEqual(recorder.phase, .idle)
        XCTAssertNil(recorder.recordingURL)
        let exists = await backend.fileExists
        XCTAssertFalse(exists)
    }

    func testLockAndFinishKeepRecordingForPreviewUntilCancelled() async throws {
        let backend = VoiceTestBackend(duration: 2.4)
        let recorder = makeRecorder(backend)
        recorder.begin()
        try await eventually { recorder.phase == .recording }
        recorder.lock()
        XCTAssertEqual(recorder.phase, .locked)

        let result = await recorder.finish()
        XCTAssertEqual(recorder.phase, .ready)
        XCTAssertEqual(recorder.recordingURL, result)
        XCTAssertEqual(recorder.duration, 2.4, accuracy: 0.001)
        let exists = await backend.fileExists
        XCTAssertTrue(exists)
        XCTAssertEqual(recorder.level, 0)

        recorder.cancel()
        try await eventually { !(await backend.fileExists) }
        XCTAssertEqual(recorder.phase, .idle)
    }

    func testTapToRecordStartsLockedAndTransfersFileToAttachmentPipeline() async throws {
        let backend = VoiceTestBackend()
        let recorder = makeRecorder(backend)
        recorder.begin(locked: true)
        try await eventually { recorder.phase == .locked }
        let finishedURL = await recorder.finish()
        let transferredURL = recorder.takeRecording()
        XCTAssertEqual(transferredURL, finishedURL)
        XCTAssertNotNil(transferredURL)
        XCTAssertEqual(recorder.phase, .idle)
        try await eventually { await backend.releaseCount == 1 }

        recorder.cancel()
        let exists = await backend.fileExists
        XCTAssertTrue(exists, "The attachment pipeline owns the file after transfer.")
        if let transferredURL { try? FileManager.default.removeItem(at: transferredURL.deletingLastPathComponent()) }
    }

    func testRecordingShorterThanOneSecondIsDiscarded() async throws {
        let backend = VoiceTestBackend(duration: 0.7)
        let recorder = makeRecorder(backend)
        recorder.begin()
        try await eventually { recorder.phase == .recording }
        let result = await recorder.finish()
        XCTAssertNil(result)
        XCTAssertEqual(recorder.phase, .idle)
        XCTAssertNil(recorder.recordingURL)
        let exists = await backend.fileExists
        XCTAssertFalse(exists)
    }

    func testCancellationWhileFinishingNeverReturnsStaleFile() async throws {
        let backend = VoiceTestBackend(delayStop: true)
        let recorder = makeRecorder(backend)
        recorder.begin()
        try await eventually { recorder.phase == .recording }
        let finishing = Task { await recorder.finish() }
        try await eventually { await backend.isWaitingToStop }
        XCTAssertEqual(recorder.phase, .finishing)

        recorder.cancel()
        await backend.resumeStop()
        let result = await finishing.value
        XCTAssertNil(result)
        XCTAssertEqual(recorder.phase, .idle)
        XCTAssertNil(recorder.recordingURL)
        let exists = await backend.fileExists
        XCTAssertFalse(exists)
    }

    func testInterruptionPreservesPreviewAndLateGestureReleaseDoesNotSend() async throws {
        let notifications = NotificationCenter()
        let backend = VoiceTestBackend()
        let recorder = makeRecorder(backend, notificationCenter: notifications)
        recorder.begin()
        try await eventually { recorder.phase == .recording }

        notifications.post(name: AVAudioSession.interruptionNotification, object: nil, userInfo: [
            AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue,
        ])
        try await eventually { recorder.phase == .ready }
        XCTAssertNotNil(recorder.recordingURL)
        let mayDeactivate = await backend.stopMayDeactivateSession
        XCTAssertEqual(mayDeactivate, false)
        let lateRelease = await recorder.finish()
        XCTAssertNil(lateRelease)
        recorder.cancel()
    }

    func testInterruptionDuringFinishOnlyProducesPreview() async throws {
        let backend = VoiceTestBackend(delayStop: true)
        let recorder = makeRecorder(backend)
        recorder.begin()
        try await eventually { recorder.phase == .recording }
        let finishing = Task { await recorder.finish() }
        try await eventually { await backend.isWaitingToStop }
        await recorder.finishForInterruption()
        await backend.resumeStop()
        let result = await finishing.value
        XCTAssertNil(result)
        XCTAssertEqual(recorder.phase, .ready)
        XCTAssertNotNil(recorder.recordingURL)
        recorder.cancel()
    }

    func testBackgroundStopsRecordingIntoPreview() async throws {
        let notifications = NotificationCenter()
        let backend = VoiceTestBackend()
        let recorder = makeRecorder(backend, notificationCenter: notifications)
        recorder.begin(locked: true)
        try await eventually { recorder.phase == .locked }
        notifications.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
        try await eventually { recorder.phase == .ready }
        XCTAssertNotNil(recorder.recordingURL)
        let lateRelease = await recorder.finish()
        XCTAssertNil(lateRelease)
        recorder.cancel()
    }

    func testMaximumLengthStopsIntoPreviewWithoutSending() async throws {
        let backend = VoiceTestBackend(duration: 5, sampleDuration: 5, sampleEnded: true)
        let recorder = makeRecorder(backend, maximumDuration: 5, meteringInterval: 1_000_000)
        recorder.begin(locked: true)
        try await eventually { recorder.phase == .ready }
        XCTAssertEqual(recorder.duration, 5)
        XCTAssertNotNil(recorder.recordingURL)
        let lateRelease = await recorder.finish()
        XCTAssertNil(lateRelease)
        recorder.cancel()
    }

    func testCallPreventsStartingAndSavesExistingRecordingWithoutTakingSession() async throws {
        IrisAudioActivity.setCallActive(false)
        defer { IrisAudioActivity.setCallActive(false) }
        let backend = VoiceTestBackend()
        let recorder = makeRecorder(backend, notificationCenter: .default)
        recorder.begin()
        try await eventually { recorder.phase == .recording }
        IrisAudioActivity.setCallActive(true)
        try await eventually { recorder.phase == .ready }
        let mayDeactivate = await backend.stopMayDeactivateSession
        XCTAssertEqual(mayDeactivate, false)
        let lateRelease = await recorder.finish()
        XCTAssertNil(lateRelease)
        recorder.cancel()

        recorder.begin()
        XCTAssertEqual(recorder.phase, .idle)
        let starts = await backend.startCount
        XCTAssertEqual(starts, 1)
    }

    func testOldCancellationCannotReleaseNewRecordingsPlaybackExclusion() async throws {
        let firstBackend = VoiceTestBackend(delayDiscard: true)
        let first = makeRecorder(firstBackend)
        first.begin()
        try await eventually { first.phase == .recording }
        XCTAssertTrue(IrisAudioActivity.isRecordingActive)
        first.cancel()
        try await eventually { await firstBackend.isWaitingToDiscard }

        let secondBackend = VoiceTestBackend()
        let second = makeRecorder(secondBackend)
        second.begin()
        try await eventually { second.phase == .recording }
        await firstBackend.resumeDiscard()
        try await eventually { await firstBackend.discardCount == 1 }
        await Task.yield()
        XCTAssertTrue(IrisAudioActivity.isRecordingActive)

        second.cancel()
        try await eventually { !IrisAudioActivity.isRecordingActive }
    }

    private func makeRecorder(
        _ backend: VoiceTestBackend,
        permission: @escaping () async -> Bool = { true },
        notificationCenter: NotificationCenter = NotificationCenter(),
        maximumDuration: TimeInterval = 600,
        meteringInterval: UInt64 = 60_000_000_000
    ) -> IrisVoiceMessageRecorder {
        IrisVoiceMessageRecorder(
            backend: backend, requestPermission: permission, notificationCenter: notificationCenter,
            maximumDuration: maximumDuration, meteringIntervalNanoseconds: meteringInterval
        )
    }

    private func eventually(
        file: StaticString = #filePath, line: UInt = #line,
        _ condition: () async -> Bool
    ) async throws {
        for _ in 0..<200 {
            if await condition() { return }
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        XCTFail("Timed out waiting for recording lifecycle.", file: file, line: line)
    }
}

private actor VoicePermissionGate {
    private var continuation: CheckedContinuation<Bool, Never>?
    private(set) var hasReturned = false
    var isWaiting: Bool { continuation != nil }

    func request() async -> Bool {
        let allowed = await withCheckedContinuation { continuation = $0 }
        hasReturned = true
        return allowed
    }

    func resolve(_ allowed: Bool) {
        continuation?.resume(returning: allowed)
        continuation = nil
    }
}

private actor VoiceTestBackend: IrisVoiceRecordingBackend {
    private let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("iris-voice-recorder-test-" + UUID().uuidString, isDirectory: true)
        .appendingPathComponent("Voice message.m4a")
    private let duration: TimeInterval
    private let sampleDuration: TimeInterval
    private let sampleEnded: Bool
    private let delayStart: Bool
    private let delayStop: Bool
    private let delayDiscard: Bool
    private var startContinuation: CheckedContinuation<Void, Never>?
    private var stopContinuation: CheckedContinuation<Void, Never>?
    private var discardContinuation: CheckedContinuation<Void, Never>?
    private var didDelayDiscard = false
    private(set) var startCount = 0
    private(set) var discardCount = 0
    private(set) var releaseCount = 0
    private(set) var startReturned = false
    private(set) var stopMayDeactivateSession: Bool?
    var isWaitingToStart: Bool { startContinuation != nil }
    var isWaitingToStop: Bool { stopContinuation != nil }
    var isWaitingToDiscard: Bool { discardContinuation != nil }
    var fileExists: Bool { FileManager.default.fileExists(atPath: url.path) }

    init(duration: TimeInterval = 2, delayStart: Bool = false, delayStop: Bool = false, delayDiscard: Bool = false,
         sampleDuration: TimeInterval = 0.2, sampleEnded: Bool = false) {
        self.duration = duration
        self.delayStart = delayStart
        self.delayStop = delayStop
        self.delayDiscard = delayDiscard
        self.sampleDuration = sampleDuration
        self.sampleEnded = sampleEnded
    }

    func start(_ request: IrisVoiceRecordingRequest, maximumDuration: TimeInterval) async throws {
        startCount += 1
        if delayStart { await withCheckedContinuation { startContinuation = $0 } }
        defer { startReturned = true }
        guard !request.isCancelled else { throw CancellationError() }
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("test audio".utf8).write(to: url)
    }

    func sample(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecordingSample? {
        IrisVoiceRecordingSample(duration: sampleDuration, level: 0.4, ended: sampleEnded)
    }

    func stop(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecording? {
        if delayStop { await withCheckedContinuation { stopContinuation = $0 } }
        stopMayDeactivateSession = request.canDeactivateSession
        return IrisVoiceRecording(url: url, duration: duration)
    }

    func discard(_ request: IrisVoiceRecordingRequest) async {
        if delayDiscard && !didDelayDiscard {
            didDelayDiscard = true
            await withCheckedContinuation { discardContinuation = $0 }
        }
        discardCount += 1
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    }

    func release(_ request: IrisVoiceRecordingRequest) async { releaseCount += 1 }

    func resumeStart() {
        startContinuation?.resume()
        startContinuation = nil
    }

    func resumeStop() {
        stopContinuation?.resume()
        stopContinuation = nil
    }

    func resumeDiscard() {
        discardContinuation?.resume()
        discardContinuation = nil
    }
}
#endif
