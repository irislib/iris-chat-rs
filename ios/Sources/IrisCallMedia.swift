import AVFoundation
import CoreImage
import ImageIO
import Foundation

/// The wire format is shared with Android and the desktop client: 20 ms of
/// signed little-endian PCM, mono at 16 kHz; video is a small independent JPEG.
enum IrisCallMediaFormat {
    static let audioBytes = 640
    static let maxVideoBytes = 65_536

    static func audioSamples(_ data: Data) -> [Float]? {
        guard data.count == audioBytes else { return nil }
        let bytes = [UInt8](data)
        return stride(from: 0, to: bytes.count, by: 2).map { offset in
            Float(Int16(bitPattern: UInt16(bytes[offset]) | UInt16(bytes[offset + 1]) << 8)) / 32768
        }
    }

    static func videoImage(_ data: Data) -> CGImage? {
        guard data.count <= maxVideoBytes,
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetType(source) as String? == "public.jpeg",
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 320, height <= 240 else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }
}

/// All hardware and playback operations stay on one serial queue. Capture
/// callbacks use a tiny lock to read the current generation and mute state.
protocol IrisCallMediaHandling: AnyObject {
    func configure(callID: String?, muted: Bool, video: Bool)
    func receiveAudio(callID: String, data: Data)
}

final class IrisCallMediaEngine: NSObject, IrisCallMediaHandling, AVCaptureVideoDataOutputSampleBufferDelegate {
    private let queue = DispatchQueue(label: "iris.call.hardware", qos: .userInteractive)
    private let captureQueue = DispatchQueue(label: "iris.call.camera", qos: .userInitiated)
    private let lock = NSLock()
    private var captureCallID: String?
    private var captureMuted = false
    private var captureVideo = false
    private var configurationGeneration: UInt64 = 0
    private var audioEngine: AVAudioEngine?
    private var player: AVAudioPlayerNode?
    private var camera: AVCaptureSession?
    private var queuedAudio = 0
    private var currentCallID: String?
    private var cameraCallID: String?
    private var lastVideoTime: TimeInterval = 0
    private let imageContext = CIContext(options: [.cacheIntermediates: false])
    private let send: (String, UInt8, Data) -> Void
    private let preview: (String, CGImage) -> Void
    private let failed: (String, String) -> Void

    init(send: @escaping (String, UInt8, Data) -> Void,
         preview: @escaping (String, CGImage) -> Void,
         failed: @escaping (String, String) -> Void) {
        self.send = send
        self.preview = preview
        self.failed = failed
    }

    func configure(callID: String?, muted: Bool, video: Bool) {
        lock.lock()
        captureCallID = callID
        captureMuted = muted
        captureVideo = video
        configurationGeneration &+= 1
        let generation = configurationGeneration
        lock.unlock()
        queue.async { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let current = self.configurationGeneration == generation
            self.lock.unlock()
            guard current else { return }
            if self.currentCallID != callID {
                self.stopHardware()
                self.currentCallID = callID
                if let callID {
                    do { try self.startAudio(callID: callID) }
                    catch { self.failed(callID, "Couldn’t start the microphone.") }
                }
            }
            if video, let callID, self.camera == nil {
                do { try self.startCamera(callID: callID) }
                catch { self.failed(callID, "Couldn’t start the camera.") }
            } else if !video {
                self.stopCamera()
            }
        }
    }

