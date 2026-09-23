import AVFoundation
import SwiftUI

/// Keeps at most one pending pixel buffer while the UI is busy. Frames never
/// become published app state and never pass through JPEG or bitmap conversion.
final class IrisCallVideoSurface {
    private let lock = NSLock()
    private var callID: String?
    private var pending: CVPixelBuffer?
    private var scheduled = false
    private var display: ((CVPixelBuffer?) -> Void)?

    func setCallID(_ id: String?) {
        lock.lock()
        if callID != id { pending = nil }
        callID = id
        lock.unlock()
        if id == nil { DispatchQueue.main.async { [weak self] in self?.display?(nil) } }
    }

    func present(_ pixel: CVPixelBuffer, callID: String) {
        lock.lock()
        guard self.callID == callID else { lock.unlock(); return }
        pending = pixel
        guard !scheduled else { lock.unlock(); return }
        scheduled = true
        lock.unlock()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let pixel = self.pending
            self.pending = nil
            self.scheduled = false
            self.lock.unlock()
            self.display?(pixel)
        }
    }

    func attach(_ display: @escaping (CVPixelBuffer?) -> Void) { self.display = display }
    func detach() { display = nil }
}

private func displayCallFrame(_ pixel: CVPixelBuffer?, on layer: AVSampleBufferDisplayLayer) {
    guard let pixel else { layer.flushAndRemoveImage(); return }
    if layer.status == .failed { layer.flush() }
    guard layer.isReadyForMoreMediaData else { return }
    var format: CMVideoFormatDescription?
    guard CMVideoFormatDescriptionCreateForImageBuffer(allocator: kCFAllocatorDefault,
        imageBuffer: pixel, formatDescriptionOut: &format) == noErr, let format else { return }
    var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
    var sample: CMSampleBuffer?
    guard CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: pixel,
        formatDescription: format, sampleTiming: &timing, sampleBufferOut: &sample) == noErr, let sample else { return }
    if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true) {
        let dictionary = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
        CFDictionarySetValue(dictionary, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                             Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
    }
    layer.enqueue(sample)
}

struct IrisCallVideoView {
    let surface: IrisCallVideoSurface
    func makeCoordinator() -> IrisCallVideoSurface { surface }
}

#if os(iOS)
final class IrisCallVideoHost: UIView {
    override class var layerClass: AnyClass { AVSampleBufferDisplayLayer.self }
    var displayLayer: AVSampleBufferDisplayLayer { layer as! AVSampleBufferDisplayLayer }
}
extension IrisCallVideoView: UIViewRepresentable {
    func makeUIView(context: Context) -> IrisCallVideoHost {
        let view = IrisCallVideoHost()
        view.displayLayer.videoGravity = .resizeAspect
        surface.attach { [weak view] pixel in
            if let view { displayCallFrame(pixel, on: view.displayLayer) }
        }
        return view
    }
    func updateUIView(_ view: IrisCallVideoHost, context: Context) {}
    static func dismantleUIView(_ view: IrisCallVideoHost, coordinator: IrisCallVideoSurface) {
        coordinator.detach(); view.displayLayer.flushAndRemoveImage()
    }
}
#elseif os(macOS)
final class IrisCallVideoHost: NSView {
    let displayLayer = AVSampleBufferDisplayLayer()
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.addSublayer(displayLayer)
        displayLayer.videoGravity = .resizeAspect
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layout() { super.layout(); displayLayer.frame = bounds }
}
extension IrisCallVideoView: NSViewRepresentable {
    func makeNSView(context: Context) -> IrisCallVideoHost {
        let view = IrisCallVideoHost()
        surface.attach { [weak view] pixel in
            if let view { displayCallFrame(pixel, on: view.displayLayer) }
        }
        return view
    }
    func updateNSView(_ view: IrisCallVideoHost, context: Context) {}
    static func dismantleNSView(_ view: IrisCallVideoHost, coordinator: IrisCallVideoSurface) {
        coordinator.detach(); view.displayLayer.flushAndRemoveImage()
    }
}
#endif
