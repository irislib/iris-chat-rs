import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

private final class CallMediaProbe: IrisCallMediaHandling {
    var configurations: [(callID: String?, muted: Bool, video: Bool)] = []
    var audioCallIDs: [String] = []

    func configure(callID: String?, muted: Bool, video: Bool) {
        configurations.append((callID, muted, video))
    }

    func receiveAudio(callID: String, data: Data) {
        audioCallIDs.append(callID)
    }
}

final class CallControllerTests: XCTestCase {
    @MainActor
    func testHangupRejectsQueuedConnectedStateAndMediaUntilANewCall() {
        let media = CallMediaProbe()
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0) },
            send: { _, _, _ in }, showError: { _ in }, mediaForTesting: media)
        let first = connectedCall(id: "first")
        controller.update(first)
        XCTAssertEqual(media.configurations.last?.callID, "first")
        controller.receive(callID: "first", kind: 1, data: Data(repeating: 0, count: 640))
        XCTAssertEqual(media.audioCallIDs, ["first"])

        controller.end()
        let configurationsAtHangup = media.configurations.count
        // The core's already-queued state can reach the UI after local hangup.
        controller.update(first)
        controller.receive(callID: "first", kind: 1, data: Data(repeating: 0, count: 640))
        XCTAssertTrue(media.configurations[configurationsAtHangup...].allSatisfy { $0.callID == nil && !$0.video })
        XCTAssertEqual(media.audioCallIDs, ["first"], "old audio must not resume after hangup")
        XCTAssertTrue(actions.contains { if case .endCall(callId: "first") = $0 { return true }; return false })

        controller.update(connectedCall(id: "next"))
        XCTAssertEqual(media.configurations.last?.callID, "next")
        controller.receive(callID: "first", kind: 1, data: Data(repeating: 0, count: 640))
        controller.receive(callID: "next", kind: 1, data: Data(repeating: 0, count: 640))
        XCTAssertEqual(media.audioCallIDs, ["first", "next"])
    }

    @MainActor
    func testOnlyConnectedStateStartsMediaAndRespectsPreAnswerControls() {
        let media = CallMediaProbe()
        let controller = IrisCallController(dispatch: { _ in }, send: { _, _, _ in },
            showError: { _ in }, mediaForTesting: media)
        var call = connectedCall(id: "consent")
        for phase in ["incoming", "outgoing"] {
            call.phase = phase
            controller.update(call)
            XCTAssertNil(media.configurations.last?.callID)
            XCTAssertEqual(media.configurations.last?.video, false)
        }
        call.phase = "connected"
        call.muted = true
        call.video = false
        controller.update(call)
        XCTAssertEqual(media.configurations.last?.callID, "consent")
        XCTAssertEqual(media.configurations.last?.muted, true)
        XCTAssertEqual(media.configurations.last?.video, false)

        call.phase = "ended"
        controller.update(call)
        XCTAssertNil(media.configurations.last?.callID)
    }

    private func connectedCall(id: String) -> CallSnapshot {
        CallSnapshot(callId: id, chatId: "peer", peerName: "Alex", phase: "connected", video: true,
                     videoCapable: true, muted: false, remoteVideo: true, remoteMuted: false,
                     startedAtSecs: 1, connectedAtSecs: 2, endReason: nil)
    }
}
