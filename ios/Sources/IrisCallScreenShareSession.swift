#if os(macOS)
import CoreVideo
import Foundation

@MainActor
protocol IrisScreenShareCapturing: AnyObject {
    func choose(frame: @escaping (CVPixelBuffer, UInt64) -> Void,
                ready: @escaping (Bool, String?) -> Void,
                stopped: @escaping (String?) -> Void)
    func stop()
}

/// Owns user intent separately from asynchronous picker/capture callbacks.
@MainActor
final class IrisCallScreenShareSession {
    private(set) var isChoosing = false
    private(set) var isSharing = false
    private var request: UUID?
    private var callID: String?
    private var previousCamera = false
    private let capture: IrisScreenShareCapturing
    private let changeVideo: (String, Bool, Bool) -> Void
    private let frame: (String, CVPixelBuffer, UInt64) -> Void
    private let changed: () -> Void
    private let error: (String) -> Void

    init(capture: IrisScreenShareCapturing,
         changeVideo: @escaping (String, Bool, Bool) -> Void,
         frame: @escaping (String, CVPixelBuffer, UInt64) -> Void,
         changed: @escaping () -> Void, error: @escaping (String) -> Void) {
        self.capture = capture; self.changeVideo = changeVideo
        self.frame = frame; self.changed = changed; self.error = error
    }

    func choose(callID: String, connected: Bool, videoCapable: Bool, camera: Bool) {
        guard connected, videoCapable, request == nil else { return }
        let request = UUID()
        self.request = request; self.callID = callID
        previousCamera = camera; isChoosing = true
        changed()
        // Frame delivery stays on the capture/media queues, never the UI queue.
        let frame = self.frame
        capture.choose(frame: { pixel, timestamp in frame(callID, pixel, timestamp) }, ready: { [weak self] started, message in
            guard let self, self.request == request else { return }
            if !started {
                self.stop()
                if let message { self.error(message) }
                return
            }
            self.isChoosing = false; self.isSharing = true
            self.changeVideo(callID, true, true)
            self.changed()
        }, stopped: { [weak self] message in
            guard let self, self.request == request else { return }
            self.stop()
            if let message { self.error(message) }
        })
    }

    func reconcile(callID: String?, connected: Bool) {
        if self.callID != callID || !connected { stop(restoreCamera: false) }
    }

    func stop(restoreCamera: Bool = true) {
        guard request != nil else { return }
        request = nil
        let wasSharing = isSharing
        isChoosing = false; isSharing = false
        if wasSharing, let callID { changeVideo(callID, restoreCamera && previousCamera, false) }
        capture.stop()
        callID = nil
        changed()
    }
}
#endif
