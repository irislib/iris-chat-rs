import AVFoundation
import Foundation

struct IrisCallMediaSession: Equatable {
    let callID: String
    let videoCapable: Bool
}

protocol IrisCallMediaHandling: AnyObject {
    func start(_ session: IrisCallMediaSession)
    func configure(callID: String?, muted: Bool, video: Bool)
    func setQuality(_ quality: IrisCallQuality, customKilobits: Int)
    func adapt(targetBitrate: UInt32, keyFrameGeneration: UInt32)
    func receive(callID: String, kind: UInt8, sequence: UInt32, timestampUs: UInt64, keyFrame: Bool, data: Data)
    func stop()
}

/// No sockets or network stack live in the media engine. Only authenticated
/// call actions carry Opus and H.264 frames through the Rust FIPS connection.
final class IrisCallMediaEngine: IrisCallMediaHandling {
    private let queue = DispatchQueue(label: "iris.call.media", qos: .userInteractive)
    private let lock = NSLock()
    private let incomingVideoSlots = DispatchSemaphore(value: 3)
    private let incomingAudioSlots = DispatchSemaphore(value: 8)
    private let captureGate = IrisCallSendGate()
    private var captureCallID: String?
    private var captureMuted = true
    private var captureVideo = false
    private var generation: UInt64 = 0
    private var cameraGeneration: UInt64 = 0
    private var encodedCameraGeneration: UInt64 = 0
    private var session: IrisCallMediaSession?
    private var audio: IrisCallAudioHandling?
    private let audioForTesting: IrisCallAudioHandling?
    private var camera: IrisCallCamera?
    private var encoder: IrisH264Encoder?
    private var receiver: IrisCallVideoReceiver?
    private var quality = IrisCallQuality.automatic
    private var targetBitrate: UInt32 = 2_000_000
    private var keyFrameGeneration: UInt32 = 0
    private var ready = false
    private let send: (String, UInt8, UInt64, Bool, Data, @escaping () -> Bool) -> Void
    private let frame: (String, Bool, CVPixelBuffer) -> Void
    private let connectionChanged: (String, Bool) -> Void
    private let requestKeyFrame: (String) -> Void
    private let failed: (String, String) -> Void

    init(send: @escaping (String, UInt8, UInt64, Bool, Data, @escaping () -> Bool) -> Void,
         frame: @escaping (String, Bool, CVPixelBuffer) -> Void,
         connectionChanged: @escaping (String, Bool) -> Void,
         requestKeyFrame: @escaping (String) -> Void,
         failed: @escaping (String, String) -> Void,
         audioForTesting: IrisCallAudioHandling? = nil) {
        self.send = send
        self.frame = frame
        self.connectionChanged = connectionChanged
        self.requestKeyFrame = requestKeyFrame
        self.failed = failed
        self.audioForTesting = audioForTesting
    }

    func start(_ session: IrisCallMediaSession) {
        queue.async { [weak self] in
            guard let self, self.session?.callID != session.callID else { return }
            self.stopHardware()
            self.session = session
            self.receiver = IrisCallVideoReceiver(queue: self.queue,
                output: { [weak self] pixel in self?.frame(session.callID, false, pixel) },
                requestKeyFrame: { [weak self] in self?.requestKeyFrame(session.callID) })
            self.encoder = IrisH264Encoder { [weak self] data, timestamp, key, allowed in
                guard let self, self.allowed(session.callID, video: true) else { return }
                self.send(session.callID, 2, timestamp, key, data, allowed)
            }
            self.encoder?.setBitrate(Int(self.targetBitrate))
            self.audio = self.audioForTesting ?? IrisCallAudio(queue: self.queue, permission: { [captureGate = self.captureGate] timestamp in captureGate.permission(callID: session.callID, kind: 1, capturedAtUs: timestamp) }) {
                [weak self] data, timestamp, allowed in
                guard let self, self.allowed(session.callID, video: false) else { return }
                self.send(session.callID, 1, timestamp, false, data, allowed)
            }
            self.camera = IrisCallCamera(queue: self.queue) { [weak self] pixel, timestamp in
                guard let self, self.allowed(session.callID, video: true) else { return }
                let allowed = self.captureGate.permission(callID: session.callID, kind: 2, capturedAtUs: timestamp)
                guard allowed() else { return }
                self.lock.lock()
                let cameraGeneration = self.cameraGeneration
                self.lock.unlock()
                if self.encodedCameraGeneration != cameraGeneration {
                    self.encodedCameraGeneration = cameraGeneration
                    self.encoder?.requestKeyFrame()
                }
                self.frame(session.callID, true, pixel)
                do { try self.encoder?.encode(pixel, timestampUs: timestamp, isCurrent: allowed) }
                catch { self.failed(session.callID, "Couldn’t start call video.") }
            }
        }
    }

