#if DEBUG
import AVFoundation
import Darwin

/// Opt-in diagnostic only. No captured samples, call IDs or account data leave
/// the audio graph. Fixed atomic storage keeps callbacks allocation/log/lock free.
/// OSAtomic is used for the iOS 16 deployment target (Swift Atomics is not a dependency).
final class IrisCallAudioContinuityProbe: @unchecked Sendable {
    enum Stage: String, CaseIterable {
        case captureCallbacks, captureFrames, capturePermissionRejected, captureConversionRejected
        case captureBatches, captureBatchFrames, captureOversizeBatches, captureBusyBatches
        case captureExpiredBatches, captureExpiredFrames, encodePermissionRejected, encodeRejected, encodedFrames
        case controllerEnqueued, controllerFull, controllerRejected, controllerDeadlineChecksExpired
        case dispatchEnqueued, dispatchFull, dispatchRejected, dispatchDeadlineRejected, ffiAttempted, ffiReturned, ffiFailed
        case receivePackets, receiveBusy, receiveSessionRejected, receiveCodecPackets
        case playoutFrames, playoutSamples, playoutEnergyMicro, scheduledFrames
        case captureSamples, captureEnergyMicro, renderCallbacks, renderSamples, renderEnergyMicro
        case invalidPCM, truncatedPCM, invalidHostTime, atomicObservationLost
        case watchdogTicks, renderMissingTicks, engineStopped, engineGenerations, routeGenerations, sessionActivations, sessionDeactivations
    }

    private final class Cell {
        private let value = UnsafeMutablePointer<Int64>.allocate(capacity: 1)
        init() { value.initialize(to: 0) }
        deinit { value.deinitialize(count: 1); value.deallocate() }
        var read: Int64 { OSAtomicAdd64Barrier(0, value) }
        func add(_ amount: Int64 = 1) { OSAtomicAdd64Barrier(amount, value) }
        func replace(_ old: Int64, _ new: Int64) -> Bool { OSAtomicCompareAndSwap64Barrier(old, new, value) }
        func maximum(_ next: Int64) -> Bool {
            // Bounded even if a callback overlaps a route transition.
            for _ in 0..<8 {
                let old = read
                if old >= next || replace(old, next) { return true }
            }
            return false
        }
    }

    static let shared = IrisCallAudioContinuityProbe()
    private static let stages = Stage.allCases
    private let cells = Stage.allCases.map { _ in Cell() }
    private let active = Cell()
    private let begun = Cell()
    private let expires = Cell()
    private let firstReceive = Cell()
    private let lastReceive = Cell()
    private let lastRender = Cell()
    private let maxRenderGap = Cell()
    private let maxWatchdogGap = Cell()
    private let firstRender = Cell()
    private let mixerPCM = PCMRun(renderClock: true)
    private let playoutPCM = PCMRun(renderClock: false)
    private let route = Cell()
    private let renderExpectedSince = Cell()
    private let observer = DispatchQueue(label: "iris.call.continuity.observer", qos: .utility)
    private var timer: DispatchSourceTimer?
    private var reports: [[String: Any]] = [] // <= 91 one-second aggregate records
    private var started: UInt64 = 0
    private var tickCount = 0
    private var routeObserver: NSObjectProtocol?
    private let now: () -> UInt64

    init(now: @escaping () -> UInt64 = { DispatchTime.now().uptimeNanoseconds }) { self.now = now }

    static func accepts(_ env: [String: String]) -> Bool {
        guard env["IRIS_CALL_CONTINUITY_PROBE"] == "1",
              env["IRIS_IOS_HARNESS_ACTION"] == "call_audio_continuity",
              let id = env["IRIS_IOS_HARNESS_RUN_ID"],
              id.range(of: "^audio-continuity-[A-Za-z0-9-]{8,64}$", options: .regularExpression) != nil,
              env["IRIS_UI_TEST_RUN_ID"] == id + "-shell" else { return false }
        return ["IRIS_IOS_HARNESS_USE_APP_STORAGE", "IRIS_IOS_HARNESS_USE_KEYCHAIN", "IRIS_IOS_HARNESS_RESET",
                "IRIS_UI_TEST_RESET", "IRIS_UI_TEST_DATA_DIR", "IRIS_IOS_HARNESS_SERVICE"].allSatisfy { env[$0] == nil }
    }

