import VideoToolbox
import XCTest

enum CallVideoTestSupport {
    static func requireHardwareH264Encoder() throws {
#if targetEnvironment(simulator)
        guard #available(iOS 17.4, *) else { return }
        // Low-latency rate control selects a distinct encoder capability. Probe
        // both modes independently of Iris; generic hardware support is not enough.
        func probe(lowLatency: Bool) -> (status: OSStatus, sessionCreated: Bool) {
            var specification: [CFString: Any] = [kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true]
            if lowLatency { specification[kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = true }
            var session: VTCompressionSession?
            let status = VTCompressionSessionCreate(
                allocator: kCFAllocatorDefault, width: 1280, height: 720,
                codecType: kCMVideoCodecType_H264, encoderSpecification: specification as CFDictionary,
                imageBufferAttributes: nil, compressedDataAllocator: nil,
                outputCallback: nil, refcon: nil, compressionSessionOut: &session)
            // Release the probe before testing another mode or the production encoder.
            defer { if let session { VTCompressionSessionInvalidate(session) } }
            return (status, session != nil)
        }
        let hardware = probe(lowLatency: false)
        let lowLatency = probe(lowLatency: true)
        let evidence = """
        {"platform":"iOS Simulator","codec":"h264","width":1280,"height":720,"hardwareRequired":true,"genericHardwareCreateStatus":\(hardware.status),"genericHardwareSessionCreated":\(hardware.sessionCreated),"hardwareLowLatencyCreateStatus":\(lowLatency.status),"hardwareLowLatencySessionCreated":\(lowLatency.sessionCreated)}
        """
        print("CALL_VIDEO_CAPABILITY \(evidence)")
        XCTContext.runActivity(named: "Independent hardware H.264 encoder modes") { activity in
            let attachment = XCTAttachment(string: evidence)
            attachment.name = "hardware-h264-capability.json"
            attachment.lifetime = .keepAlways
            activity.add(attachment)
        }
        for result in [hardware, lowLatency] {
            guard (result.status == noErr && result.sessionCreated) ||
                    (result.status == kVTCouldNotFindVideoEncoderErr && !result.sessionCreated) else {
                throw NSError(domain: NSOSStatusErrorDomain, code: Int(result.status), userInfo: [
                    NSLocalizedDescriptionKey: "Independent H.264 encoder-mode probe failed unexpectedly."
                ])
            }
        }
        if lowLatency.status == kVTCouldNotFindVideoEncoderErr {
            throw XCTSkip("Simulator hardware H.264 low-latency mode is unavailable: independent required-mode session creation returned -12908; generic hardware returned \(hardware.status). Physical-device codec coverage is still required.")
        }
        // An available required mode must still exercise every production assertion.
#endif
    }
}
