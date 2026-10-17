#if DEBUG
import AVFoundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class CallAudioContinuityProbeTests: XCTestCase {
    private let env = ["IRIS_CALL_CONTINUITY_PROBE": "1", "IRIS_IOS_HARNESS_ACTION": "call_audio_continuity",
        "IRIS_IOS_HARNESS_RUN_ID": "audio-continuity-fixture12", "IRIS_UI_TEST_RUN_ID": "audio-continuity-fixture12-shell"]

    private func count(_ probe: IrisCallAudioContinuityProbe, _ stage: IrisCallAudioContinuityProbe.Stage) -> Int64? {
        (probe.snapshot()["counts"] as? [String: Int64])?[stage.rawValue]
    }

    func testExplicitIsolatedOptInAndOneRunPerProcess() throws {
        XCTAssertFalse(IrisCallAudioContinuityProbe.accepts([:]))
        XCTAssertTrue(IrisCallAudioContinuityProbe.accepts(env))
        for key in ["IRIS_IOS_HARNESS_USE_APP_STORAGE", "IRIS_IOS_HARNESS_USE_KEYCHAIN", "IRIS_UI_TEST_RESET",
                    "IRIS_UI_TEST_DATA_DIR", "IRIS_IOS_HARNESS_SERVICE"] {
            var changed = env; changed[key] = "1"
            XCTAssertFalse(IrisCallAudioContinuityProbe.accepts(changed), key)
        }
        var changed = env; changed["IRIS_UI_TEST_RUN_ID"] = "ordinary"
        XCTAssertFalse(IrisCallAudioContinuityProbe.accepts(changed))
        let probe = IrisCallAudioContinuityProbe()
        XCTAssertThrowsError(try probe.begin(environment: [:], startTimer: false))
        try probe.begin(environment: env, startTimer: false)
        XCTAssertThrowsError(try probe.begin(environment: env, startTimer: false))
        _ = probe.finish()
        XCTAssertFalse(probe.enabled)
    }

    func testWatchdogDetectsNoCallbacksIndependentlyAndHasAHardEnd() throws {
        var now: UInt64 = 1_000_000_000
        let probe = IrisCallAudioContinuityProbe(now: { now })
        try probe.begin(environment: env, duration: 15, startTimer: false)
        now += 500_000_000; probe.tick()
        XCTAssertEqual(count(probe, .renderMissingTicks), 0, "No render claim before the engine is active")
        probe.event(.engineGenerations)
        now += 250_000_000; probe.tick()
        XCTAssertEqual(count(probe, .renderMissingTicks), 1)
        XCTAssertEqual(probe.snapshot()["maxWatchdogRenderGapNs"] as? Int64, 250_000_000)
        probe.event(.engineStopped)
        now += 250_000_000; probe.tick()
        XCTAssertEqual(count(probe, .renderMissingTicks), 1, "Stopping intentionally is not an output stall")
        now += 20_000_000_000; probe.tick()
        XCTAssertFalse(probe.enabled)
        probe.event(.encodedFrames)
        XCTAssertEqual(count(probe, .encodedFrames), 0)
        let report = probe.finish()
        XCTAssertEqual(report["externalSpeakerAudibilityProven"] as? Bool, false)
        XCTAssertEqual(report["secondPhoneCellularProven"] as? Bool, false)
    }

    func testActualPCMObservationSeparatesCaptureAndRenderAndInvalidatesTruncation() throws {
        let probe = IrisCallAudioContinuityProbe()
        try probe.begin(environment: env, startTimer: false)
        let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1))
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 960))
        buffer.frameLength = 960
        let channel = try XCTUnwrap(buffer.floatChannelData?[0])
        for i in 0..<960 { channel[i] = Float(sin(Double(i) * 0.037) * (0.1 + Double(i) / 9600)) }
        let time = AVAudioTime(hostTime: 1_000_000_000)
        probe.pcm(buffer, time: time, render: false)
        XCTAssertGreaterThan(try XCTUnwrap(count(probe, .captureEnergyMicro)), 0)
        XCTAssertEqual(count(probe, .renderSamples), 0)
        probe.pcm(buffer, time: time, render: true)
        XCTAssertEqual(count(probe, .renderSamples), 960)
        XCTAssertGreaterThan(try XCTUnwrap(count(probe, .renderEnergyMicro)), 0)
        XCTAssertEqual(probe.snapshot()["observationsComplete"] as? Bool, true)
        let oversized = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 8193))
        oversized.frameLength = 8193
        probe.pcm(oversized, time: time, render: false)
        XCTAssertEqual(count(probe, .truncatedPCM), 1)
        XCTAssertEqual(probe.snapshot()["observationsComplete"] as? Bool, false)
        _ = probe.finish()
    }

    func testContinuousCallbacksStillExposeContiguousSilentPCMAtMixerAndPlayout() throws {
        let probe = IrisCallAudioContinuityProbe()
        try probe.begin(environment: env, startTimer: false)
        let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1))
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 960))
        buffer.frameLength = 960
        let channel = try XCTUnwrap(buffer.floatChannelData?[0])
        let start = ProcessInfo.processInfo.systemUptime
        for index in 0..<61 {
            let value: Int16 = index == 40 ? 8_000 : 0
            for sample in 0..<960 { channel[sample] = Float(value) / 32768 }
            probe.pcm(buffer, time: AVAudioTime(hostTime: AVAudioTime.hostTime(forSeconds: start + Double(index) * 0.02)), render: true)
            probe.decoded(Array(repeating: value, count: 960))
        }
        let snapshot = probe.snapshot()
        let mixer = try XCTUnwrap(snapshot["mixerPCM"] as? [String: Any])
        let playout = try XCTUnwrap(snapshot["playoutPCM"] as? [String: Any])
        XCTAssertEqual(count(probe, .renderCallbacks), 61)
        XCTAssertLessThan(try XCTUnwrap(snapshot["maxRenderHostGapNs"] as? Int64), 21_000_000)
        for coverage in [mixer, playout] {
            XCTAssertEqual(coverage["sampleRateMilliHz"] as? Int64, 48_000_000)
            XCTAssertEqual(coverage["silentFrames"] as? Int64, 60 * 960)
            XCTAssertEqual(coverage["voicedFrames"] as? Int64, 960)
            XCTAssertEqual(coverage["maximumSilentDurationNs"] as? Int64, 800_000_000)
            XCTAssertEqual(coverage["timelineDiscontinuities"] as? Int64, 0)
            let first = try XCTUnwrap(coverage["maximumSilentStartNs"] as? Int64)
            let last = try XCTUnwrap(coverage["maximumSilentEndNs"] as? Int64)
            XCTAssertEqual(last - first, 800_000_000)
        }
        XCTAssertEqual(mixer["clock"] as? String, "buffer-host-time")
        XCTAssertEqual(playout["clock"] as? String, "scheduled-sample-clock")
        _ = probe.finish()
    }

    func testHostTimeGapsAndMeasurementLossRemainVisible() throws {
        let probe = IrisCallAudioContinuityProbe()
        try probe.begin(environment: env, startTimer: false)
        probe.renderAt(1_000_000_000); probe.renderAt(1_020_000_000); probe.renderAt(6_020_000_000)
        XCTAssertEqual(probe.snapshot()["maxRenderHostGapNs"] as? Int64, 5_000_000_000)
        probe.renderAt(1)
        XCTAssertEqual(count(probe, .invalidHostTime), 1)
        probe.event(.atomicObservationLost)
        XCTAssertEqual(probe.snapshot()["observationsComplete"] as? Bool, false)
        _ = probe.finish()
    }

    func testProductionCaptureQueueReportsExactOverloadAndOriginalDeadlineDrops() throws {
        var now: UInt64 = 1_000_000_000
        let probe = IrisCallAudioContinuityProbe(now: { now })
        try probe.begin(environment: env, startTimer: false)
        let queue = DispatchQueue(label: "test.continuity.capture")
        let capture = IrisCallAudioCaptureQueue(queue: queue, now: { now })
        capture.continuityProbe = probe
        let frames = Array(repeating: Array(repeating: Int16(12), count: 960), count: 5)
        queue.suspend()
        for _ in 0..<3 { capture.submit(frames, timestampUs: 0) { _, _ in XCTFail("Expired frame sent") } }
        now += 101_000_000
        queue.resume(); queue.sync {}
        XCTAssertEqual(count(probe, .captureBatches), 3)
        XCTAssertEqual(count(probe, .captureBatchFrames), 15)
        XCTAssertEqual(count(probe, .captureBusyBatches), 1)
        XCTAssertEqual(count(probe, .captureExpiredBatches), 2)
        XCTAssertEqual(count(probe, .captureExpiredFrames), 10)
        var delivered = 0
        capture.submit(frames, timestampUs: 0) { _, _ in delivered += 1 }
        queue.sync {}
        XCTAssertEqual(delivered, 5, "Instrumentation must not change original recovery or deadline")
        _ = probe.finish()
    }

    func testConcurrentEventsHaveFixedStorageAndExactCounts() throws {
        let probe = IrisCallAudioContinuityProbe()
        try probe.begin(environment: env, startTimer: false)
        DispatchQueue.concurrentPerform(iterations: 8) { _ in
            for _ in 0..<1000 { probe.event(.receivePackets) }
        }
        XCTAssertEqual(count(probe, .receivePackets), 8000)
        XCTAssertEqual(count(probe, .atomicObservationLost), 0)
        _ = probe.finish()
    }
}
#endif
