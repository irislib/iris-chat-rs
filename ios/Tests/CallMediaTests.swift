import AVFoundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

private final class CallAudioProbe: IrisCallAudioHandling {
    var started = false
    var muted = false
    var error: Error?
    var stopped: (() -> Void)?
    func start() throws { if let error { throw error }; started = true }
    func setMuted(_ muted: Bool) { self.muted = muted }
    func receive(sequence: UInt32, data: Data) {}
    func stop() { stopped?() }
}

final class CallMediaTests: XCTestCase {
    func testMutedVoiceCallIsReadyAfterLocalAudioInitialization() {
        let ready = expectation(description: "muted voice call ready without incoming media")
        let stopped = expectation(description: "audio cleaned up")
        let audio = CallAudioProbe()
        audio.stopped = { stopped.fulfill() }
        let engine = IrisCallMediaEngine(send: { _, _, _, _, _, _ in XCTFail("muted call cannot capture") },
            frame: { _, _, _ in XCTFail("voice call cannot capture video") },
            connectionChanged: { id, connected in
                XCTAssertEqual(id, "muted"); XCTAssertTrue(connected); ready.fulfill()
            }, requestKeyFrame: { _ in }, failed: { _, message in XCTFail(message) }, audioForTesting: audio)
        engine.start(IrisCallMediaSession(callID: "muted", videoCapable: false))
        engine.configure(callID: "muted", muted: true, video: false)
        wait(for: [ready], timeout: 2)
        XCTAssertTrue(audio.started); XCTAssertTrue(audio.muted)
        engine.stop()
        wait(for: [stopped], timeout: 2)
        audio.stopped = nil
    }

    func testAudioInitializationFailureDoesNotReportReady() {
        let failed = expectation(description: "audio failure surfaced")
        let audio = CallAudioProbe()
        audio.error = NSError(domain: "test", code: 1)
        let engine = IrisCallMediaEngine(send: { _, _, _, _, _, _ in }, frame: { _, _, _ in },
            connectionChanged: { _, _ in XCTFail("failed audio cannot connect") },
            requestKeyFrame: { _ in }, failed: { _, _ in failed.fulfill() }, audioForTesting: audio)
        engine.start(IrisCallMediaSession(callID: "failed", videoCapable: false))
        engine.configure(callID: "failed", muted: true, video: false)
        wait(for: [failed], timeout: 2)
        engine.stop()
    }

    func testOpusFFIPlaysAudioAcrossReorderingAndLossThenFallsSilent() throws {
        let encoder = try CallAudioCodec()
        let decoder = try CallAudioCodec()
        let packets = try (0..<12).map { frame in
            try encoder.encode(samples: (0..<960).map { index in
                Int16(sin(Double(frame * 960 + index) * 440 * 2 * Double.pi / 48_000) * 12_000)
            })
        }
        XCTAssertTrue(packets.allSatisfy { !$0.isEmpty && $0.count <= 1_275 })
        for index in [0, 2, 1, 3, 4, 6, 7, 8, 9, 10, 11] {
            decoder.queue(sequence: UInt32(index), data: packets[index])
        }
        var audible = 0
        for _ in 0..<22 {
            let samples = decoder.playout()
            XCTAssertEqual(samples.count, 960)
            if samples.contains(where: { abs(Int($0)) > 100 }) { audible += 1 }
        }
        XCTAssertGreaterThan(audible, 6)
        XCTAssertTrue(decoder.playout().allSatisfy { $0 == 0 })
    }

    func testH264DecoderRequiresAKeyFrameAndRejectsMalformedAccessUnits() {
        let decoder = IrisH264Decoder { _ in XCTFail("malformed input must not produce a frame") }
        let invalid = [Data(), Data([1, 2, 3]), Data([0, 0, 1]), Data([0, 0, 1, 0x80]),
                       Data(repeating: 0, count: IrisH264Wire.maximumBytes + 1)]
        for data in invalid { XCTAssertFalse(decoder.decode(data, timestampUs: 1, keyFrame: true)) }
        XCTAssertFalse(decoder.decode(Data([0, 0, 0, 1, 0x41, 1]), timestampUs: 1, keyFrame: false))
        XCTAssertFalse(decoder.decode(Data([0, 0, 0, 1, 0x65, 1]), timestampUs: 1, keyFrame: true))
    }