    func receiveAudio(callID: String, data: Data) {
        guard let samples = IrisCallMediaFormat.audioSamples(data) else { return }
        queue.async { [weak self] in
            guard let self, self.currentCallID == callID,
                  let player = self.player, self.queuedAudio < 5,
                  let format = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1),
                  let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 320),
                  let channel = buffer.floatChannelData?[0] else { return }
            buffer.frameLength = 320
            for index in samples.indices { channel[index] = samples[index] }
            self.queuedAudio += 1
            player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { [weak self, weak player] _ in
                self?.queue.async { [weak self, weak player] in
                    guard let self, self.player === player else { return }
                    self.queuedAudio = max(0, self.queuedAudio - 1)
                }
            }
        }
    }

    private func startAudio(callID: String) throws {
        let engine = AVAudioEngine()
        let input = engine.inputNode
        // System voice processing provides echo cancellation on speakerphone.
        try? input.setVoiceProcessingEnabled(true)
        let inputFormat = input.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0,
              let wireFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1),
              let converter = AVAudioConverter(from: inputFormat, to: wireFormat) else {
            throw NSError(domain: "IrisCall", code: 1)
        }
        let player = AVAudioPlayerNode()
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: wireFormat)
        var pending = Data()
        input.installTap(onBus: 0, bufferSize: 1024, format: inputFormat) { [weak self] buffer, _ in
            guard let self else { return }
            self.lock.lock()
            let enabled = self.captureCallID == callID && !self.captureMuted
            self.lock.unlock()
            guard enabled else { pending.removeAll(keepingCapacity: true); return }
            let capacity = AVAudioFrameCount(ceil(Double(buffer.frameLength) * 16_000 / inputFormat.sampleRate)) + 32
            guard let output = AVAudioPCMBuffer(pcmFormat: wireFormat, frameCapacity: capacity) else { return }
            var supplied = false
            var error: NSError?
            converter.convert(to: output, error: &error) { _, status in
                if supplied { status.pointee = .noDataNow; return nil }
                supplied = true
                status.pointee = .haveData
                return buffer
            }
            guard error == nil, let samples = output.floatChannelData?[0] else { return }
            for index in 0..<Int(output.frameLength) {
                let sample = samples[index].isFinite ? max(-1, min(1, samples[index])) : 0
                let value = Int16(max(-32768, min(32767, Int(sample * 32768))))
                let bits = UInt16(bitPattern: value)
                pending.append(UInt8(bits & 0xff))
                pending.append(UInt8(bits >> 8))
            }
            while pending.count >= IrisCallMediaFormat.audioBytes {
                self.send(callID, 1, Data(pending.prefix(IrisCallMediaFormat.audioBytes)))
                pending.removeFirst(IrisCallMediaFormat.audioBytes)
            }
        }
        do {
            engine.prepare()
            try engine.start()
            player.play()
            audioEngine = engine
            self.player = player
        } catch {
            input.removeTap(onBus: 0)
            engine.stop()
            throw error
        }
    }

    private func startCamera(callID: String) throws {
        let session = AVCaptureSession()
        session.sessionPreset = .low
#if os(iOS)
        let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
#else
        let device = AVCaptureDevice.default(for: .video)
#endif
        guard let device else { throw NSError(domain: "IrisCall", code: 2) }
        let input = try AVCaptureDeviceInput(device: device)
        guard session.canAddInput(input) else { throw NSError(domain: "IrisCall", code: 3) }
        session.addInput(input)
        let output = AVCaptureVideoDataOutput()
        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
        output.setSampleBufferDelegate(self, queue: captureQueue)
        guard session.canAddOutput(output) else { throw NSError(domain: "IrisCall", code: 4) }
        session.addOutput(output)
#if os(iOS)
        if let connection = output.connection(with: .video), connection.isVideoOrientationSupported {
            connection.videoOrientation = .portrait
        }
#endif
        captureQueue.sync { cameraCallID = callID; lastVideoTime = 0 }
        camera = session
        session.startRunning()
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer,
                       from connection: AVCaptureConnection) {
        let now = ProcessInfo.processInfo.systemUptime
        guard let callID = cameraCallID, now - lastVideoTime >= 0.125,
              let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        lock.lock()
        let enabled = captureCallID == callID && captureVideo
        lock.unlock()
        guard enabled else { return }
        lastVideoTime = now
        let source = CIImage(cvPixelBuffer: buffer)
        let scale = min(320 / source.extent.width, 240 / source.extent.height, 1)
        let resized = source.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        guard let jpeg = imageContext.jpegRepresentation(of: resized, colorSpace: CGColorSpaceCreateDeviceRGB(),
                            options: [kCGImageDestinationLossyCompressionQuality as CIImageRepresentationOption: 0.45]),
              jpeg.count <= IrisCallMediaFormat.maxVideoBytes,
              let image = IrisCallMediaFormat.videoImage(jpeg) else { return }
        send(callID, 2, jpeg)
        preview(callID, image)
    }

    private func stopCamera() {
        camera?.stopRunning()
        camera = nil
        captureQueue.sync { cameraCallID = nil }
    }

    private func stopHardware() {
        stopCamera()
        audioEngine?.inputNode.removeTap(onBus: 0)
        audioEngine?.stop()
        player?.stop()
        audioEngine = nil
        player = nil
        queuedAudio = 0
    }

    deinit {
        camera?.stopRunning()
        audioEngine?.stop()
    }
}
