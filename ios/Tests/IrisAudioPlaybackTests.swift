#if os(iOS)
import Combine
import XCTest
@testable import IrisChat

final class IrisAudioPlaybackTests: XCTestCase {
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