    func testHardwareEncoderPreservesOriginalCapturePermission() throws {
        let gate = IrisCallSendGate()
        gate.update(callID: "video", muted: false, video: true)
        let stale = gate.permission(callID: "video", kind: 2)
        gate.update(callID: "video", muted: false, video: false)
        gate.update(callID: "video", muted: false, video: true)
        var outputCount = 0
        var queuedPermission: (() -> Bool)?
        let encoder = IrisH264Encoder { _, _, _, allowed in
            outputCount += 1
            queuedPermission = allowed
        }
        var pixel: CVPixelBuffer?
        XCTAssertEqual(CVPixelBufferCreate(kCFAllocatorDefault, 1280, 720, kCVPixelFormatType_32BGRA,
            [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &pixel), kCVReturnSuccess)
        let buffer = try XCTUnwrap(pixel)
        CVPixelBufferLockBaseAddress(buffer, [])
        memset(CVPixelBufferGetBaseAddress(buffer), 128, CVPixelBufferGetDataSize(buffer))
        CVPixelBufferUnlockBaseAddress(buffer, [])
        try encoder.encode(buffer, timestampUs: 1, isCurrent: stale)
        encoder.stop()
        XCTAssertEqual(outputCount, 0)
        try encoder.encode(buffer, timestampUs: 2, isCurrent: gate.permission(callID: "video", kind: 2))
        encoder.stop()
        XCTAssertEqual(outputCount, 1)
        XCTAssertEqual(queuedPermission?(), true)
        gate.update(callID: "video", muted: false, video: false)
        gate.update(callID: "video", muted: false, video: true)
        XCTAssertEqual(queuedPermission?(), false)
    }

    func testHardwareH264AccessUnitIncludesRecoveryHeadersAndDecodesAt720p() throws {
        let decoded = expectation(description: "hardware encoded access unit decodes")
        let queue = DispatchQueue(label: "iris.codec.test")
        let decoder = IrisH264Decoder { pixel in
            XCTAssertEqual(CVPixelBufferGetWidth(pixel), 1280)
            XCTAssertEqual(CVPixelBufferGetHeight(pixel), 720)
            decoded.fulfill()
        }
        let encoder = IrisH264Encoder { data, timestamp, key, _ in
            XCTAssertTrue(key)
            XCTAssertLessThanOrEqual(data.count, IrisH264Wire.maximumBytes)
            let units = IrisH264Wire.units(data) ?? []
            XCTAssertTrue(units.contains { $0.first.map { $0 & 31 == 7 } == true })
            XCTAssertTrue(units.contains { $0.first.map { $0 & 31 == 8 } == true })
            queue.async { XCTAssertTrue(decoder.decode(data, timestampUs: timestamp, keyFrame: key)) }
        }
        var pixel: CVPixelBuffer?
        XCTAssertEqual(CVPixelBufferCreate(kCFAllocatorDefault, 1280, 720, kCVPixelFormatType_32BGRA,
            [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &pixel), kCVReturnSuccess)
        let buffer = try XCTUnwrap(pixel)
        CVPixelBufferLockBaseAddress(buffer, [])
        memset(CVPixelBufferGetBaseAddress(buffer), 128, CVPixelBufferGetDataSize(buffer))
        CVPixelBufferUnlockBaseAddress(buffer, [])
        try encoder.encode(buffer, timestampUs: 1_000_000)
        encoder.stop()
        wait(for: [decoded], timeout: 5)
        queue.sync { decoder.stop() }
    }
}
