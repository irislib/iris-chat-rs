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
    func testDownloadStartsOnlyOnPlayAndRetriesFailure() async {
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
#endif
