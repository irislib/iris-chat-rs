#if os(macOS)
import XCTest
import CoreVideo
@testable import IrisChatMac

@MainActor
private final class ScreenCaptureProbe: IrisScreenShareCapturing {
    var ready: ((Bool, String?) -> Void)?
    var stopped: ((String?) -> Void)?
    var picks = 0
    var stops = 0
    func choose(frame: @escaping (CVPixelBuffer, UInt64) -> Void,
                ready: @escaping (Bool, String?) -> Void, stopped: @escaping (String?) -> Void) {
        picks += 1; self.ready = ready; self.stopped = stopped
    }
    func stop() { stops += 1 }
}

final class ScreenShareTests: XCTestCase {
    @MainActor
    func testPickerCancelAndFailureLeaveCallVideoUnchanged() {
        let capture = ScreenCaptureProbe()
        var changes: [Bool] = [], errors: [String] = []
        let session = IrisCallScreenShareSession(capture: capture, changeVideo: { _, video, _ in changes.append(video) },
            frame: { _, _, _ in }, changed: {}, error: { errors.append($0) })
        session.choose(callID: "call", connected: true, videoCapable: true, camera: false)
        XCTAssertTrue(session.isChoosing)
        capture.ready?(false, nil)
        XCTAssertFalse(session.isChoosing); XCTAssertFalse(session.isSharing)
        XCTAssertTrue(changes.isEmpty); XCTAssertTrue(errors.isEmpty)
        session.choose(callID: "call", connected: true, videoCapable: true, camera: true)
        capture.ready?(false, "Permission denied")
        XCTAssertTrue(changes.isEmpty); XCTAssertEqual(errors, ["Permission denied"])
    }

    @MainActor
    func testStopAndSystemStopRestorePreviousCameraState() {
        for camera in [false, true] {
            let capture = ScreenCaptureProbe()
            var changes: [(video: Bool, screen: Bool)] = []
            let session = IrisCallScreenShareSession(capture: capture,
                changeVideo: { _, video, screen in changes.append((video, screen)) },
                frame: { _, _, _ in }, changed: {}, error: { XCTFail($0) })
            session.choose(callID: "call", connected: true, videoCapable: true, camera: camera)
            capture.ready?(true, nil)
            XCTAssertTrue(session.isSharing)
            XCTAssertEqual(changes.map(\.video), [true])
            if camera { session.stop() } else { capture.stopped?(nil) }
            XCTAssertFalse(session.isSharing)
            XCTAssertEqual(changes.map(\.video), [true, camera])
            XCTAssertEqual(changes.map(\.screen), [true, false])
            XCTAssertEqual(capture.stops, 1)
        }
    }

    @MainActor
    func testCallEndWhilePickerOpenRejectsLateSelectionIncludingAfterNextCall() {
        let capture = ScreenCaptureProbe()
        var changes = 0
        let session = IrisCallScreenShareSession(capture: capture, changeVideo: { _, _, _ in changes += 1 },
            frame: { _, _, _ in }, changed: {}, error: { XCTFail($0) })
        session.choose(callID: "old", connected: true, videoCapable: true, camera: true)
        let oldReady = capture.ready
        session.reconcile(callID: nil, connected: false)
        oldReady?(true, nil)
        XCTAssertEqual(changes, 0)
        session.choose(callID: "new", connected: true, videoCapable: true, camera: false)
        oldReady?(true, nil)
        XCTAssertTrue(session.isChoosing); XCTAssertFalse(session.isSharing)
        XCTAssertEqual(changes, 0)
        capture.ready?(true, nil)
        XCTAssertEqual(changes, 1)
    }

    @MainActor
    func testEndWhileSharingStopsWithoutRestoringCamera() {
        let capture = ScreenCaptureProbe()
        var enabled: [Bool] = []
        let session = IrisCallScreenShareSession(capture: capture, changeVideo: { _, video, _ in enabled.append(video) },
            frame: { _, _, _ in }, changed: {}, error: { XCTFail($0) })
        session.choose(callID: "call", connected: true, videoCapable: true, camera: true)
        capture.ready?(true, nil)
        session.reconcile(callID: "call", connected: false)
        XCTAssertEqual(enabled, [true, false])
        XCTAssertFalse(session.isSharing)
        capture.stopped?("Late system error")
        XCTAssertEqual(enabled, [true, false])
    }

    @MainActor
    func testOnlyConnectedVideoCapableCallCanOpenPicker() {
        let capture = ScreenCaptureProbe()
        let session = IrisCallScreenShareSession(capture: capture, changeVideo: { _, _, _ in XCTFail() },
            frame: { _, _, _ in }, changed: {}, error: { XCTFail($0) })
        session.choose(callID: "call", connected: false, videoCapable: true, camera: true)
        session.choose(callID: "call", connected: true, videoCapable: false, camera: false)
        XCTAssertEqual(capture.picks, 0)
    }

    func testSourceSwitchRevokesQueuedFramesEvenWhenSwitchingBack() {
        let gate = IrisCallVideoSourceGate()
        let camera = gate.permission(screen: false, capturedAt: .max)
        XCTAssertTrue(camera())
        gate.select(screen: true)
        XCTAssertFalse(camera())
        XCTAssertFalse(gate.permission(screen: false, capturedAt: .max)())
        let screen = gate.permission(screen: true, capturedAt: .max)
        XCTAssertTrue(screen())
        gate.select(screen: false)
        XCTAssertFalse(camera()); XCTAssertFalse(screen())
        XCTAssertFalse(gate.permission(screen: false, capturedAt: 0)())
        XCTAssertTrue(gate.permission(screen: false, capturedAt: .max)())
    }

    @MainActor
    func testCaptureDimensionsFitExistingH264LimitsAndPreserveAspect() {
        XCTAssertEqual(IrisMacScreenShare.captureSize(CGSize(width: 2560, height: 1440), pixelScale: 2), CGSize(width: 1920, height: 1080))
        XCTAssertEqual(IrisMacScreenShare.captureSize(CGSize(width: 1080, height: 1920), pixelScale: 1), CGSize(width: 606, height: 1080))
        XCTAssertEqual(IrisMacScreenShare.captureSize(CGSize(width: 640, height: 480), pixelScale: 1), CGSize(width: 640, height: 480))
    }
}
#endif