    /// Must run on the fixture's main thread before starting any call. One run per process.
    func begin(environment: [String: String], duration: Int = 60, startTimer: Bool = true) throws {
        guard Self.accepts(environment), (15...90).contains(duration), begun.replace(0, 1) else {
            throw NSError(domain: "CallContinuityFixture", code: 1)
        }
        started = now()
        expires.add(Int64(started + UInt64(duration) * 1_000_000_000))
        active.add()
#if os(iOS)
        routeChanged()
        routeObserver = NotificationCenter.default.addObserver(forName: AVAudioSession.routeChangeNotification,
            object: nil, queue: .main) { [weak self] _ in self?.routeChanged() }
#endif
        guard startTimer else { return }
        let timer = DispatchSource.makeTimerSource(queue: observer)
        timer.schedule(deadline: .now() + .milliseconds(100), repeating: .milliseconds(100))
        timer.setEventHandler { [weak self] in self?.tick() }
        self.timer = timer
        timer.resume()
    }

    var enabled: Bool { active.read == 1 && Int64(now()) <= expires.read }

    private func cell(_ stage: Stage) -> Cell {
        // Fixed enum lookup, bounded independently of frames or call duration.
        cells[Self.stages.firstIndex(of: stage)!]
    }

    func event(_ stage: Stage, _ amount: Int = 1) {
        guard enabled, amount >= 0 else { return }
        cell(stage).add(Int64(amount))
        if stage == .engineGenerations {
            if !renderExpectedSince.maximum(Int64(now())) { cell(.atomicObservationLost).add() }
        } else if stage == .engineStopped {
            for _ in 0..<8 { if renderExpectedSince.replace(renderExpectedSince.read, 0) { return } }
            cell(.atomicObservationLost).add()
        }
        if stage == .receiveCodecPackets {
            let stamp = Int64(now())
            _ = firstReceive.replace(0, stamp)
            if !lastReceive.maximum(stamp) { cell(.atomicObservationLost).add() }
        }
    }

    func decoded(_ samples: [Int16]) {
        guard enabled else { return }
        guard samples.count == 960 else { event(.invalidPCM); return }
        var energy = 0.0, coverage = PCMCoverage()
        for sample in samples {
            let value = Double(sample) / 32768
            energy += value * value
            coverage.add(peak: abs(value))
        }
        if !playoutPCM.observe(coverage, rate: 48_000, hostNs: Int64(now())) { event(.atomicObservationLost) }
        event(.playoutFrames); event(.playoutSamples, samples.count)
        event(.playoutEnergyMicro, Int(energy * 1_000_000))
    }

    /// Called directly by existing capture and actual mainMixerNode taps. Scan
    /// all samples only for <= 24576 capture / 8192 mixer frames and <= 2
    /// noninterleaved float channels. Capture includes the supported400ms batch.
    /// Unsupported/truncated observations invalidate completeness, never guess.
    func pcm(_ buffer: AVAudioPCMBuffer, time: AVAudioTime, render: Bool) {
        guard enabled else { return }
        let stamp = Int64(now())
        if render {
            event(.renderCallbacks)
            if !time.isHostTimeValid { event(.invalidHostTime) }
            else {
                let seconds = AVAudioTime.seconds(forHostTime: time.hostTime)
                if seconds.isFinite, seconds >= 0, seconds < Double(Int64.max) / 1e9 {
                    renderAt(Int64(seconds * 1e9))
                } else { event(.invalidHostTime) }
            }
            // Wall-time arrival is independent from the supplied buffer host time.
            if !lastRender.maximum(stamp) { event(.atomicObservationLost) }
        } else { event(.captureCallbacks); event(.captureFrames, Int(buffer.frameLength)) }
        let frames = Int(buffer.frameLength), channels = Int(buffer.format.channelCount)
        guard frames > 0, frames <= (render ? 8192 : 24576), (1...2).contains(channels),
              !buffer.format.isInterleaved, let data = buffer.floatChannelData else {
            event(.truncatedPCM); return
        }
        var energy = 0.0, coverage = PCMCoverage()
        for index in 0..<frames {
            var peak = 0.0
            for channel in 0..<channels {
                let sample = Double(data[channel][index])
                guard sample.isFinite, abs(sample) <= 16 else { event(.invalidPCM); return }
                energy += sample * sample
                peak = max(peak, abs(sample))
            }
            coverage.add(peak: peak)
        }
        if render, time.isHostTimeValid {
            let rate = buffer.format.sampleRate
            guard rate.isFinite, (8_000...192_000).contains(rate) else { event(.invalidPCM); return }
            let hostNs = AVAudioTime.seconds(forHostTime: time.hostTime) * 1e9
            guard hostNs.isFinite, hostNs >= 0, hostNs < Double(Int64.max) else { event(.invalidHostTime); return }
            if !mixerPCM.observe(coverage, rate: rate, hostNs: Int64(hostNs)) { event(.atomicObservationLost) }
        }
        event(render ? .renderSamples : .captureSamples, frames * channels)
        event(render ? .renderEnergyMicro : .captureEnergyMicro, Int(energy * 1_000_000))
    }

