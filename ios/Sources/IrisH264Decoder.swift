import AVFoundation
import VideoToolbox

final class IrisH264Decoder {
    private var session: VTDecompressionSession?
    private var format: CMVideoFormatDescription?
    private var parameterSets: [Data] = []
    private var needsKeyFrame = true
    private let output: (CVPixelBuffer) -> Void
    var onFailure: (() -> Void)?

    init(output: @escaping (CVPixelBuffer) -> Void) { self.output = output }

    func discontinuity() { needsKeyFrame = true }

    /// Returns false when the sender should refresh the decoder with an IDR.
    func decode(_ data: Data, timestampUs: UInt64, keyFrame: Bool) -> Bool {
        guard let units = IrisH264Wire.units(data) else { return false }
        let isIDR = units.contains { $0.first.map { $0 & 0x1f == 5 } == true }
        guard keyFrame == isIDR, !needsKeyFrame || isIDR else { return false }
        if isIDR {
            guard let sps = units.first(where: { $0.first.map { $0 & 0x1f == 7 } == true }),
                  let pps = units.first(where: { $0.first.map { $0 & 0x1f == 8 } == true }) else { return false }
            if parameterSets != [sps, pps] {
                guard create(sps: sps, pps: pps) else { return false }
            }
        }
        guard let session, let format else { return false }
        let slices = units.filter { unit in
            guard let first = unit.first else { return false }
            return first & 0x1f != 7 && first & 0x1f != 8
        }
        let bytes = IrisH264Wire.avcc(slices)
        guard !bytes.isEmpty else { return false }
        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(allocator: kCFAllocatorDefault, memoryBlock: nil,
            blockLength: bytes.count, blockAllocator: kCFAllocatorDefault, customBlockSource: nil,
            offsetToData: 0, dataLength: bytes.count, flags: 0, blockBufferOut: &block) == noErr,
            let block else { return false }
        let copied = bytes.withUnsafeBytes { raw in
            CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: bytes.count)
        }
        guard copied == noErr else { return false }
        var timing = CMSampleTimingInfo(duration: .invalid,
            presentationTimeStamp: CMTime(value: Int64(clamping: timestampUs), timescale: 1_000_000), decodeTimeStamp: .invalid)
        var size = bytes.count
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateReady(allocator: kCFAllocatorDefault, dataBuffer: block, formatDescription: format,
            sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
            sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample) == noErr,
            let sample else { return false }
        let status = VTDecompressionSessionDecodeFrame(session, sampleBuffer: sample,
            flags: [._EnableAsynchronousDecompression, ._1xRealTimePlayback], frameRefcon: nil, infoFlagsOut: nil)
        needsKeyFrame = status != noErr
        return status == noErr
    }

    private func create(sps: Data, pps: Data) -> Bool {
        stop()
        var description: CMFormatDescription?
        let status = sps.withUnsafeBytes { spsBytes in
            pps.withUnsafeBytes { ppsBytes in
                let pointers = [spsBytes.bindMemory(to: UInt8.self).baseAddress!, ppsBytes.bindMemory(to: UInt8.self).baseAddress!]
                let sizes = [sps.count, pps.count]
                return CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: kCFAllocatorDefault,
                    parameterSetCount: 2, parameterSetPointers: pointers, parameterSetSizes: sizes,
                    nalUnitHeaderLength: 4, formatDescriptionOut: &description)
            }
        }
        guard status == noErr, let description else { return false }
        let dimensions = CMVideoFormatDescriptionGetDimensions(description)
        guard dimensions.width > 0, dimensions.height > 0, dimensions.width <= 1920, dimensions.height <= 1920,
              Int64(dimensions.width) * Int64(dimensions.height) <= 1920 * 1080 else { return false }
        var callback = VTDecompressionOutputCallbackRecord(decompressionOutputCallback: { refcon, _, status, _, buffer, _, _ in
            guard let refcon else { return }
            let decoder = Unmanaged<IrisH264Decoder>.fromOpaque(refcon).takeUnretainedValue()
            guard status == noErr, let buffer else { decoder.onFailure?(); return }
            decoder.output(buffer)
        }, decompressionOutputRefCon: Unmanaged.passUnretained(self).toOpaque())
        var created: VTDecompressionSession?
        var specification: CFDictionary?
        if #available(iOS 17.0, *) {
            specification = [kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: true] as CFDictionary
        }
        guard VTDecompressionSessionCreate(allocator: kCFAllocatorDefault, formatDescription: description,
            decoderSpecification: specification,
            imageBufferAttributes: [kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                                   kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary,
            outputCallback: &callback, decompressionSessionOut: &created) == noErr, let created else { return false }
        session = created
        format = description
        parameterSets = [sps, pps]
        return true
    }

    func stop() {
        if let session {
            VTDecompressionSessionWaitForAsynchronousFrames(session)
            VTDecompressionSessionInvalidate(session)
        }
        session = nil
        format = nil
        parameterSets = []
        needsKeyFrame = true
    }

    deinit { stop() }
}
