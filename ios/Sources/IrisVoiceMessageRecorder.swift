#if os(iOS)
import AVFoundation
import Combine
import UIKit

struct IrisVoiceRecordingSample: Sendable {
    let duration: TimeInterval
    let level: Double
    let ended: Bool
}

struct IrisVoiceRecording: Sendable {
    let url: URL
    let duration: TimeInterval
}

// Permission, recorder preparation, and gesture cancellation can overlap. This
// ticket also prevents cleanup from deactivating a session taken over by a call.
final class IrisVoiceRecordingRequest: @unchecked Sendable {
    let id = UUID()
    private let lock = NSLock()
    private var cancelled = false
    private var mayDeactivateSession = true

    var isCancelled: Bool { lock.withLock { cancelled } }
    var canDeactivateSession: Bool { lock.withLock { mayDeactivateSession } }

    func cancel() { lock.withLock { cancelled = true } }
    func relinquishSession() { lock.withLock { mayDeactivateSession = false } }
}

protocol IrisVoiceRecordingBackend: AnyObject, Sendable {
    func start(_ request: IrisVoiceRecordingRequest, maximumDuration: TimeInterval) async throws
    func sample(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecordingSample?
    func stop(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecording?
    func discard(_ request: IrisVoiceRecordingRequest) async
    func release(_ request: IrisVoiceRecordingRequest) async
}

private enum IrisVoiceCaptureOwnership {
    private static let lock = NSLock()
    private static var requests: Set<UUID> = []

    static func claim(_ request: IrisVoiceRecordingRequest) {
        lock.withLock {
            requests.insert(request.id)
            IrisAudioActivity.setRecordingActive(true)
        }
    }

    static func release(_ request: IrisVoiceRecordingRequest) {
        lock.withLock {
            requests.remove(request.id)
            IrisAudioActivity.setRecordingActive(!requests.isEmpty)
        }
    }
}

@MainActor
final class IrisVoiceMessageRecorder: ObservableObject {
    enum Phase: Equatable {
        case idle, requestingPermission, recording, locked, finishing, ready
    }

    @Published private(set) var phase: Phase = .idle
    @Published private(set) var duration: TimeInterval = 0
    @Published private(set) var level: Double = 0
    @Published private(set) var recordingURL: URL?
    @Published var errorMessage: String?

    var isRecording: Bool { phase == .recording || phase == .locked }

    private let backend: IrisVoiceRecordingBackend
    private let requestPermission: () async -> Bool
    private let notificationCenter: NotificationCenter
    private let maximumDuration: TimeInterval
    private let meteringIntervalNanoseconds: UInt64
    private var request: IrisVoiceRecordingRequest?
    private var meteringTask: Task<Void, Never>?
    private var observers: [NSObjectProtocol] = []
    private var locksWhenStarted = false
    private var previewOnly = false

    init(
        backend: IrisVoiceRecordingBackend = IrisAVVoiceRecordingBackend(),
        requestPermission: @escaping () async -> Bool = IrisVoiceMessageRecorder.microphonePermission,
        notificationCenter: NotificationCenter = .default,
        maximumDuration: TimeInterval = 10 * 60,
        meteringIntervalNanoseconds: UInt64 = 100_000_000
    ) {
        self.backend = backend
        self.requestPermission = requestPermission
        self.notificationCenter = notificationCenter
        self.maximumDuration = maximumDuration
        self.meteringIntervalNanoseconds = meteringIntervalNanoseconds
        observers.append(notificationCenter.addObserver(
            forName: AVAudioSession.interruptionNotification, object: nil, queue: nil
        ) { [weak self] notification in
            guard let rawValue = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: rawValue) == .began else { return }
            Task { @MainActor [weak self] in await self?.finishForInterruption() }
        })
        observers.append(notificationCenter.addObserver(
            forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: nil
        ) { [weak self] _ in
            Task { @MainActor [weak self] in await self?.finishForBackground() }
        })
        observers.append(notificationCenter.addObserver(
            forName: IrisAudioActivity.callDidChange, object: nil, queue: nil
        ) { [weak self] _ in
            guard IrisAudioActivity.isCallActive else { return }
            Task { @MainActor [weak self] in await self?.finishForInterruption() }
        })
    }

    deinit {
        observers.forEach(notificationCenter.removeObserver)
        meteringTask?.cancel()
        if let request {
            request.cancel()
            let backend = backend
            Task {
                await backend.discard(request)
                IrisVoiceCaptureOwnership.release(request)
            }
        }
    }

    func begin(locked: Bool = false) {
        guard phase == .idle, !IrisAudioActivity.isCallActive else { return }
        IrisAudioPlayback.pauseAll()
        let request = IrisVoiceRecordingRequest()
        self.request = request
        locksWhenStarted = locked
        previewOnly = false
        duration = 0
        level = 0
        recordingURL = nil
        errorMessage = nil
        phase = .requestingPermission
        let permission = requestPermission
        let backend = backend
        let maximumDuration = maximumDuration
        Task { [weak self] in
            let allowed = await permission()
            guard let self, self.owns(request) else { return }
            guard !IrisAudioActivity.isCallActive else {
                self.cancel()
                return
            }
            guard allowed else {
                self.reset()
                self.errorMessage = "Allow microphone access in Settings to record a voice message."
                return
            }
            IrisVoiceCaptureOwnership.claim(request)
            IrisAudioPlayback.pauseAll()
            do {
                try await backend.start(request, maximumDuration: maximumDuration)
                guard self.owns(request), !IrisAudioActivity.isCallActive else {
                    if IrisAudioActivity.isCallActive { request.relinquishSession() }
                    await backend.discard(request)
                    IrisVoiceCaptureOwnership.release(request)
                    if self.owns(request) { self.reset() }
                    return
                }
                self.phase = self.locksWhenStarted ? .locked : .recording
                self.startMetering(request)
            } catch {
                await backend.discard(request)
                IrisVoiceCaptureOwnership.release(request)
                guard self.owns(request) else { return }
                self.reset()
                self.errorMessage = "Couldn’t record a voice message. Try again."
            }
        }
    }

    func lock() {
        if phase == .requestingPermission { locksWhenStarted = true }
        else if phase == .recording { phase = .locked }
    }

    // A release while permission/preparation is pending cancels the request;
    // granting the prompt afterward must never start the microphone late.
    @discardableResult
    func finish() async -> URL? {
        if phase == .ready { return previewOnly ? nil : recordingURL }
        if phase == .requestingPermission {
            cancel()
            return nil
        }
        guard isRecording, let request else { return nil }
        phase = .finishing
        meteringTask?.cancel()
        meteringTask = nil
        let recording = await backend.stop(request)
        IrisVoiceCaptureOwnership.release(request)
        guard owns(request) else {
            await backend.discard(request)
            return nil
        }
        guard let recording else {
            await backend.discard(request)
            guard owns(request) else { return nil }
            reset()
            errorMessage = "Couldn’t save the voice message. Try again."
            return nil
        }
        guard recording.duration >= 1 else {
            await backend.discard(request)
            guard owns(request) else { return nil }
            reset()
            return nil
        }
        duration = recording.duration
        level = 0
        recordingURL = recording.url
        phase = .ready
        return previewOnly ? nil : recording.url
    }

    // Interruptions only produce a preview. They never return a URL to a send
    // action, and they relinquish the session before asynchronous cleanup.
    func finishForInterruption() async {
        request?.relinquishSession()
        previewOnly = true
        _ = await finish()
    }

    func cancel() {
        guard let request else {
            reset()
            return
        }
        request.cancel()
        reset()
        let backend = backend
        Task {
            await backend.discard(request)
            IrisVoiceCaptureOwnership.release(request)
        }
    }

    // Transfer ownership before passing the file to the attachment pipeline.
    // The pipeline may read it asynchronously after the composer resets.
    func takeRecording() -> URL? {
        guard phase == .ready, let url = recordingURL, let request else { return nil }
        reset()
        let backend = backend
        Task { await backend.release(request) }
        return url
    }

    private func finishForBackground() async {
        previewOnly = true
        _ = await finish()
    }

    private func owns(_ request: IrisVoiceRecordingRequest) -> Bool {
        self.request === request && !request.isCancelled
    }

    private func reset() {
        meteringTask?.cancel()
        meteringTask = nil
        request = nil
        phase = .idle
        duration = 0
        level = 0
        recordingURL = nil
        locksWhenStarted = false
        previewOnly = false
    }

    private func startMetering(_ request: IrisVoiceRecordingRequest) {
        let backend = backend
        let interval = meteringIntervalNanoseconds
        meteringTask = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(nanoseconds: interval) } catch { return }
                guard let sample = await backend.sample(request), !Task.isCancelled,
                      let self, self.owns(request), self.isRecording else { return }
                self.duration = sample.duration
                self.level = min(1, max(0, sample.level))
                if sample.ended || sample.duration >= self.maximumDuration {
                    self.previewOnly = true
                    _ = await self.finish()
                    return
                }
            }
        }
    }

    nonisolated private static func microphonePermission() async -> Bool {
        await withCheckedContinuation { continuation in
            AVAudioSession.sharedInstance().requestRecordPermission { allowed in
                continuation.resume(returning: allowed)
            }
        }
    }

    static func forComposer() -> IrisVoiceMessageRecorder {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        if environment["IRIS_UI_TEST_VOICE_RECORDING"] == "1",
           environment["IRIS_UI_TEST_RUN_ID"]?.isEmpty == false {
            return IrisVoiceMessageRecorder(
                backend: IrisSilentVoiceRecordingBackend(), requestPermission: { true }
            )
        }
        #endif
        return IrisVoiceMessageRecorder()
    }
}

