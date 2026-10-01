import VideoToolbox
import XCTest

enum CallVideoTestSupport {
    static func requireHardwareH264Encoder() throws {
#if targetEnvironment(simulator)
        guard #available(iOS 17.4, *) else { return }
        // Probe the system independently of Iris's low-latency configuration.
        // A successful probe must still exercise every production assertion.
        var session: VTCompressionSession?
        let status = VTCompressionSessionCreate(
            allocator: kCFAllocatorDefault, width: 1280, height: 720,
            codecType: kCMVideoCodecType_H264,
            encoderSpecification: [kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true] as CFDictionary,
            imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: nil, refcon: nil, compressionSessionOut: &session)
        defer { if let session { VTCompressionSessionInvalidate(session) } }
        let evidence = "{\"platform\":\"iOS Simulator\",\"codec\":\"h264\",\"width\":1280,\"height\":720,\"hardwareRequired\":true,\"createStatus\":\(status)}"
        print("CALL_VIDEO_CAPABILITY \(evidence)")
        XCTContext.runActivity(named: "Independent hardware H.264 availability") { activity in
            let attachment = XCTAttachment(string: evidence)
            attachment.name = "hardware-h264-capability.json"
            attachment.lifetime = .keepAlways
            activity.add(attachment)
        }
        if status == kVTCouldNotFindVideoEncoderErr {
            throw XCTSkip("Simulator has no hardware H.264 encoder: independent hardware-required session creation returned kVTCouldNotFindVideoEncoderErr (-12908). Physical-device codec coverage is still required.")
        }
        guard status == noErr, session != nil else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status), userInfo: [
                NSLocalizedDescriptionKey: "Independent hardware H.264 capability probe failed unexpectedly."
            ])
        }
#endif
    }
}