    private let lastRenderHost = Cell()
    func renderAt(_ hostNs: Int64) {
        guard enabled else { return }
        _ = firstRender.replace(0, hostNs)
        for _ in 0..<8 {
            let old = lastRenderHost.read
            guard hostNs > old else { event(.invalidHostTime); return }
            if lastRenderHost.replace(old, hostNs) {
                if old > 0, !maxRenderGap.maximum(hostNs - old) { event(.atomicObservationLost) }
                return
            }
        }
        event(.atomicObservationLost)
    }

    /// Runs independently of the media queue and all audio callbacks.
    func tick() {
        guard active.read == 1 else { return }
        let stamp = now(), expected = renderExpectedSince.read
        let previous = max(lastRender.read, expected)
        let gap = stamp >= UInt64(max(0, previous)) ? stamp - UInt64(max(0, previous)) : 0
        cell(.watchdogTicks).add()
        if expected > 0 && gap > 200_000_000 {
            cell(.renderMissingTicks).add()
            if !maxWatchdogGap.maximum(Int64(gap)) { cell(.atomicObservationLost).add() }
        }
        tickCount += 1
        if tickCount % 10 == 0, reports.count < 90 { reports.append(snapshot()) }
        if Int64(stamp) >= expires.read { _ = active.replace(1, 0); timer?.cancel() }
    }

    func snapshot() -> [String: Any] {
        let before = now()
        var counts: [String: Int64] = [:]
        for (stage, value) in zip(Self.stages, cells) { counts[stage.rawValue] = value.read }
        let mixer = mixerPCM.snapshot(), playout = playoutPCM.snapshot()
        return ["elapsedNs": before - started, "sampledAtUptimeNs": before, "snapshotSpanNs": now() - before,
                "counts": counts, "firstReceiveNs": firstReceive.read, "lastReceiveNs": lastReceive.read,
                "firstRenderHostNs": firstRender.read, "lastRenderHostNs": lastRenderHost.read,
                "maxRenderHostGapNs": maxRenderGap.read, "maxWatchdogRenderGapNs": maxWatchdogGap.read,
                "outputRouteCode": route.read, "mixerPCM": mixer, "playoutPCM": playout,
                "observationsComplete": mixer["snapshotValid"] as? Bool == true && playout["snapshotValid"] as? Bool == true &&
                    now() - before <= 20_000_000 &&
                    [.invalidPCM, .truncatedPCM, .invalidHostTime, .atomicObservationLost].allSatisfy { cell($0).read == 0 }]
    }

    func finish() -> [String: Any] {
        _ = active.replace(1, 0)
        timer?.cancel()
        if let routeObserver { NotificationCenter.default.removeObserver(routeObserver) }
        routeObserver = nil
        return observer.sync {
            reports.append(snapshot())
            return ["schema": 1, "samples": reports, "voiceProcessingAEC": true,
                    "externalSpeakerAudibilityProven": false, "secondPhoneCellularProven": false]
        }
    }

    /// -80 dBFS amplitude threshold. Silence is diagnostic, not evidence of
    /// loss: natural pauses, Opus DTX and voice-processing AEC also produce it.
    private struct PCMCoverage {
        var frames: Int64 = 0, silent: Int64 = 0, leading: Int64 = 0, trailing: Int64 = 0
        var maximum: Int64 = 0, maximumStart: Int64 = 0
        mutating func add(peak: Double) {
            frames += 1
            if peak <= 0.0001 {
                silent += 1; trailing += 1
                if silent == frames { leading += 1 }
                if trailing > maximum { maximum = trailing; maximumStart = frames - trailing }
            } else { trailing = 0 }
        }
    }

