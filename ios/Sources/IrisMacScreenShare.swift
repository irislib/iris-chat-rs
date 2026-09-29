#if os(macOS)
import ScreenCaptureKit
import CoreMedia

@MainActor
final class IrisMacScreenShare: NSObject, IrisScreenShareCapturing, SCStreamDelegate {
    private var request: UUID?
    private var stream: SCStream?
    private var output: IrisScreenShareOutput?
    private var ready: ((Bool, String?) -> Void)?
    private var stopped: ((String?) -> Void)?
    private var observer: IrisScreenPickerObserver?
    private let queue = DispatchQueue(label: "iris.call.screen", qos: .userInteractive)

    func choose(frame: @escaping (CVPixelBuffer, UInt64) -> Void,
                ready: @escaping (Bool, String?) -> Void, stopped: @escaping (String?) -> Void) {
        stop()
        let request = UUID()
        self.request = request; self.ready = ready; self.stopped = stopped
        output = IrisScreenShareOutput(frame: frame)
        var configuration = SCContentSharingPickerConfiguration()
        configuration.allowedPickerModes = [.singleWindow, .singleDisplay]
        configuration.allowsChangingSelectedContent = false
        let picker = SCContentSharingPicker.shared
        picker.defaultConfiguration = configuration
        picker.maximumStreamCount = 1
        let observer = IrisScreenPickerObserver { [weak self] filter, message in
            guard let self, self.request == request, self.stream == nil else { return }
            if let filter { self.start(filter: filter, request: request) }
            else { self.ready?(false, message) }
        }
        self.observer = observer
        picker.add(observer)
        picker.isActive = true
        picker.present()
    }

    func stop() {
        request = nil
        ready = nil; stopped = nil
        output?.stop(); output = nil
        let previous = stream; stream = nil
        if let observer { SCContentSharingPicker.shared.remove(observer) }
        observer = nil
        SCContentSharingPicker.shared.isActive = false
        if let previous { Task { try? await previous.stopCapture() } }
    }

    private func start(filter: SCContentFilter, request: UUID) {
        Task { @MainActor in
            guard self.request == request, self.stream == nil, let output = self.output else { return }
            let configuration = Self.configuration(for: filter)
            let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
            self.stream = stream
            do {
                try stream.addStreamOutput(output, type: .screen, sampleHandlerQueue: self.queue)
                try await stream.startCapture()
                guard self.request == request else { try? await stream.stopCapture(); return }
                self.ready?(true, nil)
                self.ready = nil
            } catch {
                guard self.request == request else { return }
                self.ready?(false, "Couldn’t share the screen. Try choosing it again.")
            }
        }
    }

    nonisolated func stream(_ stream: SCStream, didStopWithError error: Error) {
        Task { @MainActor in
            guard self.stream === stream else { return }
            let stoppedByUser = (error as NSError).code == SCStreamError.userStopped.rawValue
            self.stopped?(stoppedByUser ? nil : "Screen sharing stopped.")
        }
    }

    nonisolated func streamDidBecomeInactive(_ stream: SCStream) {
        Task { @MainActor in
            guard self.stream === stream else { return }
            self.stopped?(nil)
        }
    }

    static func configuration(for filter: SCContentFilter) -> SCStreamConfiguration {
        let configuration = SCStreamConfiguration()
        let size = captureSize(filter.contentRect.size, pixelScale: Double(filter.pointPixelScale))
        configuration.width = Int(size.width); configuration.height = Int(size.height)
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 15)
        configuration.queueDepth = 3
        configuration.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        configuration.showsCursor = true
        configuration.preservesAspectRatio = true
        configuration.capturesAudio = false
        return configuration
    }

    static func captureSize(_ size: CGSize, pixelScale: Double) -> CGSize {
        let width = max(2, size.width * pixelScale), height = max(2, size.height * pixelScale)
        let scale = min(1, min(1920 / width, 1080 / height))
        return CGSize(width: max(2, Int(width * scale) / 2 * 2), height: max(2, Int(height * scale) / 2 * 2))
    }

    deinit {
        output?.stop()
        let stream = stream, observer = observer
        Task { @MainActor in
            if let observer { SCContentSharingPicker.shared.remove(observer) }
            try? await stream?.stopCapture()
        }
    }
}

/// Each picker invocation gets its own observer, so a queued callback from an
/// old picker cannot select content for a later call.
private final class IrisScreenPickerObserver: NSObject, SCContentSharingPickerObserver {
    private let selected: @MainActor (SCContentFilter?, String?) -> Void
    init(selected: @escaping @MainActor (SCContentFilter?, String?) -> Void) { self.selected = selected }
    func contentSharingPicker(_ picker: SCContentSharingPicker, didCancelFor stream: SCStream?) {
        Task { @MainActor in selected(nil, nil) }
    }
    func contentSharingPickerStartDidFailWithError(_ error: Error) {
        Task { @MainActor in selected(nil, "Couldn’t open screen sharing.") }
    }
    func contentSharingPicker(_ picker: SCContentSharingPicker, didUpdateWith filter: SCContentFilter, for stream: SCStream?) {
        Task { @MainActor in selected(filter, nil) }
    }
}

private final class IrisScreenShareOutput: NSObject, SCStreamOutput {
    private let lock = NSLock()
    private var active = true
    private let frame: (CVPixelBuffer, UInt64) -> Void
    init(frame: @escaping (CVPixelBuffer, UInt64) -> Void) { self.frame = frame }
    func stop() { lock.lock(); active = false; lock.unlock() }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        lock.lock(); let active = self.active; lock.unlock()
        guard active, type == .screen, sampleBuffer.isValid,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let status = attachments.first?[.status] as? Int, status == SCFrameStatus.complete.rawValue,
              let pixel = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let time = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        let micros = CMTimeConvertScale(time, timescale: 1_000_000, method: .default).value
        frame(pixel, UInt64(max(0, micros)))
    }
}
#endif
