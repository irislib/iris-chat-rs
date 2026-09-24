import AVFoundation
import VideoToolbox

/// Hardware encoding stays on the call's serial media queue. VideoToolbox may
/// invoke its output callback elsewhere; that callback never waits for network IO.
final class IrisH264Encoder {
    private var session: VTCompressionSession?
    private var dimensions = CGSize.zero
    private var bitrate = 2_000_000
    private var forceKeyFrame = true
    private var lastEncodedTimestamp: UInt64?
    private var frameRate: Int { bitrate < 200_000 ? 10 : bitrate < 500_000 ? 15 : 30 }
    private let pendingFrames = DispatchSemaphore(value: 3)
    private final class PendingFrame {
        let isCurrent: () -> Bool
        init(_ isCurrent: @escaping () -> Bool) { self.isCurrent = isCurrent }
    }
    private let output: (Data, UInt64, Bool, @escaping () -> Bool) -> Void

    init(output: @escaping (Data, UInt64, Bool, @escaping () -> Bool) -> Void) { self.output = output }

    func setBitrate(_ bitrate: Int) {
        self.bitrate = max(64_000, min(10_000_000, bitrate))
        if let session { applyBitrate(session) }
    }

    func requestKeyFrame() { forceKeyFrame = true }

    func encode(_ pixel: CVPixelBuffer, timestampUs: UInt64, isCurrent: @escaping () -> Bool = { true }) throws {
        let width = CVPixelBufferGetWidth(pixel), height = CVPixelBufferGetHeight(pixel)
        guard width > 0, height > 0, width * height <= 1920 * 1080 else { return }
        // Bitrate alone cannot make detailed 720p/30 input fit a narrow link.
        // VideoToolbox scales the input into this session's encoded dimensions.
        let longEdge = bitrate < 200_000 ? 320 : bitrate < 500_000 ? 640 : bitrate < 1_000_000 ? 960 : max(width, height)
        let scale = min(1, Double(longEdge) / Double(max(width, height)))
        let encodedWidth = max(2, Int(Double(width) * scale) / 2 * 2)
        let encodedHeight = max(2, Int(Double(height) * scale) / 2 * 2)
        if session == nil || dimensions != CGSize(width: encodedWidth, height: encodedHeight) {
            stop()
            try create(width: encodedWidth, height: encodedHeight)
        }
        if frameRate < 30, let lastEncodedTimestamp, timestampUs >= lastEncodedTimestamp,
           timestampUs - lastEncodedTimestamp < UInt64(950_000 / frameRate) { return }
        guard let session, pendingFrames.wait(timeout: .now()) == .success else { return }
        lastEncodedTimestamp = timestampUs
        let properties: CFDictionary? = forceKeyFrame ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        forceKeyFrame = false
        let context = Unmanaged.passRetained(PendingFrame(isCurrent)).toOpaque()
        let status = VTCompressionSessionEncodeFrame(session, imageBuffer: pixel,
            presentationTimeStamp: CMTime(value: Int64(clamping: timestampUs), timescale: 1_000_000),
            duration: CMTime(value: 1, timescale: Int32(frameRate)), frameProperties: properties,
            sourceFrameRefcon: context, infoFlagsOut: nil)
        if status != noErr {
            Unmanaged<PendingFrame>.fromOpaque(context).release()
            pendingFrames.signal()
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
    }

    private func create(width: Int, height: Int) throws {
        var created: VTCompressionSession?
        var specification: [CFString: Any] = [kVTVideoEncoderSpecification_EnableLowLatencyRateControl: true]
        if #available(iOS 17.4, *) {
            specification[kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder] = true
        }
        let status = VTCompressionSessionCreate(allocator: kCFAllocatorDefault, width: Int32(width), height: Int32(height),
            codecType: kCMVideoCodecType_H264,
            encoderSpecification: specification as CFDictionary,
            imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: { refcon, frameRefcon, status, _, sample in
                guard let refcon else { return }
                let encoder = Unmanaged<IrisH264Encoder>.fromOpaque(refcon).takeUnretainedValue()
                defer { encoder.pendingFrames.signal() }
                guard let frameRefcon else { return }
                let context = Unmanaged<PendingFrame>.fromOpaque(frameRefcon).takeRetainedValue()
                guard context.isCurrent(), status == noErr, let sample, CMSampleBufferDataIsReady(sample) else { return }
                let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
                let key = attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool != true
                guard let bytes = IrisH264Wire.annexB(sample, keyFrame: key) else { return }
                let time = CMSampleBufferGetPresentationTimeStamp(sample)
                let microseconds = CMTimeConvertScale(time, timescale: 1_000_000, method: .default).value
                encoder.output(bytes, UInt64(max(0, microseconds)), key, context.isCurrent)
            }, refcon: Unmanaged.passUnretained(self).toOpaque(), compressionSessionOut: &created)
        guard status == noErr, let created else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
        session = created
        dimensions = CGSize(width: width, height: height)
        let profile = width * height > 1280 * 720 ? kVTProfileLevel_H264_Baseline_4_0 : kVTProfileLevel_H264_Baseline_3_1
        for (key, value) in [(kVTCompressionPropertyKey_RealTime, true as CFTypeRef),
                             (kVTCompressionPropertyKey_AllowFrameReordering, false as CFTypeRef),
                             (kVTCompressionPropertyKey_ProfileLevel, profile as CFTypeRef),
                             (kVTCompressionPropertyKey_ExpectedFrameRate, frameRate as CFTypeRef),
                             (kVTCompressionPropertyKey_MaxKeyFrameInterval, frameRate as CFTypeRef),
                             (kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, 1 as CFTypeRef)] {
            let result = VTSessionSetProperty(created, key: key, value: value)
            if result != noErr { stop(); throw NSError(domain: NSOSStatusErrorDomain, code: Int(result)) }
        }
        applyBitrate(created)
        let prepared = VTCompressionSessionPrepareToEncodeFrames(created)
        if prepared != noErr { stop(); throw NSError(domain: NSOSStatusErrorDomain, code: Int(prepared)) }
        forceKeyFrame = true
    }

    private func applyBitrate(_ session: VTCompressionSession) {
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AverageBitRate, value: bitrate as CFNumber)
        // A one-second bound allows IDR bursts without permitting an unbounded
        // queue. The transport has its own smaller per-frame deadline.
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_DataRateLimits,
                             value: [bitrate / 8, 1] as CFArray)
    }

    func stop() {
        if let session {
            VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(session)
        }
        session = nil
        dimensions = .zero
        lastEncodedTimestamp = nil
    }

    deinit { stop() }
}
