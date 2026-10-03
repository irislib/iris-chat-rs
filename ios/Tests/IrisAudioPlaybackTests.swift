#if os(iOS) || os(macOS)
import AVFoundation
import Combine
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class IrisAudioPlaybackTests: XCTestCase {
    @MainActor
    func testDeferredPreviewDoesNotDownloadUntilPlayAndStillWorksOffline() async throws {
        let url = try await silentM4A(varying: true)
        defer { try? FileManager.default.removeItem(at: url) }
        let bytes = try Data(contentsOf: url)
        var downloads = 0
        let playback = IrisAudioPlayback(filename: "large.m4a", loadPreviewData: { nil }) {
            downloads += 1
            return bytes
        }
        defer { playback.stop() }
        let deferred = expectation(description: "preview waits for an explicit tap")
        let observation = playback.$requiresDownload.filter { $0 }.prefix(1).sink { _ in deferred.fulfill() }
        playback.prepare()
        await fulfillment(of: [deferred], timeout: 3)
        XCTAssertEqual(downloads, 0)
        XCTAssertFalse(playback.isPlaying)
        XCTAssertNil(playback.errorMessage)
        playback.play()
        await waitForPlayback(playback)
        XCTAssertEqual(downloads, 1)
        XCTAssertFalse(playback.requiresDownload)
        playback.pause()
        playback.play()
        await waitForPlayback(playback)
        XCTAssertEqual(downloads, 1, "A downloaded clip must play without another network request")
        observation.cancel()
    }

    @MainActor
    func testPlayUpgradesPreviewThatIsRejectedWhileLoading() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        let bytes = try Data(contentsOf: url)
        let started = expectation(description: "preview starts")
        var continuation: CheckedContinuation<Data?, Never>?
        var downloads = 0
        let playback = IrisAudioPlayback(filename: "large.m4a", loadPreviewData: {
            await withCheckedContinuation { pending in continuation = pending; started.fulfill() }
        }) {
            downloads += 1
            return bytes
        }
        defer { playback.stop() }
        playback.prepare()
        await fulfillment(of: [started], timeout: 3)
        playback.play()
        continuation?.resume(returning: nil)
        await waitForPlayback(playback)
        XCTAssertEqual(downloads, 1)
        XCTAssertNil(playback.errorMessage)
    }

    @MainActor
    func testPreparationShowsWaveformAndAllowsSeekingWithoutPlayback() async throws {
        let url = try await silentM4A(varying: true)
        defer { try? FileManager.default.removeItem(at: url) }
        let bytes = try Data(contentsOf: url)
        var downloads = 0
        let playback = IrisAudioPlayback(filename: "Voice message.m4a") {
            downloads += 1
            return bytes
        }
        defer { playback.stop() }
        playback.prepare()
        await waitForWaveform(playback)
        XCTAssertEqual(playback.duration, 4, accuracy: 0.15)
        XCTAssertFalse(playback.isPlaying)
        XCTAssertFalse(playback.isLoading)
        XCTAssertFalse(playback.isPreparing)
        XCTAssertNil(playback.errorMessage)
        playback.seek(to: 1.5)
        XCTAssertEqual(playback.elapsed, 1.5, accuracy: 0.01)
        XCTAssertFalse(playback.isPlaying)
        playback.play()
        await waitForPlayback(playback)
        XCTAssertEqual(downloads, 1, "Play must reuse the prepared attachment")
        XCTAssertGreaterThanOrEqual(playback.elapsed, 1.4)
    }

    @MainActor
    func testPreparationDoesNotInterruptAnotherMessage() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        let playing = IrisAudioPlayback(localURL: url)
        let preview = IrisAudioPlayback(localURL: url)
        defer { playing.stop(); preview.stop() }
        playing.play()
        await waitForPlayback(playing)
        preview.prepare()
        await waitForWaveform(preview)
        XCTAssertTrue(playing.isPlaying)
        XCTAssertFalse(preview.isPlaying)
    }

    @MainActor
    func testPlayJoinsPendingPreparation() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        let bytes = try Data(contentsOf: url)
        let started = expectation(description: "preview download starts")
        var continuation: CheckedContinuation<Data?, Never>?
        var downloads = 0
        let playback = IrisAudioPlayback(filename: "voice.m4a") {
            downloads += 1
            return await withCheckedContinuation { pending in
                continuation = pending
                started.fulfill()
            }
        }
        defer { playback.stop() }
        playback.prepare()
        await fulfillment(of: [started], timeout: 2)
        XCTAssertFalse(playback.isLoading)
        playback.play()
        XCTAssertTrue(playback.isLoading)
        continuation?.resume(returning: bytes)
        await waitForPlayback(playback)
        XCTAssertEqual(downloads, 1)
    }

    @MainActor
    private func waitForWaveform(_ playback: IrisAudioPlayback) async {
        let prepared = expectation(description: "waveform ready without playback")
        let observation = playback.$isPreparing.dropFirst().filter { !$0 }.prefix(1).sink { _ in prepared.fulfill() }
        await fulfillment(of: [prepared], timeout: 5)
        XCTAssertEqual(playback.waveform.count, 47)
        observation.cancel()
    }

    @MainActor
    func testM4AAttachmentPlaysPausesSeeksAndReplaysInApp() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        let bytes = try Data(contentsOf: url)
        var downloads = 0
        let playback = IrisAudioPlayback(filename: "Voice message.m4a") {
            downloads += 1
            return bytes
        }
        defer { playback.stop() }
        playback.play()
        await waitForPlayback(playback)
        XCTAssertEqual(playback.duration, 4, accuracy: 0.15)
        XCTAssertNil(playback.errorMessage)
        XCTAssertEqual(playback.waveform.count, 47)

        playback.pause()
        XCTAssertFalse(playback.isPlaying)
        playback.seek(to: 1.5)
        XCTAssertEqual(playback.elapsed, 1.5, accuracy: 0.01)
        playback.play()
        await waitForPlayback(playback)
        XCTAssertEqual(downloads, 1, "Resuming must reuse the decrypted attachment")

        playback.seek(to: playback.duration)
        playback.pause()
        playback.play()
        await waitForPlayback(playback)
        XCTAssertLessThan(playback.elapsed, 1, "Playing a finished message must restart it")
        XCTAssertEqual(downloads, 1)
    }

    @MainActor
    func testReplayRewindsTheMediaBeforePublishingPlayback() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        var mediaPlayer: HeldSeekPlayer?
        let playback = IrisAudioPlayback(localURL: url, makePlayer: { item in
            let player = HeldSeekPlayer(playerItem: item)
            mediaPlayer = player
            return player
        })
        defer { playback.stop(); mediaPlayer?.finishAllSeeks() }
        playback.play()
        await waitForPlayback(playback)
        let player = try XCTUnwrap(mediaPlayer)
        playback.pause()
        playback.seek(to: playback.duration)
        // Wait for the real media timeline to reach the end before replaying.
        // Checking only the optimistic published elapsed value hides this race.
        let atEnd = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            abs(player.currentTime().seconds - playback.duration) < 0.05
        }, object: nil)
        await fulfillment(of: [atEnd], timeout: 5)
        XCTAssertEqual(player.currentTime().seconds, playback.duration, accuracy: 0.05)

        let rewindRequested = expectation(description: "replay requests a rewind")
        player.holdRequests = true
        player.didHoldRequest = { rewindRequested.fulfill() }
        let replayed = expectation(description: "replay starts from the beginning")
        let observation = playback.$isPlaying.filter { $0 }.prefix(1).sink { _ in
            XCTAssertLessThan(player.currentTime().seconds, 1, "The media itself must rewind before playback is announced")
            XCTAssertLessThan(playback.elapsed, 1)
            replayed.fulfill()
        }
        playback.play()
        await fulfillment(of: [rewindRequested], timeout: 5)
        XCTAssertFalse(playback.isPlaying, "Replay must wait for the media seek")
        player.finishSeekRequests()
        await fulfillment(of: [replayed], timeout: 5)
        observation.cancel()
    }

    @MainActor
    func testPauseStopAndAnotherMessageCancelPendingReplay() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        for action in ["pause", "stop", "another message"] {
            var mediaPlayer: HeldSeekPlayer?
            let playback = IrisAudioPlayback(localURL: url, makePlayer: { item in
                let player = HeldSeekPlayer(playerItem: item)
                mediaPlayer = player
                return player
            })
            let other = IrisAudioPlayback(localURL: url)
            defer { playback.stop(); other.stop(); mediaPlayer?.finishAllSeeks() }
            playback.play()
            await waitForPlayback(playback)
            let player = try XCTUnwrap(mediaPlayer)
            playback.pause()
            playback.seek(to: playback.duration)
            let atEnd = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                player.currentTime().seconds >= playback.duration - 0.05
            }, object: nil)
            await fulfillment(of: [atEnd], timeout: 5)
            player.holdCompletions = true
            let rewinding = expectation(description: "replay seek is pending for \(action)")
            player.didHoldSeek = { rewinding.fulfill() }
            playback.play()
            await fulfillment(of: [rewinding], timeout: 5)
            XCTAssertFalse(playback.isPlaying)
            XCTAssertTrue(playback.isLoading)
            switch action {
            case "pause": playback.pause()
            case "stop": playback.stop()
            default: other.play(); await waitForPlayback(other)
            }
            let resumed = expectation(description: "cancelled replay must not resume")
            resumed.isInverted = true
            let observation = playback.$isPlaying.filter { $0 }.sink { _ in resumed.fulfill() }
            player.finishAllSeeks()
            await fulfillment(of: [resumed], timeout: 0.1)
            XCTAssertFalse(playback.isPlaying)
            XCTAssertFalse(playback.isLoading)
            observation.cancel()
        }
    }

    @MainActor
    func testNewSeekAndPlaybackRateSupersedePendingReplay() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        var mediaPlayer: HeldSeekPlayer?
        let playback = IrisAudioPlayback(localURL: url, makePlayer: { item in
            let player = HeldSeekPlayer(playerItem: item)
            mediaPlayer = player
            return player
        })
        defer { playback.stop(); mediaPlayer?.finishAllSeeks() }
        playback.play()
        await waitForPlayback(playback)
        let player = try XCTUnwrap(mediaPlayer)
        playback.pause()
        playback.seek(to: playback.duration)
        let atEnd = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            player.currentTime().seconds >= playback.duration - 0.05
        }, object: nil)
        await fulfillment(of: [atEnd], timeout: 5)
        player.holdCompletions = true
        let rewinding = expectation(description: "replay seek is pending")
        player.didHoldSeek = { rewinding.fulfill() }
        playback.play()
        await fulfillment(of: [rewinding], timeout: 5)
        let scrubbed = expectation(description: "new scrub supersedes rewind")
        player.didHoldSeek = { scrubbed.fulfill() }
        playback.seek(to: 1.5)
        playback.cyclePlaybackRate()
        await fulfillment(of: [scrubbed], timeout: 5)
        // Complete the newer seek before the cancelled older completion arrives.
        let newSeekCompleted = expectation(description: "newer seek publishes its completed position")
        let positionObservation = playback.$elapsed.dropFirst().prefix(1).sink { _ in newSeekCompleted.fulfill() }
        player.finishSeek(at: 1)
        await fulfillment(of: [newSeekCompleted], timeout: 5)
        positionObservation.cancel()
        player.finishSeek(at: 0)
        await waitForPlayback(playback)
        XCTAssertEqual(player.currentTime().seconds, 1.5, accuracy: 0.2)
        XCTAssertEqual(player.rate, 1.5)
    }

    @MainActor
    func testSeekingPlayingAudioToEndStopsAndStaleEndDoesNotStopReplay() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        var mediaPlayer: AVPlayer?
        let playback = IrisAudioPlayback(localURL: url, makePlayer: { item in
            let player = AVPlayer(playerItem: item)
            mediaPlayer = player
            return player
        })
        defer { playback.stop() }
        playback.play()
        await waitForPlayback(playback)
        let stopped = expectation(description: "seeking to end stops active playback")
        let stopObservation = playback.$isPlaying.filter { !$0 }.prefix(1).sink { _ in stopped.fulfill() }
        playback.seek(to: playback.duration)
        await fulfillment(of: [stopped], timeout: 5)
        stopObservation.cancel()
        playback.play()
        await waitForPlayback(playback)
        let unexpectedlyStopped = expectation(description: "old end notification must not stop replay")
        unexpectedlyStopped.isInverted = true
        let replayObservation = playback.$isPlaying.filter { !$0 }.sink { _ in unexpectedlyStopped.fulfill() }
        NotificationCenter.default.post(name: .AVPlayerItemDidPlayToEndTime,
                                        object: try XCTUnwrap(mediaPlayer?.currentItem))
        await fulfillment(of: [unexpectedlyStopped], timeout: 0.1)
        XCTAssertTrue(playback.isPlaying)
        XCTAssertLessThan(playback.elapsed, 1)
        replayObservation.cancel()
    }

    @MainActor
    func testPlaybackSpeedCyclesWithoutStartingPausedAudio() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        let playback = IrisAudioPlayback(localURL: url)
        defer { playback.stop() }
        for rate: Float in [1.5, 2, 0.5, 1] {
            playback.cyclePlaybackRate()
            XCTAssertEqual(playback.playbackRate, rate)
            XCTAssertFalse(playback.isPlaying)
        }
        playback.play()
        await waitForPlayback(playback)
        playback.cyclePlaybackRate()
        XCTAssertTrue(playback.isPlaying)
        playback.pause()
        playback.play()
        await waitForPlayback(playback)
        XCTAssertEqual(playback.playbackRate, 1.5)
    }

    @MainActor
    func testAnotherMessageAndCallPausePreparedAudio() async throws {
        let url = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: url) }
        let first = IrisAudioPlayback(localURL: url)
        let second = IrisAudioPlayback(localURL: url)
        defer {
            first.stop()
            second.stop()
            IrisAudioActivity.setCallActive(false)
        }
        first.play()
        await waitForPlayback(first)
        second.play()
        await waitForPlayback(second)
        XCTAssertFalse(first.isPlaying)
        let paused = expectation(description: "call pauses prepared audio")
        let observation = second.$isPlaying.dropFirst().filter { !$0 }.prefix(1).sink { _ in paused.fulfill() }
        IrisAudioActivity.setCallActive(true)
        await fulfillment(of: [paused], timeout: 2)
        XCTAssertFalse(second.isPlaying)
        observation.cancel()
    }

    @MainActor
    private func waitForPlayback(_ playback: IrisAudioPlayback) async {
        let started = expectation(description: "AVPlayer starts local M4A")
        let observation = playback.$isPlaying.filter { $0 }.prefix(1).sink { _ in started.fulfill() }
        await fulfillment(of: [started], timeout: 5)
        XCTAssertTrue(playback.isPlaying, playback.errorMessage ?? "Audio did not start")
        observation.cancel()
    }

    func testLongAudioSkipsWaveformWithoutPreventingPlaybackPreparation() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("long-waveform-\(UUID().uuidString).caf")
        defer { try? FileManager.default.removeItem(at: url) }
        do {
            let file = try AVAudioFile(forWriting: url, settings: [
                AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 8000,
                AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
            ])
            let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 8000))
            buffer.frameLength = 8000
            buffer.floatChannelData?[0].initialize(repeating: 0, count: 8000)
            for _ in 0..<901 { try file.write(from: buffer) }
        }
        let peaks = await IrisAudioWaveform.decode(url)
        XCTAssertTrue(peaks.isEmpty)
        let playback = await IrisAudioPlayback(localURL: url)
        await MainActor.run { playback.prepare() }
        for _ in 0..<100 {
            if await !playback.isPreparing { break }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        let duration = await playback.duration
        let error = await playback.errorMessage
        XCTAssertEqual(duration, 901, accuracy: 0.1)
        XCTAssertNil(error)
        await playback.stop()
    }

    func testWaveformReflectsDecodedAudioAndSilence() async throws {
        let url = try await silentM4A(varying: true)
        defer { try? FileManager.default.removeItem(at: url) }
        let peaks = await IrisAudioWaveform.decode(url)
        XCTAssertEqual(peaks.count, 47)
        XCTAssertTrue(peaks[3..<10].allSatisfy { $0 < 0.02 })
        XCTAssertTrue(peaks[20..<40].contains { $0 > 0.5 })
        let silent = try await silentM4A()
        defer { try? FileManager.default.removeItem(at: silent) }
        let quiet = await IrisAudioWaveform.decode(silent)
        XCTAssertEqual(quiet.count, 47)
        XCTAssertTrue(quiet.allSatisfy { $0 < 0.02 })
    }

    private func silentM4A(varying: Bool = false) async throws -> URL {
        try await Task.detached {
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("audio-test-\(UUID().uuidString).m4a")
            let file = try AVAudioFile(forWriting: url, settings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100,
                AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 64_000,
            ])
            let frames: AVAudioFrameCount = 44_100 * 4
            let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: frames))
            let channel = try XCTUnwrap(buffer.floatChannelData?[0])
            channel.initialize(repeating: 0, count: Int(frames))
            if varying {
                for i in 44_100..<Int(frames) {
                    let t = Double(i) / 44_100
                    channel[i] = Float(sin(t * 440 * 2 * .pi) * (0.1 + 0.6 * pow(sin(t * 7), 2)))
                }
            }
            buffer.frameLength = frames
            try file.write(from: buffer)
            return url
        }.value
    }

    @MainActor
    func testDownloadIsLazyUntilPreparationOrPlayAndRetriesFailure() async {
        var downloads = 0
        let firstDownload = expectation(description: "first requested download")
        let retryDownload = expectation(description: "retry requested download")
        let playback = IrisAudioPlayback(filename: "voice.m4a") {
            downloads += 1
            if downloads == 1 { firstDownload.fulfill() }
            else { retryDownload.fulfill() }
            return nil
        }

        XCTAssertEqual(downloads, 0)
        XCTAssertFalse(playback.isLoading)
        XCTAssertFalse(playback.isPlaying)

        playback.play()
        await fulfillment(of: [firstDownload], timeout: 2)
        XCTAssertNotNil(playback.errorMessage)
        XCTAssertFalse(playback.isLoading)

        playback.play()
        await fulfillment(of: [retryDownload], timeout: 2)
        XCTAssertEqual(downloads, 2)
        XCTAssertNotNil(playback.errorMessage)
        playback.stop()
    }

    @MainActor
    func testPauseDiscardsDelayedDownloadFailure() async {
        let started = expectation(description: "download started")
        let returned = expectation(description: "download returned after pause")
        var continuation: CheckedContinuation<Data?, Never>?
        let playback = IrisAudioPlayback(filename: "voice.m4a") {
            let data = await withCheckedContinuation { pending in
                continuation = pending
                started.fulfill()
            }
            returned.fulfill()
            return data
        }

        playback.play()
        await fulfillment(of: [started], timeout: 2)
        XCTAssertTrue(playback.isLoading)
        playback.pause()
        continuation?.resume(returning: nil)
        await fulfillment(of: [returned], timeout: 2)

        XCTAssertFalse(playback.isLoading)
        XCTAssertFalse(playback.isPlaying)
        XCTAssertNil(playback.errorMessage)
    }

    @MainActor
    func testCallStopsPendingLoadAndPreventsAnotherDownload() async {
        let originalCallState = IrisAudioActivity.isCallActive
        IrisAudioActivity.setCallActive(false)
        defer { IrisAudioActivity.setCallActive(originalCallState) }
        let started = expectation(description: "download started")
        let paused = expectation(description: "call paused pending playback")
        let returned = expectation(description: "download returned after call")
        var continuation: CheckedContinuation<Data?, Never>?
        var downloads = 0
        let playback = IrisAudioPlayback(filename: "voice.m4a") {
            downloads += 1
            let data = await withCheckedContinuation { pending in
                continuation = pending
                started.fulfill()
            }
            returned.fulfill()
            return data
        }

        playback.play()
        await fulfillment(of: [started], timeout: 2)
        let observation = playback.$isLoading.dropFirst().filter { !$0 }.prefix(1).sink { _ in paused.fulfill() }
        IrisAudioActivity.setCallActive(true)
        await fulfillment(of: [paused], timeout: 2)
        continuation?.resume(returning: nil)
        await fulfillment(of: [returned], timeout: 2)
        XCTAssertFalse(playback.isLoading)
        XCTAssertFalse(playback.isPlaying)
        XCTAssertNil(playback.errorMessage)

        playback.play()
        XCTAssertEqual(downloads, 1)
        XCTAssertNotNil(playback.errorMessage)
        observation.cancel()
        playback.stop()
    }

    @MainActor
    func testPlayingAnotherAudioPausesPendingDownload() async {
        let firstStarted = expectation(description: "first download started")
        let firstReturned = expectation(description: "first download returned")
        let secondStarted = expectation(description: "second download started")
        var continuation: CheckedContinuation<Data?, Never>?
        let first = IrisAudioPlayback(filename: "first.m4a") {
            let data = await withCheckedContinuation { pending in
                continuation = pending
                firstStarted.fulfill()
            }
            firstReturned.fulfill()
            return data
        }
        let second = IrisAudioPlayback(filename: "second.m4a") {
            secondStarted.fulfill()
            return nil
        }

        first.play()
        await fulfillment(of: [firstStarted], timeout: 2)
        second.play()
        await fulfillment(of: [secondStarted], timeout: 2)
        continuation?.resume(returning: nil)
        await fulfillment(of: [firstReturned], timeout: 2)

        XCTAssertFalse(first.isLoading)
        XCTAssertFalse(first.isPlaying)
        XCTAssertNil(first.errorMessage)
        first.stop()
        second.stop()
    }
}

