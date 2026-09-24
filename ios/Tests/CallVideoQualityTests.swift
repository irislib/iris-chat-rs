import AVFoundation
import XCTest
#if !CALL_VIDEO_STANDALONE
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif
#endif

/// Runs the real Apple codecs at camera cadence, without a camera or microphone.
/// Unlike single-frame tests, no encoder flush is allowed to make the stream pass.
final class CallVideoQualityTests: XCTestCase {
    func testLiveBitrateReductionChangesEncodedTrafficAndRecovers() throws {
        let pixels = try (0..<2).map(Self.movingFrame)
        let queue = DispatchQueue(label: "iris.call.bitrate.decode")
        let lock = NSLock()
        var packets: [(at: Double, bytes: Int)] = []
        var decoded: [(at: Double, width: Int, height: Int)] = []
        let decoder = IrisH264Decoder { pixel, _ in
            lock.lock(); decoded.append((ProcessInfo.processInfo.systemUptime, CVPixelBufferGetWidth(pixel), CVPixelBufferGetHeight(pixel))); lock.unlock()
        }
        decoder.onFailure = { XCTFail("bitrate switch broke decoding") }
        let encoder = IrisH264Encoder { data, timestamp, key, _ in
            lock.lock(); packets.append((ProcessInfo.processInfo.systemUptime, data.count)); lock.unlock()
            queue.async { XCTAssertTrue(decoder.decode(data, timestampUs: timestamp, keyFrame: key)) }
        }
        defer { encoder.stop(); queue.sync { decoder.stop() } }
        var rates: [Double] = []
        for (phase, target) in [1_500_000, 150_000, 1_500_000].enumerated() {
            encoder.setBitrate(target)
            encoder.requestKeyFrame()
            let start = ProcessInfo.processInfo.systemUptime
            for frame in 0..<180 {
                let wait = start + Double(frame) / 30 - ProcessInfo.processInfo.systemUptime
                if wait > 0 { Thread.sleep(forTimeInterval: wait) }
                try encoder.encode(pixels[frame % 2], timestampUs: UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000))
            }
            Thread.sleep(forTimeInterval: 0.05)
            let end = ProcessInfo.processInfo.systemUptime, begin = end - 3
            lock.lock()
            let bytes = packets.filter { $0.at >= begin && $0.at <= end }.reduce(0) { $0 + $1.bytes }
            let frames = decoded.filter { $0.at >= begin && $0.at <= end }
            let times = [begin] + frames.map { $0.at } + [end]
            lock.unlock()
            let bitrate = Double(bytes * 8) / 3
            let fps = Double(times.count - 2) / 3
            let gap = zip(times, times.dropFirst()).map { ($1 - $0) * 1000 }.max() ?? 3000
            rates.append(bitrate)
            XCTAssertTrue(frames.allSatisfy { $0.width == (target < 200_000 ? 320 : 1280) && $0.height == (target < 200_000 ? 180 : 720) }, "resolution must fall and recover with the live target")
            print("CALL_BITRATE {\"stage\":\"apple_codec\",\"phase\":\(phase),\"target\":\(target),\"actualBps\":\(bitrate),\"decodedFps\":\(fps),\"maxGapMs\":\(gap)}")
            XCTAssertLessThan(bitrate, Double(target) * 1.4 + 20_000, "encoder must honor a live bitrate reduction")
            XCTAssertGreaterThanOrEqual(fps, target < 200_000 ? 8 : 27, "low bitrate must still produce usable video")
            XCTAssertLessThan(gap, 1000, "low bitrate must not stall the decoder")
        }
        XCTAssertLessThan(rates[1], rates[0] * 0.6)
        XCTAssertGreaterThan(rates[2], rates[1] * 2, "encoder must recover after bandwidth returns")
    }

    func testDelayedKeyframeRepairStillDecodesBufferedVideo() throws {
        let queue = DispatchQueue(label: "iris.call.repair.decode")
        let decoded = expectation(description: "all frames decode after repaired reference")
        let lock = NSLock()
        var count = 0
        let receiver = IrisCallVideoReceiver(queue: queue, output: { _ in
            lock.lock(); count += 1; let first = count == 1; lock.unlock()
            if first { decoded.fulfill() }
        }, requestKeyFrame: {})
        var sequence: UInt32 = 0
        let encoder = IrisH264Encoder { data, timestamp, key, _ in
            queue.async {
                let seq = sequence; sequence &+= 1
                // A repaired first IDR arrives after several dependent frames.
                queue.asyncAfter(deadline: .now() + .milliseconds(seq == 0 ? 120 : 0)) {
                    receiver.receive(sequence: seq, timestampUs: timestamp, keyFrame: key, data: data)
                }
            }
        }
        defer { encoder.stop(); queue.sync { receiver.stop() } }
        let pixel = try Self.movingFrame(0)
        let began = ProcessInfo.processInfo.systemUptime
        for frame in 0..<12 {
            let wait = began + Double(frame) / 30 - ProcessInfo.processInfo.systemUptime
            if wait > 0 { Thread.sleep(forTimeInterval: wait) }
            try encoder.encode(pixel, timestampUs: UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000))
        }
        wait(for: [decoded], timeout: 2)
        Thread.sleep(forTimeInterval: 0.1)
        lock.lock(); let total = count; lock.unlock()
        let encoded = queue.sync { sequence }
        XCTAssertGreaterThanOrEqual(encoded, 4, "need dependent frames to exercise repair")
        XCTAssertEqual(total, Int(encoded), "repaired reference must unlock every emitted frame")
    }

    func testSustainedVideoLatency() throws {
        let pixels = try (0..<12).map(Self.movingFrame)
        let lock = NSLock()
        let decodeQueue = DispatchQueue(label: "iris.call.quality.decode")
        var encodeMs: [Double] = []
        var decodeMs: [Double] = []
        var arrivals: [Double] = []
        var bytes = 0
        var keyframes = 0
        let now = { ProcessInfo.processInfo.systemUptime }
        var measurementStartUs = UInt64.max
        var firstFrameMs: Double?
        let startup = now()
        let decoder = IrisH264Decoder { pixel, timestamp in
            XCTAssertEqual(CVPixelBufferGetWidth(pixel), 1280)
            XCTAssertEqual(CVPixelBufferGetHeight(pixel), 720)
            let time = now()
            lock.lock()
            if firstFrameMs == nil { firstFrameMs = (time - startup) * 1000 }
            if timestamp >= measurementStartUs {
                decodeMs.append(time * 1000 - Double(timestamp) / 1000)
                arrivals.append(time)
            }
            lock.unlock()
        }
        decoder.onFailure = { XCTFail("decoder rejected a frame") }
        let encoder = IrisH264Encoder { data, timestamp, key, _ in
            lock.lock()
            if timestamp >= measurementStartUs {
                encodeMs.append(now() * 1000 - Double(timestamp) / 1000)
                bytes += data.count
                if key { keyframes += 1 }
            }
            lock.unlock()
            decodeQueue.async {
                XCTAssertTrue(decoder.decode(data, timestampUs: timestamp, keyFrame: key))
            }
        }
        defer { encoder.stop(); decodeQueue.sync { decoder.stop() } }
        // Cold hardware setup is measured separately from steady-state cadence.
        // Otherwise initialization delay causes the synthetic camera to submit a
        // burst of catch-up frames and falsely report a sustained encoding stall.
        for index in 0..<30 {
            try encoder.encode(pixels[index % pixels.count], timestampUs: UInt64(now() * 1_000_000))
            Thread.sleep(forTimeInterval: 1.0 / 30)
        }
        let start = now()
        lock.lock()
        measurementStartUs = UInt64(start * 1_000_000)
        lock.unlock()
        let frameCount = 90
        for index in 0..<frameCount {
            let delay = start + Double(index) / 30 - now()
            if delay > 0 { Thread.sleep(forTimeInterval: delay) }
            if index % 30 == 0 { encoder.requestKeyFrame() }
            try encoder.encode(pixels[index % pixels.count], timestampUs: UInt64(now() * 1_000_000))
        }
        // Let callbacks finish without flushing a potentially stalled encoder.
        Thread.sleep(forTimeInterval: 0.15)
        lock.lock()
        let encoded = encodeMs.sorted()
        let decoded = decodeMs.sorted()
        let times = [start] + arrivals + [max(start + 3, arrivals.last ?? start)]
        let totalBytes = bytes
        let keys = keyframes
        let startupMs = firstFrameMs ?? -1
        lock.unlock()
        let p95 = { (values: [Double]) in values.isEmpty ? -1 : values[values.count * 95 / 100] }
        let maxGap = zip(times, times.dropFirst()).map { ($1 - $0) * 1000 }.max() ?? 3000
        let result: [String: Any] = [
            "stage": "apple_codec", "width": 1280, "height": 720,
            "captured": frameCount, "encoded": encoded.count, "decoded": decoded.count,
            "fps": Double(decoded.count) / 3, "encode_p95_ms": p95(encoded),
            "capture_to_decode_p95_ms": p95(decoded), "max_frame_gap_ms": maxGap,
            "bitrate_bps": Double(totalBytes * 8) / 3, "keyframes": keys,
            "first_frame_ms": startupMs,
        ]
        let json = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        print("CALL_QUALITY \(String(decoding: json, as: UTF8.self))")
        XCTAssertGreaterThanOrEqual(encoded.count, 87, "encoder stalled or dropped more than 3 frames")
        XCTAssertGreaterThanOrEqual(decoded.count, 87, "decoder cannot sustain camera cadence")
        XCTAssertGreaterThanOrEqual(keys, 3, "recovery frames must be delivered during the stream")
        XCTAssertGreaterThanOrEqual(startupMs, 0, "no video arrived")
        XCTAssertLessThan(startupMs, 1000, "cold codec startup took more than a second")
        XCTAssertLessThan(p95(encoded), 100, "encoding exceeded its latency budget")
        XCTAssertLessThan(p95(decoded), 150, "capture-to-decode exceeded its latency budget")
        XCTAssertLessThan(maxGap, 250, "video froze during a continuous stream")
    }

    private static func movingFrame(_ phase: Int) throws -> CVPixelBuffer {
        var pixel: CVPixelBuffer?
        let status = CVPixelBufferCreate(kCFAllocatorDefault, 1280, 720, kCVPixelFormatType_32BGRA,
            [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &pixel)
        XCTAssertEqual(status, kCVReturnSuccess)
        let buffer = try XCTUnwrap(pixel)
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        let base = try XCTUnwrap(CVPixelBufferGetBaseAddress(buffer))
        let rowBytes = CVPixelBufferGetBytesPerRow(buffer)
        // Detailed moving input, rather than a flat image that compresses to nothing.
        for y in 0..<720 {
            for x in stride(from: 0, to: 1280, by: 16) {
                let shade = ((x / 16 + y / 16 + phase) % 2 == 0) ? 48 : 208
                memset(base.advanced(by: y * rowBytes + x * 4), Int32(shade), 16 * 4)
            }
        }
        return buffer
    }
}
