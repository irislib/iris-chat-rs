import AVFoundation

/// Voice processing stays in Apple's audio graph; Rust owns the shared Opus
/// codec, bounded jitter buffer and packet-loss concealment on every platform.
protocol IrisCallAudioHandling: AnyObject {
    func start() throws
    func setMuted(_ muted: Bool)
    func receive(sequence: UInt32, data: Data)
    func stop()
}

final class IrisCallAudio: IrisCallAudioHandling {
    private let queue: DispatchQueue
    private let permission: (UInt64) -> (() -> Bool)
    private let send: (Data, UInt64, @escaping () -> Bool) -> Void
    private let failed: (Error) -> Void
    private let captureQueue: IrisCallAudioCaptureQueue
    private var engine: AVAudioEngine?
    private var player: AVAudioPlayerNode?
    private var codec: CallAudioCodec?
    private var configurationObserver: NSObjectProtocol?
    private var muted = true
    private var playoutBuffers: IrisCallAudioPlayoutQueue?
    private var loggedCapture = false
    private var loggedPlayback = false
    private var loggedPlayed = false

    init(queue: DispatchQueue, permission: @escaping (UInt64) -> (() -> Bool), failed: @escaping (Error) -> Void,
         send: @escaping (Data, UInt64, @escaping () -> Bool) -> Void) {
        self.queue = queue
        self.captureQueue = IrisCallAudioCaptureQueue(queue: queue)
        self.permission = permission
        self.send = send
        self.failed = failed
    }

    func start() throws {
        guard engine == nil else { return }
        let codec = try CallAudioCodec()
        let engine = AVAudioEngine()
        let input = engine.inputNode
        try input.setVoiceProcessingEnabled(true)
        input.isVoiceProcessingInputMuted = muted
        let inputFormat = input.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0,
              let wireFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1),
              let converter = AVAudioConverter(from: inputFormat, to: wireFormat) else {
            throw NSError(domain: "IrisCall", code: 1)
        }
        let player = AVAudioPlayerNode()
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: wireFormat)
        var pending: [Int16] = []
        var pendingPermission: (() -> Bool)?
        let captureBufferFrames = AVAudioFrameCount(ceil(inputFormat.sampleRate * 0.1))
        input.installTap(onBus: 0, bufferSize: captureBufferFrames, format: inputFormat) { [weak self, codec] buffer, time in
            guard let self else { return }
            let timestamp = UInt64(max(0, (time.isHostTimeValid ? AVAudioTime.seconds(forHostTime: time.hostTime) :
                ProcessInfo.processInfo.systemUptime) * 1_000_000))
            let allowed = self.permission(timestamp)
            if pendingPermission?() != true { pending.removeAll(keepingCapacity: true); converter.reset() }
            pendingPermission = allowed
            guard allowed() else { pending.removeAll(keepingCapacity: true); return }
            let capacity = AVAudioFrameCount(ceil(Double(buffer.frameLength) * 48_000 / inputFormat.sampleRate)) + 32
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
                pending.append(Int16(max(-32768, min(32767, Int(sample * 32768)))))
            }
            var frames: [[Int16]] = []
            while pending.count >= 960 {
                let frame = Array(pending.prefix(960))
                pending.removeFirst(960)
                frames.append(frame)
            }
            self.captureQueue.submit(frames, timestampUs: timestamp) { [weak self, weak codec] frame, frameTimestamp in
                guard let self, self.codec === codec, allowed(), let codec,
                      let bytes = try? codec.encode(samples: frame), !bytes.isEmpty else { return }
#if DEBUG
                if !self.loggedCapture { self.loggedCapture = true; NSLog("IrisCall first microphone frame encoded") }
#endif
                self.send(Data(bytes), frameTimestamp, allowed)
            }
        }
        do {
            self.codec = codec
            configurationObserver = NotificationCenter.default.addObserver(
                forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil
            ) { [weak self, weak engine] _ in
                self?.queue.async { [weak self, weak engine] in
                    guard let self, let engine, self.engine === engine, !engine.isRunning else { return }
#if DEBUG
                    NSLog("IrisCall restarting stopped audio graph after route change")
#endif
                    // A CallKit/headset route change can stop the graph after
                    // start() succeeds. Keep the voice-processing I/O unit and
                    // its tap format; creating a second unit can fail on phones.
                    do {
                        self.playoutBuffers?.stop()
                        self.player?.stop()
                        engine.prepare()
                        try engine.start()
                        self.playoutBuffers?.start()
                        self.player?.play()
                    } catch {
#if DEBUG
                        NSLog("IrisCall audio route recovery failed: %@", error.localizedDescription)
#endif
                        self.failed(error)
                    }
                }
            }
            engine.prepare()
            try engine.start()
            self.engine = engine
            self.player = player
            playoutBuffers = IrisCallAudioPlayoutQueue(queue: queue) { [weak self] done in
                self?.playout(completion: done) ?? false
            }
            playoutBuffers?.start()
            player.play()
        } catch {
            if let configurationObserver { NotificationCenter.default.removeObserver(configurationObserver) }
            configurationObserver = nil
            self.codec = nil
            input.removeTap(onBus: 0)
            engine.stop()
            throw error
        }
    }

    func setMuted(_ muted: Bool) {
        self.muted = muted
        engine?.inputNode.isVoiceProcessingInputMuted = muted
    }

    func receive(sequence: UInt32, data: Data) {
        guard !data.isEmpty, data.count <= 1_275 else { return }
#if DEBUG
        if !loggedPlayback { loggedPlayback = true; NSLog("IrisCall first remote audio queued") }
#endif
        codec?.queue(sequence: sequence, data: data)
    }

    private func playout(completion: @escaping () -> Void) -> Bool {
        guard let codec, let player,
              let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 960),
              let channel = buffer.floatChannelData?[0] else { return false }
        let samples = codec.playout()
        guard samples.count == 960 else { return false }
#if DEBUG
        if !loggedPlayed && samples.contains(where: { $0 != 0 }) {
            loggedPlayed = true
            NSLog("IrisCall first non-silent remote audio played")
        }
#endif
        buffer.frameLength = 960
        for index in samples.indices { channel[index] = Float(samples[index]) / 32768 }
        player.scheduleBuffer(buffer, completionCallbackType: .dataConsumed) { _ in completion() }
        return true
    }

    func stop() {
        playoutBuffers?.stop()
        playoutBuffers = nil
        if let configurationObserver { NotificationCenter.default.removeObserver(configurationObserver) }
        configurationObserver = nil
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        player?.stop()
        engine = nil
        player = nil
        codec = nil
    }

    deinit { stop() }
}
