#if os(iOS)
import AVFoundation
import SwiftUI
import UIKit

@MainActor
final class IrisAudioPlayback: ObservableObject {
    @Published private(set) var isPlaying = false
    @Published private(set) var isLoading = false
    @Published private(set) var elapsed: TimeInterval = 0
    @Published private(set) var duration: TimeInterval
    @Published private(set) var errorMessage: String?

    private static weak var activePlayback: IrisAudioPlayback?
    private let localURL: URL?
    private let filename: String
    private let loadData: (() async -> Data?)?
    private var player: AVPlayer?
    private var itemObservation: NSKeyValueObservation?
    private var timeObserver: Any?
    private var notifications: [NSObjectProtocol] = []
    private var loadTask: Task<Void, Never>?
    private var activationTask: Task<Void, Never>?
    private var loadID = UUID()
    private var temporaryURL: URL?
    private var wantsToPlay = false

    init(localURL: URL, duration: TimeInterval = 0) {
        self.localURL = localURL
        self.filename = localURL.lastPathComponent
        self.loadData = nil
        self.duration = Self.validTime(duration)
        observeAudioLifecycle()
    }

    init(filename: String, loadData: @escaping () async -> Data?) {
        self.localURL = nil
        self.filename = filename
        self.loadData = loadData
        self.duration = 0
        observeAudioLifecycle()
    }

    deinit {
        loadTask?.cancel()
        activationTask?.cancel()
        player?.pause()
        if let timeObserver { player?.removeTimeObserver(timeObserver) }
        for token in notifications { NotificationCenter.default.removeObserver(token) }
        if let temporaryURL { Self.removeTemporaryAudio(temporaryURL) }
    }

    static func pauseAll() {
        activePlayback?.pause()
    }

    func toggle() {
        if isPlaying || isLoading { pause() }
        else { play() }
    }

    func play() {
        guard canPlay else {
            errorMessage = IrisAudioActivity.isCallActive ? "Finish your call to play audio." : "Audio is unavailable."
            return
        }
        Self.activePlayback?.pause()
        Self.activePlayback = self
        errorMessage = nil
        wantsToPlay = true

        if let player, player.currentItem?.status != .failed {
            if player.currentItem?.status == .readyToPlay { startPreparedPlayer() }
            else { isLoading = true }
            return
        }

        releasePlayer()
        isLoading = true
        let requestID = UUID()
        loadID = requestID
        let localURL = localURL
        let loadData = loadData
        let filename = filename
        loadTask = Task { @MainActor [weak self] in
            var ownedURL: URL?
            do {
                let url: URL
                if let localURL {
                    url = localURL
                } else {
                    guard let data = await loadData?(), !data.isEmpty else {
                        throw AudioLoadError.unavailable
                    }
                    try Task.checkCancellation()
                    url = try await Self.writeTemporaryAudio(data, filename: filename)
                    ownedURL = url
                }
                try Task.checkCancellation()
                // AVURLAsset loads metadata and decodes audio asynchronously.
                let asset = AVURLAsset(url: url)
                let playable = try await asset.load(.isPlayable)
                let assetDuration = try await asset.load(.duration)
                try Task.checkCancellation()
                guard playable else { throw AudioLoadError.unavailable }
                guard let self, self.loadID == requestID, self.wantsToPlay, self.canPlay else {
                    if let ownedURL { Self.removeTemporaryAudio(ownedURL) }
                    return
                }
                self.temporaryURL = ownedURL
                self.duration = Self.validTime(assetDuration.seconds)
                self.installPlayer(asset: asset)
                self.loadTask = nil
            } catch {
                if let ownedURL { Self.removeTemporaryAudio(ownedURL) }
                guard let self, self.loadID == requestID, !Task.isCancelled else { return }
                self.loadTask = nil
                self.isLoading = false
                self.wantsToPlay = false
                self.errorMessage = "Couldn't play audio. Try again."
            }
        }
    }

    func pause() {
        wantsToPlay = false
        loadID = UUID()
        loadTask?.cancel()
        loadTask = nil
        activationTask?.cancel()
        activationTask = nil
        player?.pause()
        isPlaying = false
        isLoading = false
    }

    func stop() {
        pause()
        releasePlayer()
        elapsed = 0
    }