    /// One serialized graph callback is the writer. Readers use a bounded
    /// sequence check; a route overlap loses a diagnostic, never blocks audio.
    private final class PCMRun {
        private let renderClock: Bool
        private let version = Cell(), rate = Cell(), frames = Cell(), silent = Cell()
        private let previousEnd = Cell(), trailing = Cell(), first = Cell(), observed = Cell()
        private let maximumNs = Cell(), maximumStart = Cell(), maximumEnd = Cell(), discontinuities = Cell()
        init(renderClock: Bool) { self.renderClock = renderClock }
        private func set(_ cell: Cell, _ value: Int64) { cell.add(value - cell.read) }
        func observe(_ batch: PCMCoverage, rate sampleRate: Double, hostNs: Int64) -> Bool {
            let sequence = version.read
            guard sequence % 2 == 0, version.replace(sequence, sequence + 1) else { return false }
            defer { version.add() }
            let oldRate = rate.read, oldFrames = frames.read
            let rateMilliHz = Int64(sampleRate * 1000)
            if first.read == 0 { set(first, hostNs) }
            // Playout coverage uses a scheduled sample clock. Its observation
            // time is retained separately; it is not a receive/acoustic clock.
            let start = renderClock ? hostNs : first.read + Int64(Double(oldFrames) * 1e9 / sampleRate)
            let end = start + Int64(Double(batch.frames) * 1e9 / sampleRate)
            let contiguous = oldRate == 0 || oldRate == rateMilliHz &&
                (!renderClock || abs(start - previousEnd.read) <= 2_000_000)
            if !contiguous { discontinuities.add(); set(trailing, 0) }
            let previousSilent = trailing.read
            let combined = previousSilent + batch.leading
            let longest = max(combined, batch.maximum)
            let longestStart = combined > batch.maximum ? start - Int64(Double(previousSilent) * 1e9 / sampleRate) :
                start + Int64(Double(batch.maximumStart) * 1e9 / sampleRate)
            let duration = Int64(Double(longest) * 1e9 / sampleRate)
            if duration > maximumNs.read {
                set(maximumNs, duration); set(maximumStart, longestStart); set(maximumEnd, longestStart + duration)
            }
            set(trailing, batch.silent == batch.frames ? previousSilent + batch.frames : batch.trailing)
            frames.add(batch.frames); silent.add(batch.silent)
            set(rate, rateMilliHz); set(previousEnd, end); set(observed, hostNs)
            return true
        }
        func snapshot() -> [String: Any] {
            for _ in 0..<3 {
                let before = version.read
                guard before % 2 == 0 else { continue }
                let value: [String: Any] = ["clock": renderClock ? "buffer-host-time" : "scheduled-sample-clock",
                    "silenceAmplitudeThreshold": 0.0001, "sampleRateMilliHz": rate.read,
                    "frames": frames.read, "silentFrames": silent.read, "voicedFrames": frames.read - silent.read,
                    "maximumSilentDurationNs": maximumNs.read, "maximumSilentStartNs": maximumStart.read,
                    "maximumSilentEndNs": maximumEnd.read, "lastObservedUptimeNs": observed.read,
                    "trailingSilentDurationNs": rate.read > 0 ? Int64(Double(trailing.read) * 1e12 / Double(rate.read)) : 0,
                    "trailingSilentEndNs": previousEnd.read,
                    "trailingSilentStartNs": rate.read > 0 ? previousEnd.read - Int64(Double(trailing.read) * 1e12 / Double(rate.read)) : 0,
                    "timelineDiscontinuities": discontinuities.read, "snapshotValid": true]
                if before == version.read { return value }
            }
            return ["snapshotValid": false]
        }
    }

#if os(iOS)
    private func routeChanged() {
        event(.routeGenerations)
        let outputs = AVAudioSession.sharedInstance().currentRoute.outputs.map(\.portType)
        // Stable categories only: no route UID/name/device identifier.
        let code: Int64 = outputs.contains(.builtInSpeaker) ? 1 : outputs.contains(.builtInReceiver) ? 2 :
            outputs.contains(.headphones) ? 3 : outputs.contains(.bluetoothHFP) ? 4 : 0
        for _ in 0..<8 { if route.replace(route.read, code) { return } }
        event(.atomicObservationLost)
    }
#endif
}
#endif
