#if os(iOS) || os(macOS)
import AVFoundation
import SwiftUI

enum IrisAudioWaveform {
    static let barCount = 47

    private static let worker = IrisAudioWaveformWorker()

    static func decode(_ url: URL, cacheKey: String? = nil) async -> [Float] {
        let task = Task.detached(priority: .utility) {
            await worker.decode(url, cacheKey: cacheKey)
        }
        return await withTaskCancellationHandler { await task.value } onCancel: { task.cancel() }
    }

    // Read PCM in small blocks. Waveform analysis is optional and must stay cheap.
    static func sample(_ url: URL) -> [Float] {
        do {
            try Task.checkCancellation()
            let file = try AVAudioFile(forReading: url)
            guard file.length > 0, file.processingFormat.sampleRate > 0,
                  Double(file.length) / file.processingFormat.sampleRate <= 15 * 60,
                  let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 4096)
            else { return [] }
            var sums = [Double](repeating: 0, count: barCount)
            var counts = [Int](repeating: 0, count: barCount)
            let deadline = ContinuousClock.now.advanced(by: .seconds(3))
            while file.framePosition < file.length {
                try Task.checkCancellation()
                guard ContinuousClock.now < deadline else { return [] }
                let start = file.framePosition
                try file.read(into: buffer)
                guard buffer.frameLength > 0, let channels = buffer.floatChannelData else { break }
                for frame in 0..<Int(buffer.frameLength) {
                    let bin = min(barCount - 1, Int((start + Int64(frame)) * Int64(barCount) / file.length))
                    for channel in 0..<Int(buffer.format.channelCount) {
                        let value = abs(channels[channel][frame])
                        if value.isFinite { sums[bin] += Double(value) * Double(value); counts[bin] += 1 }
                    }
                }
            }
            var peaks = sums.enumerated().map { Float(sqrt($0.element / Double(max(1, counts[$0.offset])))) }
            if let maximum = peaks.max(), maximum > 0.0001 { peaks = peaks.map { $0 / maximum } }
            return peaks
        } catch { return [] }
    }
}

/// No suspension during sampling: only one waveform is decoded at a time, off the UI thread.
/// Content-addressed message keys let recreated cells reuse both successful and skipped results.
actor IrisAudioWaveformWorker {
    private var cache: [String: [Float]] = [:]
    private var recency: [String] = []
    private let capacity: Int
    private let sample: @Sendable (URL) -> [Float]

    init(capacity: Int = 128, sample: @escaping @Sendable (URL) -> [Float] = { IrisAudioWaveform.sample($0) }) {
        self.capacity = max(1, capacity)
        self.sample = sample
    }

    func decode(_ url: URL, cacheKey: String?) -> [Float] {
        guard !Task.isCancelled else { return [] }
        if let key = cacheKey, let result = cache[key] {
            touch(key)
            return result
        }
        let result = sample(url)
        guard !Task.isCancelled else { return [] }
        if let key = cacheKey {
            cache[key] = result
            touch(key)
            while recency.count > capacity { cache.removeValue(forKey: recency.removeFirst()) }
        }
        return result
    }

    private func touch(_ key: String) {
        recency.removeAll { $0 == key }
        recency.append(key)
    }
}

#if os(macOS)
import AppKit

struct IrisWaveformSlider: NSViewRepresentable {
    @Binding var value: Double
    let duration: Double
    let peaks: [Float]
    let color: Color
    let enabled: Bool

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSSlider {
        let slider = NSSlider()
        slider.cell = WaveformCell()
        slider.target = context.coordinator
        slider.action = #selector(Coordinator.changed(_:))
        slider.isContinuous = true
        slider.setAccessibilityLabel("Audio position")
        slider.setAccessibilityIdentifier("chatAudioProgress")
        return slider
    }
    func updateNSView(_ slider: NSSlider, context: Context) {
        context.coordinator.parent = self
        slider.minValue = 0
        slider.maxValue = max(1, duration)
        slider.doubleValue = value
        slider.isEnabled = enabled
        if let cell = slider.cell as? WaveformCell {
            cell.peaks = peaks
            cell.color = NSColor(color)
        }
        slider.needsDisplay = true
    }
    final class Coordinator: NSObject {
        var parent: IrisWaveformSlider
        init(_ parent: IrisWaveformSlider) { self.parent = parent }
        @objc func changed(_ sender: NSSlider) { parent.value = sender.doubleValue }
    }
}