    func seek(to seconds: TimeInterval) {
        guard let player, duration > 0 else { return }
        let position = min(duration, Self.validTime(seconds))
        elapsed = position
        player.seek(to: CMTime(seconds: position, preferredTimescale: 600),
                    toleranceBefore: .zero, toleranceAfter: .zero)
    }

    private var canPlay: Bool {
        !IrisAudioActivity.isCallActive &&
            !IrisAudioActivity.isRecordingActive &&
            UIApplication.shared.applicationState == .active
    }

    private func installPlayer(asset: AVURLAsset) {
        let item = AVPlayerItem(asset: asset)
        let player = AVPlayer(playerItem: item)
        self.player = player
        itemObservation = item.observe(\.status, options: [.initial, .new]) { [weak self] item, _ in
            Task { @MainActor [weak self] in
                guard let self, self.player?.currentItem === item else { return }
                switch item.status {
                case .readyToPlay:
                    self.isLoading = false
                    if self.wantsToPlay { self.startPreparedPlayer() }
                case .failed:
                    self.playbackFailed()
                default:
                    break
                }
            }
        }
        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(seconds: 0.25, preferredTimescale: 600), queue: .main
        ) { [weak self, weak player] time in
            Task { @MainActor [weak self, weak player] in
                guard let self, let player, self.player === player else { return }
                self.elapsed = min(self.duration, Self.validTime(time.seconds))
            }
        }
    }

    private func startPreparedPlayer() {
        guard canPlay, wantsToPlay, let player else { pause(); return }
        guard activationTask == nil else { return }
        isLoading = true
        let requestID = loadID
        activationTask = Task { @MainActor [weak self, weak player] in
            do {
                let available = try await Self.activatePlaybackSession()
                guard let self, let player, self.player === player,
                      self.loadID == requestID, !Task.isCancelled else { return }
                self.activationTask = nil
                guard available, self.canPlay, self.wantsToPlay else { self.pause(); return }
                if self.duration > 0, self.elapsed >= self.duration - 0.05 { self.seek(to: 0) }
                player.play()
                self.isPlaying = true
                self.isLoading = false
            } catch {
                guard let self, self.loadID == requestID, !Task.isCancelled else { return }
                self.activationTask = nil
                self.playbackFailed()
            }
        }
    }

    private func playbackFailed() {
        pause()
        releasePlayer()
        errorMessage = "Couldn't play audio. Try again."
    }

    private func releasePlayer() {
        itemObservation = nil
        if let timeObserver { player?.removeTimeObserver(timeObserver) }
        timeObserver = nil
        player?.pause()
        player = nil
        if let temporaryURL { Self.removeTemporaryAudio(temporaryURL) }
        temporaryURL = nil
    }

    private func observeAudioLifecycle() {
        let center = NotificationCenter.default
        for name in [UIApplication.willResignActiveNotification, AVAudioSession.interruptionNotification,
                     IrisAudioActivity.callDidChange] {
            notifications.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor [weak self] in self?.pause() }
            })
        }
        notifications.append(center.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: nil, queue: .main) {
            [weak self] notification in
            Task { @MainActor [weak self] in
                guard let self, let item = notification.object as? AVPlayerItem,
                      self.player?.currentItem === item else { return }
                self.pause()
                self.elapsed = self.duration
            }
        })
        notifications.append(center.addObserver(forName: .AVPlayerItemFailedToPlayToEndTime, object: nil, queue: .main) {
            [weak self] notification in
            Task { @MainActor [weak self] in
                guard let self, let item = notification.object as? AVPlayerItem,
                      self.player?.currentItem === item else { return }
                self.playbackFailed()
            }
        })
    }

    private nonisolated static func validTime(_ time: TimeInterval) -> TimeInterval {
        time.isFinite ? max(0, time) : 0
    }

    private nonisolated static func activatePlaybackSession() async throws -> Bool {
        let activation = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            // Serialize only the audio-session transition with call
            // ownership; OS activation must not stall UI rendering.
            return try IrisAudioActivity.withPlaybackSessionIfAvailable {
                try Task.checkCancellation()
                let session = AVAudioSession.sharedInstance()
                try session.setCategory(.playback, mode: .default)
                try Task.checkCancellation()
                try session.setActive(true)
                return true
            } ?? false
        }
        return try await withTaskCancellationHandler {
            try await activation.value
        } onCancel: {
            activation.cancel()
        }
    }

    private nonisolated static func writeTemporaryAudio(_ data: Data, filename: String) async throws -> URL {
        let write = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let suffix = URL(fileURLWithPath: filename).pathExtension.lowercased()
            let fileExtension = chatAudioExtensions.contains(suffix) ? suffix : "audio"
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("iris-audio-\(UUID().uuidString).\(fileExtension)")
            do {
                try data.write(to: url, options: [.atomic, .completeFileProtectionUnlessOpen])
                try Task.checkCancellation()
                return url
            } catch {
                try? FileManager.default.removeItem(at: url)
                throw error
            }
        }
        return try await withTaskCancellationHandler {
            try await write.value
        } onCancel: {
            write.cancel()
        }
    }

    private nonisolated static func removeTemporaryAudio(_ url: URL) {
        Task.detached(priority: .utility) { try? FileManager.default.removeItem(at: url) }
    }

    private enum AudioLoadError: Error { case unavailable }
}

