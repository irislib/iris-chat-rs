import AVFoundation

/// Capture delegates execute on the media queue; late camera frames are dropped
/// by AVFoundation instead of growing latency behind encoding/network work.
final class IrisCallCamera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    private var session: AVCaptureSession?
    private let queue: DispatchQueue
    private let output: (CVPixelBuffer, UInt64) -> Void
    private var height = 0

    init(queue: DispatchQueue, output: @escaping (CVPixelBuffer, UInt64) -> Void) {
        self.queue = queue
        self.output = output
    }

    func start(height: Int) throws {
        guard session == nil || self.height != height else { return }
        stop()
        let session = AVCaptureSession()
        session.beginConfiguration()
        let preset: AVCaptureSession.Preset = height > 720 ? .hd1920x1080 : .hd1280x720
        if session.canSetSessionPreset(preset) { session.sessionPreset = preset }
        else if session.canSetSessionPreset(.hd1280x720) { session.sessionPreset = .hd1280x720 }
        else { session.sessionPreset = .vga640x480 }
#if os(iOS)
        let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
#else
        let device = AVCaptureDevice.default(for: .video)
#endif
        guard let device else { throw NSError(domain: "IrisCall", code: 1) }
        let input = try AVCaptureDeviceInput(device: device)
        guard session.canAddInput(input) else { throw NSError(domain: "IrisCall", code: 2) }
        session.addInput(input)
        let output = AVCaptureVideoDataOutput()
        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
        output.setSampleBufferDelegate(self, queue: queue)
        guard session.canAddOutput(output) else { throw NSError(domain: "IrisCall", code: 3) }
        session.addOutput(output)
#if os(iOS)
        if let connection = output.connection(with: .video), connection.isVideoOrientationSupported {
            connection.videoOrientation = .portrait
        }
#endif
        session.commitConfiguration()
        if device.activeFormat.videoSupportedFrameRateRanges.contains(where: { $0.minFrameRate <= 30 && $0.maxFrameRate >= 30 }) {
            try device.lockForConfiguration()
            device.activeVideoMinFrameDuration = CMTime(value: 1, timescale: 30)
            device.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: 30)
            device.unlockForConfiguration()
        }
        self.height = height
        self.session = session
        session.startRunning()
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer,
                       from connection: AVCaptureConnection) {
        guard session != nil, let image = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let time = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        let micros = CMTimeConvertScale(time, timescale: 1_000_000, method: .default).value
        self.output(image, UInt64(max(0, micros)))
    }

    func stop() { session?.stopRunning(); session = nil; height = 0 }
    deinit { stop() }
}