/// Keep the real AVPlayer timeline, but control when its seek acknowledgement
/// reaches playback so cancellation and reordered completions are reproducible.
private final class HeldSeekPlayer: AVPlayer, @unchecked Sendable {
    var holdRequests = false
    var didHoldRequest: (() -> Void)?
    var holdCompletions = false
    var didHoldSeek: (() -> Void)?
    private var completions: [() -> Void] = []
    private var requests: [() -> Void] = []

    override func seek(to time: CMTime, toleranceBefore: CMTime, toleranceAfter: CMTime) {
        if holdRequests {
            requests.append { [weak self] in self?.seek(to: time, toleranceBefore: toleranceBefore, toleranceAfter: toleranceAfter) }
            didHoldRequest?()
        } else {
            super.seek(to: time, toleranceBefore: toleranceBefore, toleranceAfter: toleranceAfter)
        }
    }

    override func seek(to time: CMTime, toleranceBefore: CMTime, toleranceAfter: CMTime,
                       completionHandler: @escaping (Bool) -> Void) {
        if holdRequests {
            requests.append { [weak self] in self?.seek(to: time, toleranceBefore: toleranceBefore, toleranceAfter: toleranceAfter,
                                                      completionHandler: completionHandler) }
            didHoldRequest?()
            return
        }
        super.seek(to: time, toleranceBefore: toleranceBefore, toleranceAfter: toleranceAfter) { [weak self] finished in
            DispatchQueue.main.async {
                guard let self, self.holdCompletions else { completionHandler(finished); return }
                self.completions.append { completionHandler(finished) }
                self.didHoldSeek?()
            }
        }
    }

    func finishSeek(at index: Int) {
        guard completions.indices.contains(index) else { XCTFail("Missing held seek completion"); return }
        completions.remove(at: index)()
    }
    func finishAllSeeks() {
        holdCompletions = false
        finishSeekRequests()
        while !completions.isEmpty { finishSeek(at: 0) }
    }
    func finishSeekRequests() {
        holdRequests = false
        while !requests.isEmpty { requests.removeFirst()() }
    }
}
#endif