private final class WaveformCell: NSSliderCell {
    var peaks: [Float] = []
    var color: NSColor = .labelColor
    override func drawBar(inside rect: NSRect, flipped: Bool) {
        let step = rect.width / CGFloat(IrisAudioWaveform.barCount)
        let fraction = (doubleValue - minValue) / max(1, maxValue - minValue)
        for index in 0..<IrisAudioWaveform.barCount {
            let height = 3 + CGFloat(index < peaks.count ? peaks[index] : 0) * 21
            color.withAlphaComponent((Double(index) + 0.5) / Double(IrisAudioWaveform.barCount) <= fraction ? 1 : 0.35).setFill()
            NSBezierPath(roundedRect: NSRect(x: rect.minX + CGFloat(index) * step, y: rect.midY - height / 2,
                                             width: max(1, step - 1.5), height: height), xRadius: 1, yRadius: 1).fill()
        }
    }
    override func drawKnob(_ rect: NSRect) {
        guard isEnabled else { return }
        color.setFill()
        NSBezierPath(ovalIn: NSRect(x: rect.midX - 3, y: rect.midY - 3, width: 6, height: 6)).fill()
    }
}
#else
import UIKit

struct IrisWaveformSlider: UIViewRepresentable {
    @Binding var value: Double
    let duration: Double
    let peaks: [Float]
    let color: Color
    let enabled: Bool

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> WaveformSliderView {
        let slider = WaveformSliderView()
        slider.addTarget(context.coordinator, action: #selector(Coordinator.changed(_:)), for: .valueChanged)
        slider.accessibilityLabel = "Audio position"
        slider.accessibilityIdentifier = "chatAudioProgress"
        return slider
    }
    func updateUIView(_ slider: WaveformSliderView, context: Context) {
        context.coordinator.parent = self
        slider.minimumValue = 0
        slider.maximumValue = Float(max(1, duration))
        slider.value = Float(value)
        slider.isEnabled = enabled
        slider.peaks = peaks
        slider.waveColor = UIColor(color)
        slider.accessibilityValue = "\(Int(value)) of \(Int(duration)) seconds"
        slider.setNeedsLayout()
    }
    final class Coordinator: NSObject {
        var parent: IrisWaveformSlider
        init(_ parent: IrisWaveformSlider) { self.parent = parent }
        @objc func changed(_ sender: UISlider) { parent.value = Double(sender.value) }
    }
}

final class WaveformSliderView: UISlider {
    var peaks: [Float] = []
    var waveColor: UIColor = .label {
        didSet { if waveColor != oldValue { updateThumb() } }
    }
    private let bars = (0..<IrisAudioWaveform.barCount).map { _ in CALayer() }
    override init(frame: CGRect) {
        super.init(frame: frame)
        minimumTrackTintColor = .clear
        maximumTrackTintColor = .clear
        for bar in bars { bar.cornerRadius = 1; layer.addSublayer(bar) }
        updateThumb()
    }
    private func seek(at point: CGPoint) {
        let track = trackRect(forBounds: bounds).insetBy(dx: 2, dy: 0)
        let fraction = min(1, max(0, (point.x - track.minX) / max(1, track.width)))
        value = minimumValue + Float(fraction) * (maximumValue - minimumValue)
        setNeedsLayout()
        sendActions(for: .valueChanged)
    }
    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        seek(at: touch.location(in: self))
        return true
    }
    override func continueTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        seek(at: touch.location(in: self))
        return true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layoutSubviews() {
        super.layoutSubviews()
        let track = trackRect(forBounds: bounds).insetBy(dx: 2, dy: 0)
        let fraction = CGFloat((value - minimumValue) / max(1, maximumValue - minimumValue))
        let step = track.width / CGFloat(bars.count)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (index, bar) in bars.enumerated() {
            let height = 3 + CGFloat(index < peaks.count ? peaks[index] : 0) * 21
            bar.backgroundColor = waveColor.withAlphaComponent((CGFloat(index) + 0.5) / CGFloat(bars.count) <= fraction ? 1 : 0.35).cgColor
            bar.frame = CGRect(x: track.minX + CGFloat(index) * step, y: bounds.midY - height / 2,
                               width: max(1, step - 1.5), height: height)
        }
        CATransaction.commit()
    }
    private func updateThumb() {
        let thumb = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8)).image { _ in
            waveColor.setFill()
            UIBezierPath(ovalIn: CGRect(x: 1, y: 1, width: 6, height: 6)).fill()
        }
        setThumbImage(thumb, for: .normal)
    }
}
#endif
#endif