// Every AVAudioRecorder/session operation and file write runs on this queue.
// Nothing here publishes UI state or blocks the main actor.
final class IrisAVVoiceRecordingBackend: IrisVoiceRecordingBackend, @unchecked Sendable {
    private let queue = DispatchQueue(label: "fi.siriusbusiness.irischat.voice-recording", qos: .userInitiated)
    private var recorder: AVAudioRecorder?
    private var activeRequest: IrisVoiceRecordingRequest?
    private var files: [UUID: URL] = [:]
    private var ownsSession = false

    func start(_ request: IrisVoiceRecordingRequest, maximumDuration: TimeInterval) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            queue.async {
                do {
                    guard !request.isCancelled, !IrisAudioActivity.isCallActive else { throw CancellationError() }
                    if let previous = self.activeRequest, previous.isCancelled {
                        self.discardOnQueue(previous)
                    }
                    let session = AVAudioSession.sharedInstance()
                    guard self.recorder == nil else {
                        throw RecordingError.unavailable
                    }
                    let directory = FileManager.default.temporaryDirectory
                        .appendingPathComponent("iris-voice-messages", isDirectory: true)
                        .appendingPathComponent(request.id.uuidString, isDirectory: true)
                    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                    let url = directory.appendingPathComponent("Voice message.m4a")
                    self.files[request.id] = url
                    guard !request.isCancelled, !IrisAudioActivity.isCallActive else { throw CancellationError() }
                    guard try IrisAudioActivity.withRecordingSessionIfNoCall({ () -> Bool in
                        guard !request.isCancelled else { throw CancellationError() }
                        try session.setCategory(.record, mode: .default)
                        self.activeRequest = request
                        self.ownsSession = true
                        try session.setActive(true)
                        return true
                    }) == true else { throw CancellationError() }
                    guard !request.isCancelled, !IrisAudioActivity.isCallActive else { throw CancellationError() }
                    let recorder = try AVAudioRecorder(url: url, settings: [
                        AVFormatIDKey: kAudioFormatMPEG4AAC,
                        AVSampleRateKey: 44_100,
                        AVNumberOfChannelsKey: 1,
                        AVEncoderBitRateKey: 64_000,
                        AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
                    ])
                    self.recorder = recorder
                    recorder.isMeteringEnabled = true
                    guard recorder.prepareToRecord(), !request.isCancelled, !IrisAudioActivity.isCallActive,
                          recorder.record(forDuration: maximumDuration) else {
                        throw RecordingError.unavailable
                    }
                    continuation.resume()
                } catch {
                    self.discardOnQueue(request)
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    func sample(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecordingSample? {
        await withCheckedContinuation { continuation in
            queue.async {
                guard self.activeRequest === request, let recorder = self.recorder else {
                    continuation.resume(returning: nil)
                    return
                }
                recorder.updateMeters()
                continuation.resume(returning: IrisVoiceRecordingSample(
                    duration: recorder.isRecording ? recorder.currentTime : self.fileDuration(recorder.url),
                    level: min(1, max(0, pow(10, Double(recorder.averagePower(forChannel: 0)) / 40))),
                    ended: !recorder.isRecording
                ))
            }
        }
    }

    func stop(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecording? {
        await withCheckedContinuation { continuation in
            queue.async {
                guard self.activeRequest === request, let recorder = self.recorder,
                      let url = self.files[request.id] else {
                    continuation.resume(returning: nil)
                    return
                }
                recorder.stop()
                let duration = self.fileDuration(url)
                self.recorder = nil
                self.deactivateOnQueue(request)
                self.activeRequest = nil
                continuation.resume(returning: duration > 0 ? IrisVoiceRecording(url: url, duration: duration) : nil)
            }
        }
    }

    func discard(_ request: IrisVoiceRecordingRequest) async {
        await withCheckedContinuation { continuation in
            queue.async {
                self.discardOnQueue(request)
                continuation.resume()
            }
        }
    }

    func release(_ request: IrisVoiceRecordingRequest) async {
        await withCheckedContinuation { continuation in
            queue.async {
                self.files.removeValue(forKey: request.id)
                continuation.resume()
            }
        }
    }

    private func discardOnQueue(_ request: IrisVoiceRecordingRequest) {
        if activeRequest === request {
            recorder?.stop()
            recorder = nil
            deactivateOnQueue(request)
            activeRequest = nil
        }
        if let url = files.removeValue(forKey: request.id) {
            try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
        }
    }

    private func deactivateOnQueue(_ request: IrisVoiceRecordingRequest) {
        let session = AVAudioSession.sharedInstance()
        IrisAudioActivity.withRecordingSessionIfNoCall {
            if ownsSession, request.canDeactivateSession, session.category == .record, session.mode == .default {
                do {
                    try session.setActive(false, options: .notifyOthersOnDeactivation)
                    try session.setCategory(.ambient, mode: .default)
                } catch {
                    // A system interruption may already own the session.
                }
            }
        }
        ownsSession = false
    }

    private func fileDuration(_ url: URL) -> TimeInterval {
        guard let file = try? AVAudioFile(forReading: url), file.processingFormat.sampleRate > 0 else { return 0 }
        return Double(file.length) / file.processingFormat.sampleRate
    }

    private enum RecordingError: Error { case unavailable }
}

#if DEBUG
// UI automation exercises the production state machine without opening the
// microphone. The only replacement is a silent AAC capture device.
private actor IrisSilentVoiceRecordingBackend: IrisVoiceRecordingBackend {
    private var startTimes: [UUID: Date] = [:]
    private var maximumDurations: [UUID: TimeInterval] = [:]
    private var files: [UUID: URL] = [:]

    func start(_ request: IrisVoiceRecordingRequest, maximumDuration: TimeInterval) async throws {
        guard !request.isCancelled else { throw CancellationError() }
        startTimes[request.id] = Date()
        maximumDurations[request.id] = maximumDuration
    }

    func sample(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecordingSample? {
        guard let start = startTimes[request.id], let maximum = maximumDurations[request.id] else { return nil }
        let elapsed = Date().timeIntervalSince(start)
        return IrisVoiceRecordingSample(duration: min(elapsed, maximum), level: 0.35, ended: elapsed >= maximum)
    }

    func stop(_ request: IrisVoiceRecordingRequest) async -> IrisVoiceRecording? {
        guard let start = startTimes.removeValue(forKey: request.id),
              let maximum = maximumDurations.removeValue(forKey: request.id), !request.isCancelled else { return nil }
        let duration = min(Date().timeIntervalSince(start), maximum)
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-voice-messages-tests", isDirectory: true)
            .appendingPathComponent(request.id.uuidString, isDirectory: true)
        let url = directory.appendingPathComponent("Voice message.m4a")
        files[request.id] = url
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let sampleRate = 44_100.0
            let file = try AVAudioFile(forWriting: url, settings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: sampleRate,
                AVNumberOfChannelsKey: 1,
                AVEncoderBitRateKey: 64_000,
            ])
            guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 1_024),
                  let channel = buffer.floatChannelData?[0] else { return nil }
            channel.initialize(repeating: 0, count: 1_024)
            var remaining = AVAudioFramePosition(duration * sampleRate)
            while remaining > 0 {
                guard !request.isCancelled else { throw CancellationError() }
                buffer.frameLength = AVAudioFrameCount(min(1_024, remaining))
                try file.write(from: buffer)
                remaining -= AVAudioFramePosition(buffer.frameLength)
            }
            return IrisVoiceRecording(url: url, duration: duration)
        } catch {
            await discard(request)
            return nil
        }
    }

    func discard(_ request: IrisVoiceRecordingRequest) async {
        startTimes.removeValue(forKey: request.id)
        maximumDurations.removeValue(forKey: request.id)
        if let url = files.removeValue(forKey: request.id) {
            try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
        }
    }

    func release(_ request: IrisVoiceRecordingRequest) async { files.removeValue(forKey: request.id) }
}
#endif
#endif
