import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

private final class CallMediaProbe: IrisCallMediaHandling {
    var configurations: [(callID: String?, muted: Bool, video: Bool)] = []
    var started: [IrisCallMediaSession] = []
    var received: [String] = []
    var stopCount = 0

    func start(_ session: IrisCallMediaSession) { started.append(session) }
    func adapt(targetBitrate: UInt32, keyFrameGeneration: UInt32) {}
    func receive(callID: String, kind: UInt8, sequence: UInt32, timestampUs: UInt64, keyFrame: Bool, data: Data) { received.append(callID) }
    func configure(callID: String?, muted: Bool, video: Bool) {
        configurations.append((callID, muted, video))
    }
    func setQuality(_ quality: IrisCallQuality, customKilobits: Int) {}
    func stop() { stopCount += 1 }
}

final class CallControllerTests: XCTestCase {
    @MainActor
    func testDeclineImmediatelyDismissesAndDoesNotReappearFromQueuedState() {
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0) },
            showError: { _ in }, mediaForTesting: CallMediaProbe())
        var call = connectedCall(id: "declined")
        call.phase = "incoming"
        controller.update(call)
        XCTAssertEqual(controller.presentedCall?.phase, "incoming")
        // The CallKit end delegate and the in-app Decline button both use end().
        controller.end()
        XCTAssertNil(controller.presentedCall)
        controller.update(call)
        XCTAssertNil(controller.presentedCall)
        call.phase = "ended"
        call.endReason = "Call declined"
        controller.update(call)
        XCTAssertNil(controller.presentedCall)
        XCTAssertEqual(actions.filter { if case .endCall(callId: "declined") = $0 { return true }; return false }.count, 2)
        controller.update(connectedCall(id: "next"))
        XCTAssertEqual(controller.presentedCall?.callId, "next")
    }

    @MainActor
    func testRemoteEndDismissesAutomaticallyWithoutDismissingANewCall() async throws {
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0) },
            showError: { _ in }, mediaForTesting: CallMediaProbe())
        var ended = connectedCall(id: "remote")
        ended.phase = "ended"
        ended.endReason = "Call ended"
        controller.update(ended)
        XCTAssertEqual(controller.presentedCall?.endReason, "Call ended")
        try await Task.sleep(nanoseconds: 1_800_000_000)
        XCTAssertNil(controller.presentedCall)
        XCTAssertTrue(actions.contains { if case .endCall(callId: "remote") = $0 { return true }; return false })

        ended.callId = "old"
        controller.update(ended)
        controller.update(connectedCall(id: "new"))
        try await Task.sleep(nanoseconds: 1_800_000_000)
        XCTAssertEqual(controller.presentedCall?.callId, "new")
        XCTAssertFalse(actions.contains { if case .endCall(callId: "old") = $0 { return true }; return false })
    }

    @MainActor
    func testHangupRejectsQueuedConnectedStateAndMediaUntilANewCall() {
        let media = CallMediaProbe()
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0) },
            showError: { _ in }, mediaForTesting: media)
        let first = connectedCall(id: "first")
        controller.update(first)
        XCTAssertEqual(media.started.map(\.callID), ["first"])
        XCTAssertEqual(media.configurations.last?.callID, "first")
        controller.receiveMedia(callID: "first", kind: 1, sequence: 0, timestampUs: 1, keyFrame: false, data: Data([1]))
        XCTAssertEqual(media.received.count, 1)

        let stopped = media.stopCount
        controller.end()
        XCTAssertEqual(media.stopCount, stopped + 1)
        let configurationsAtHangup = media.configurations.count
        // A connected full-state update may already be queued at local hangup.
        controller.update(first)
        XCTAssertEqual(media.started.map(\.callID), ["first"])
        controller.receiveMedia(callID: "first", kind: 1, sequence: 0, timestampUs: 1, keyFrame: false, data: Data([1]))
        XCTAssertEqual(media.received.count, 1)
        XCTAssertTrue(media.configurations[configurationsAtHangup...].allSatisfy { $0.callID == nil && !$0.video })
        XCTAssertTrue(actions.contains { if case .endCall(callId: "first") = $0 { return true }; return false })

        controller.update(connectedCall(id: "next"))
        XCTAssertEqual(media.started.map(\.callID), ["first", "next"])
        XCTAssertEqual(media.configurations.last?.callID, "next")
    }

    @MainActor
    func testOnlyAcceptedStateStartsMediaAndPreservesPreAnswerControls() {
        let media = CallMediaProbe()
        let controller = IrisCallController(dispatch: { _ in },
            showError: { _ in }, mediaForTesting: media)
        var call = connectedCall(id: "consent")
        for phase in ["incoming", "outgoing"] {
            call.phase = phase
            controller.update(call)
            XCTAssertTrue(media.started.isEmpty)
            XCTAssertNil(media.configurations.last?.callID)
            XCTAssertEqual(media.configurations.last?.video, false)
        }
        call.phase = "connected"
        call.muted = true
        call.video = false
        controller.update(call)
        XCTAssertEqual(media.started.count, 1)
        XCTAssertEqual(media.configurations.last?.callID, "consent")
        XCTAssertEqual(media.configurations.last?.muted, true)
        XCTAssertEqual(media.configurations.last?.video, false)

        call.phase = "ended"
        let stopped = media.stopCount
        controller.update(call)
        XCTAssertEqual(media.stopCount, stopped + 1)
        XCTAssertNil(media.configurations.last?.callID)
    }

    @MainActor
    func testMutingAndCameraOffDisableCaptureBeforeCoreUpdate() {
        let media = CallMediaProbe()
        let controller = IrisCallController(dispatch: { _ in },
            showError: { _ in }, mediaForTesting: media)
        controller.update(connectedCall(id: "privacy"))
        controller.toggleMuted()
        XCTAssertEqual(media.configurations.last?.muted, true)
        controller.toggleCamera()
        XCTAssertEqual(media.configurations.last?.video, false)
        XCTAssertEqual(media.configurations.last?.muted, true, "camera toggle must preserve pending mute")
        controller.update(connectedCall(id: "privacy"))
        XCTAssertEqual(media.configurations.last?.video, false, "queued old state must not restart the camera")
        XCTAssertEqual(media.configurations.last?.muted, true, "queued old state must not unmute")
    }

    func testQueuedMediaCannotRegainConsentAfterCaptureRestarts() {
        let gate = IrisCallSendGate()
        gate.update(callID: "call", muted: false, video: true)
        let audio = gate.permission(callID: "call", kind: 1)
        let video = gate.permission(callID: "call", kind: 2)
        XCTAssertTrue(audio()); XCTAssertTrue(video())
        let capturedBefore = UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000)
        gate.update(callID: "call", muted: true, video: false)
        XCTAssertFalse(audio()); XCTAssertFalse(video())
        gate.update(callID: "call", muted: false, video: true)
        XCTAssertFalse(audio()); XCTAssertFalse(video())
        XCTAssertTrue(gate.permission(callID: "call", kind: 1)())
        XCTAssertFalse(gate.permission(callID: "call", kind: 1, capturedAtUs: capturedBefore &- 1)())
        XCTAssertFalse(gate.permission(callID: "call", kind: 2, capturedAtUs: capturedBefore &- 1)())
        gate.update(callID: nil, muted: true, video: false)
        XCTAssertFalse(gate.permission(callID: "call", kind: 1)())
    }

    private func connectedCall(id: String) -> CallSnapshot {
        CallSnapshot(outgoing: false, targetBitrateBps: 2_000_000, keyFrameGeneration: 0,
                     mediaConnected: true, maxBitrateBps: 2_000_000,
                     callId: id, chatId: "peer", peerName: "Alex", phase: "connected", video: true,
                     videoCapable: true, muted: false, remoteVideo: true, remoteMuted: false,
                     startedAtSecs: 1, connectedAtSecs: 2, endReason: nil)
    }
}