    func configure(callID: String?, muted: Bool, video: Bool) {
        captureGate.update(callID: callID, muted: muted, video: video)
        lock.lock()
        if captureCallID != callID || captureVideo != video { cameraGeneration &+= 1 }
        captureCallID = callID
        captureMuted = muted
        captureVideo = video
        generation &+= 1
        let currentGeneration = generation
        lock.unlock()
        queue.async { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let current = self.generation == currentGeneration
            self.lock.unlock()
            guard current else { return }
            guard let session = self.session, session.callID == callID else {
                self.audio?.stop()
                self.camera?.stop()
                self.encoder?.stop()
                return
            }
            do {
                try self.audio?.start()
                self.audio?.setMuted(muted)
                if video && session.videoCapable { try self.camera?.start(height: self.quality.captureHeight) }
                else { self.camera?.stop(); self.encoder?.stop() }
                if !self.ready { self.ready = true; self.connectionChanged(session.callID, true) }
            } catch { self.failed(session.callID, video ? "Couldn’t start the camera or microphone." : "Couldn’t start the microphone.") }
        }
    }

    func setQuality(_ quality: IrisCallQuality, customKilobits: Int) {
        queue.async { [weak self] in self?.quality = quality }
    }

    func adapt(targetBitrate: UInt32, keyFrameGeneration: UInt32) {
        queue.async { [weak self] in
            guard let self else { return }
            self.targetBitrate = targetBitrate
            self.encoder?.setBitrate(Int(targetBitrate))
            if self.keyFrameGeneration != keyFrameGeneration {
                self.keyFrameGeneration = keyFrameGeneration
                self.encoder?.requestKeyFrame()
            }
        }
    }

    func receive(callID: String, kind: UInt8, sequence: UInt32, timestampUs: UInt64, keyFrame: Bool, data: Data) {
        guard kind == 1 || kind == 2, data.count <= (kind == 1 ? 1_275 : IrisH264Wire.maximumBytes) else { return }
        let slots = kind == 1 ? incomingAudioSlots : incomingVideoSlots
        guard slots.wait(timeout: .now()) == .success else { return }
        queue.async { [weak self] in
            defer { slots.signal() }
            guard let self, self.session?.callID == callID else { return }
            if kind == 1 { self.audio?.receive(sequence: sequence, data: data); return }
            self.receiver?.receive(sequence: sequence, timestampUs: timestampUs, keyFrame: keyFrame, data: data)
        }
    }

    private func allowed(_ id: String, video: Bool) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return captureCallID == id && (video ? captureVideo : !captureMuted)
    }

    func stop() {
        captureGate.update(callID: nil, muted: true, video: false)
        lock.lock()
        captureCallID = nil
        captureMuted = true
        captureVideo = false
        generation &+= 1
        lock.unlock()
        queue.async { [weak self] in self?.stopHardware() }
    }

    private func stopHardware() {
        audio?.stop(); camera?.stop(); encoder?.stop(); receiver?.stop()
        audio = nil; camera = nil; encoder = nil; receiver = nil; session = nil
        ready = false
    }

    deinit { audio?.stop(); camera?.stop(); encoder?.stop(); receiver?.stop() }
}
