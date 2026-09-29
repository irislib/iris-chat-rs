#if os(macOS)
import AVFoundation
import XCTest
@testable import IrisChatMac

private final class ScreenShareAudioProbe: IrisCallAudioHandling {
    var stops = 0
    func start() throws {}
    func setMuted(_ muted: Bool) {}
    func receive(sequence: UInt32, data: Data) {}
    func setDevices(_ selection: IrisMacAudioDeviceSelection) throws {}
    func stop() { stops += 1 }
}

final class ScreenShareMediaTests: XCTestCase {
    func testScreenUsesH264AndRecoversUnchangedFrameWithoutRestartingAudio() throws {
        let ready = expectation(description: "audio ready")
        let first = expectation(description: "screen decoded")
        let recovery = expectation(description: "unchanged screen keyframe decoded")
        let drained = expectation(description: "media queue drained")
        let audio = ScreenShareAudioProbe()
        let decodeQueue = DispatchQueue(label: "screen.test.decode")
        var decoded = 0
        let decoder = IrisH264Decoder { pixel, _ in
            XCTAssertEqual(CVPixelBufferGetWidth(pixel), 1280)
            XCTAssertEqual(CVPixelBufferGetHeight(pixel), 720)
            decoded += 1
            if decoded == 1 { first.fulfill() } else if decoded == 2 { recovery.fulfill() }
            else { XCTFail("Stopped screen source sent another frame") }
        }
        var readyCount = 0
        let engine = IrisCallMediaEngine(send: { id, kind, timestamp, key, data, allowed in
            XCTAssertEqual(id, "screen"); XCTAssertEqual(kind, 2); XCTAssertTrue(key)
            guard allowed() else { return }
            decodeQueue.async { XCTAssertTrue(decoder.decode(data, timestampUs: timestamp, keyFrame: key)) }
        }, frame: { _, _, _ in }, connectionChanged: { _, _ in readyCount += 1; ready.fulfill() },
            requestKeyFrame: { _ in }, failed: { _, message in XCTFail(message) }, audioForTesting: audio)
        engine.start(.init(callID: "screen", videoCapable: true))
        engine.configure(callID: "screen", muted: true, video: false)
        wait(for: [ready], timeout: 2)
        engine.setScreenSharing(true)
        engine.configure(callID: "screen", muted: true, video: true)
        var pixel: CVPixelBuffer?
        XCTAssertEqual(CVPixelBufferCreate(kCFAllocatorDefault, 1280, 720, kCVPixelFormatType_32BGRA,
            [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &pixel), kCVReturnSuccess)
        let buffer = try XCTUnwrap(pixel)
        CVPixelBufferLockBaseAddress(buffer, [])
        memset(CVPixelBufferGetBaseAddress(buffer), 128, CVPixelBufferGetDataSize(buffer))
        CVPixelBufferUnlockBaseAddress(buffer, [])
        engine.captureScreenFrame(callID: "screen", pixel: buffer, timestamp: UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000))
        wait(for: [first], timeout: 5)
        engine.adapt(targetBitrate: 2_000_000, keyFrameGeneration: 1)
        wait(for: [recovery], timeout: 5)
        engine.setScreenSharing(false)
        engine.configure(callID: "screen", muted: true, video: false)
        engine.captureScreenFrame(callID: "screen", pixel: buffer, timestamp: UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000))
        engine.setAudioDevices(.init()) { _ in drained.fulfill() }
        wait(for: [drained], timeout: 2)
        decodeQueue.sync { XCTAssertEqual(decoded, 2); decoder.stop() }
        XCTAssertEqual(readyCount, 1)
        XCTAssertEqual(audio.stops, 0, "Screen source changes must leave call audio running")
        engine.stop()
    }
}
#endif