@MainActor
struct IrisAudioPlaybackControl: View {
    @Environment(\.irisPalette) private var palette
    @StateObject private var playback: IrisAudioPlayback
    private let foreground: Color?

    init(localURL: URL, duration: TimeInterval = 0) {
        _playback = StateObject(wrappedValue: IrisAudioPlayback(localURL: localURL, duration: duration))
        foreground = nil
    }

    init(filename: String, foreground: Color, loadData: @escaping () async -> Data?) {
        _playback = StateObject(wrappedValue: IrisAudioPlayback(filename: filename, loadData: loadData))
        self.foreground = foreground
    }

    var body: some View {
        HStack(spacing: 10) {
            Button { playback.toggle() } label: {
                Group {
                    if playback.isLoading { ProgressView().tint(color) }
                    else {
                        Image(systemName: playback.errorMessage != nil ? "arrow.clockwise" :
                            (playback.isPlaying ? "pause.fill" : "play.fill"))
                            .font(.system(size: 18, weight: .semibold))
                    }
                }
                .frame(width: 44, height: 44)
                .background(color.opacity(0.12), in: Circle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel(playback.isLoading ? "Cancel loading" :
                (playback.errorMessage != nil ? "Retry audio" : (playback.isPlaying ? "Pause audio" : "Play audio")))
            .accessibilityIdentifier("chatAudioPlayButton")

            VStack(alignment: .leading, spacing: 0) {
                Slider(value: Binding(get: { playback.elapsed }, set: { playback.seek(to: $0) }),
                       in: 0...max(1, playback.duration))
                    .tint(color)
                    .disabled(playback.duration <= 0 || playback.isLoading)
                    .accessibilityLabel("Audio position")
                    .accessibilityValue("\(time(playback.elapsed)) of \(time(playback.duration))")
                    .accessibilityIdentifier("chatAudioProgress")
                Text(playback.errorMessage ?? durationLabel)
                    .font(.system(.caption, design: .rounded))
                    .monospacedDigit()
                    .foregroundStyle(color.opacity(0.7))
                    .lineLimit(2)
                    .accessibilityIdentifier("chatAudioDuration")
            }
        }
        .foregroundStyle(color)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatAudioPlayer")
        .onDisappear { playback.stop() }
    }

    private var color: Color { foreground ?? palette.textPrimary }

    private var durationLabel: String {
        playback.duration > 0 ? "\(time(playback.elapsed)) / \(time(playback.duration))" : "Audio"
    }

    private func time(_ seconds: TimeInterval) -> String {
        let value = Int(exactly: floor(max(0, seconds))) ?? 0
        return "\(value / 60):" + String(format: "%02d", value % 60)
    }
}

@MainActor
struct IrisAudioMessagePlayer: View {
    @Environment(\.irisPalette) private var palette
    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?

    var body: some View {
        IrisAudioPlaybackControl(filename: attachment.filename,
                                 foreground: isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs) {
            await downloadAttachment(attachment)
        }
        .id(attachment.htreeUrl)
        .frame(width: 224)
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }
}
#endif
