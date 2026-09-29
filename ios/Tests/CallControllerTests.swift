import XCTest
import AVFoundation
#if os(iOS)
import CallKit
#endif
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
#if os(macOS)
    func setAudioDevices(_ selection: IrisMacAudioDeviceSelection, completion: @escaping (Bool) -> Void) { completion(true) }
    func setScreenSharing(_ enabled: Bool) {}
    func captureScreenFrame(callID: String, pixel: CVPixelBuffer, timestamp: UInt64) {}
#endif
}

@MainActor
private final class CallToneProbe: IrisCallTonePlaying {
    var isPlaying = false
    var numberOfLoops = 0
    var canPlay = true
    var playCount = 0
    var stopCount = 0
    func play() -> Bool { playCount += 1; isPlaying = canPlay; return canPlay }
    func stop() { stopCount += 1; isPlaying = false }
}

final class CallControllerTests: XCTestCase {
    @MainActor
    func testCanceledToastTimerDoesNotEraseNewCallError() async throws {
        let toasts = ToastCenter()
        let message = "Calling is unavailable. Try again when connected."
        toasts.show(message, duration: 0.2)
        try await Task.sleep(nanoseconds: 20_000_000)
        toasts.show("Another error", duration: 0.2)
        toasts.show(message, duration: 0.2)
        try await Task.sleep(nanoseconds: 20_000_000)
        XCTAssertEqual(toasts.message, message, "The canceled timer must not clear the retry's feedback")
        try await Task.sleep(nanoseconds: 250_000_000)
        XCTAssertNil(toasts.message)
    }

    @MainActor
    func testUnchangedSnapshotsDoNotKeepCallErrorVisibleIndefinitely() async throws {
        let toasts = ToastCenter()
        let message = "Calling is unavailable. Try again when connected."
        toasts.show(message, duration: 0.1)
        toasts.show(message, duration: 10)
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertNil(toasts.message)
        toasts.show(message)
        XCTAssertEqual(toasts.message, message, "A retry after dismissal must show feedback again")
    }

    @MainActor
    func testGrantedCallPermissionsNeverRequestAccessAgain() async {
        let permissions = IrisCallPermissions(status: { _ in .authorized }, request: { _ in
            XCTFail("A returning caller must not wait for another permission request")
            return false
        })
        for video in [false, true] {
            let error = await permissions.error(video: video)
            XCTAssertNil(error)
        }
    }

    @MainActor
    func testOnlyMissingCallPermissionsAreRequestedAndDenialIsReported() async {
        var requested: [AVMediaType] = []
        let permissions = IrisCallPermissions(status: { $0 == .audio ? .authorized : .notDetermined }, request: {
            requested.append($0)
            return false
        })
        let voiceError = await permissions.error(video: false)
        XCTAssertNil(voiceError)
        XCTAssertTrue(requested.isEmpty)
        let videoError = await permissions.error(video: true)
        XCTAssertEqual(videoError, "Allow camera access in Settings for video calls.")
        XCTAssertEqual(requested, [.video])
        let denied = IrisCallPermissions(status: { _ in .denied }, request: { _ in
            XCTFail("Denied access must offer Settings without requesting again")
            return true
        })
        let deniedError = await denied.error(video: false)
        XCTAssertEqual(deniedError, "Allow microphone access in Settings to call.")
    }

    @MainActor
    func testRepeatedTapsKeepFirstCallWhilePermissionIsPending() async {
        let prompted = expectation(description: "permission prompt")
        let dispatched = expectation(description: "call starts")
        var resume: CheckedContinuation<Bool, Never>?
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0); dispatched.fulfill() },
            showError: { XCTFail($0) }, mediaForTesting: CallMediaProbe(),
            permissionAccess: IrisCallPermissions(status: { _ in .notDetermined }, request: { _ in
                await withCheckedContinuation { resume = $0; prompted.fulfill() }
            }))
        controller.start(chatID: "peer", video: false)
        XCTAssertEqual(controller.startingVideo, false, "A tap is acknowledged synchronously")
        controller.start(chatID: "other", video: true)
        await fulfillment(of: [prompted], timeout: 1)
        XCTAssertTrue(actions.isEmpty)
        resume?.resume(returning: true)
        await fulfillment(of: [dispatched], timeout: 1)
        XCTAssertEqual(actions, [.startCall(chatId: "peer", video: false)])
        XCTAssertEqual(controller.startingVideo, false, "Feedback continues after permission approval")
        controller.start(chatID: "other", video: true)
        XCTAssertEqual(actions.count, 1)
        controller.update(connectedCall(id: "outgoing"))
        XCTAssertNil(controller.startingVideo)
    }

    @MainActor
    func testFirstVideoCallDispatchesImmediatelyAfterBothPermissionsAndKeepsFeedback() async {
        var requested: [AVMediaType] = []
        var resumes: [CheckedContinuation<Bool, Never>] = []
        let microphone = expectation(description: "microphone permission")
        let camera = expectation(description: "camera permission")
        let dispatched = expectation(description: "call dispatched after approval")
        let controller = IrisCallController(dispatch: {
            XCTAssertEqual($0, .startCall(chatId: "peer", video: true))
            dispatched.fulfill()
        }, showError: { XCTFail($0) }, mediaForTesting: CallMediaProbe(),
        permissionAccess: IrisCallPermissions(status: { _ in .notDetermined }, request: { type in
            requested.append(type)
            return await withCheckedContinuation {
                resumes.append($0)
                (type == .audio ? microphone : camera).fulfill()
            }
        }))
        controller.start(chatID: "peer", video: true)
        await fulfillment(of: [microphone], timeout: 1)
        resumes.removeFirst().resume(returning: true)
        await fulfillment(of: [camera], timeout: 1)
        controller.update(nil) // Background snapshots must not clear progress.
        XCTAssertEqual(controller.startingVideo, true)
        resumes.removeFirst().resume(returning: true)
        await fulfillment(of: [dispatched], timeout: 1)
        XCTAssertEqual(requested, [.audio, .video])
        XCTAssertEqual(controller.startingVideo, true)
        controller.update(nil)
        XCTAssertEqual(controller.startingVideo, true)
        controller.update(connectedCall(id: "video"))
        XCTAssertNil(controller.startingVideo)
    }

    @MainActor
    func testOutgoingToneFollowsConfirmedRingingAndStopsOnAnswerOrEnd() {
        var call = connectedCall(id: "outgoing")
        call.outgoing = true
        call.phase = "outgoing"
        XCTAssertEqual(IrisCallTones.tone(for: call), "connecting")
        call.phase = "ringing"
        XCTAssertEqual(IrisCallTones.tone(for: call), "ringing")
        for phase in ["connected", "ended", "incoming"] {
            call.phase = phase
            XCTAssertNil(IrisCallTones.tone(for: call))
        }
        call.phase = "ringing"
        call.outgoing = false
        XCTAssertNil(IrisCallTones.tone(for: call))
        XCTAssertNil(IrisCallTones.tone(for: nil))
        for name in ["connecting", "ringing"] {
            XCTAssertNotNil(Bundle.main.url(forResource: "call-\(name)", withExtension: "wav"))
        }
    }

    @MainActor
    func testOutgoingToneRetriesFailedPlaybackWithoutWaitingForAnotherPhase() {
        let player = CallToneProbe()
        player.canPlay = false
        let tones = IrisCallTones(makePlayer: { _ in player })
        var call = connectedCall(id: "dialing")
        call.outgoing = true
        call.phase = "outgoing"
        tones.update(call)
        XCTAssertFalse(player.isPlaying)
        player.canPlay = true
        tones.update(call)
        XCTAssertTrue(player.isPlaying)
        XCTAssertEqual(player.playCount, 2)
        XCTAssertEqual(player.numberOfLoops, -1)
        tones.update(nil)
        XCTAssertFalse(player.isPlaying)
    }

    @MainActor
    func testOutgoingToneRecoversStoppedPlayerWithoutRestartingHealthyPlayback() {
        let player = CallToneProbe()
        let tones = IrisCallTones(makePlayer: { _ in player })
        var call = connectedCall(id: "ringing")
        call.outgoing = true
        call.phase = "ringing"
        tones.update(call)
        tones.update(call)
        XCTAssertEqual(player.playCount, 1, "Unchanged updates must preserve the ringing cadence")
        player.isPlaying = false // A route change or interruption stopped playback.
        tones.update(call)
        XCTAssertTrue(player.isPlaying)
        XCTAssertEqual(player.playCount, 2)
        call.phase = "connected"
        tones.update(call)
        XCTAssertFalse(player.isPlaying, "The answered call must stop ringing immediately")
    }

    @MainActor
    func testCancelOrIncomingCallInvalidatesPendingStart() async {
        for incoming in [false, true] {
            let prompted = expectation(description: "permission prompt")
            let resumed = expectation(description: "permission returned")
            var resume: CheckedContinuation<Bool, Never>?
            var actions: [AppAction] = []
            let controller = IrisCallController(dispatch: { actions.append($0) },
                showError: { XCTFail($0) }, mediaForTesting: CallMediaProbe(),
                permissionAccess: IrisCallPermissions(status: { _ in .notDetermined }, request: { _ in
                    let granted = await withCheckedContinuation { resume = $0; prompted.fulfill() }
                    resumed.fulfill()
                    return granted
                }))
            controller.start(chatID: "peer", video: false)
            await fulfillment(of: [prompted], timeout: 1)
            if incoming {
                var call = connectedCall(id: "incoming")
                call.phase = "incoming"
                controller.update(call)
                controller.update(nil)
            } else {
                controller.end()
            }
            XCTAssertNil(controller.startingVideo)
            resume?.resume(returning: true)
            await fulfillment(of: [resumed], timeout: 1)
            await Task.yield()
            XCTAssertTrue(actions.isEmpty)
        }
    }

    @MainActor
    func testPermissionFailureAllowsRetry() async {
        let denied = expectation(description: "denial shown")
        let dispatched = expectation(description: "retry starts")
        var granted = false
        let controller = IrisCallController(dispatch: { _ in dispatched.fulfill() },
            showError: { _ in denied.fulfill() }, mediaForTesting: CallMediaProbe(),
            permissionAccess: IrisCallPermissions(status: { _ in granted ? .authorized : .denied }))
        controller.start(chatID: "peer", video: false)
        await fulfillment(of: [denied], timeout: 1)
        XCTAssertNil(controller.startingVideo)
        granted = true
        controller.start(chatID: "peer", video: false)
        await fulfillment(of: [dispatched], timeout: 1)
        XCTAssertEqual(controller.startingVideo, false)
        controller.update(nil, error: "Calling is unavailable. Try again when connected.")
        XCTAssertNil(controller.startingVideo)
    }

#if os(iOS)
    @MainActor
    func testOutgoingTonePlaysBeforeCallKitActivationWithoutStartingCapture() {
        let media = CallMediaProbe()
        let player = CallToneProbe()
        let controller = IrisCallController(dispatch: { _ in }, showError: { XCTFail($0) },
            mediaForTesting: media, tonesForTesting: IrisCallTones(makePlayer: { _ in player }))
        let provider = CXProvider(configuration: CXProviderConfiguration())
        let session = AVAudioSession.sharedInstance()
        controller.provider(provider, didDeactivate: session)
        var call = connectedCall(id: "before-activation")
        call.outgoing = true
        call.phase = "outgoing"
        controller.update(call)
        XCTAssertFalse(player.isPlaying, "Wait until the outgoing session is configured")
        controller.outgoingAudioSessionConfigured()
        XCTAssertTrue(player.isPlaying)
        XCTAssertTrue(media.started.isEmpty)
        XCTAssertTrue(media.configurations.allSatisfy { $0.callID == nil })
        call.phase = "ringing"
        controller.update(call)
        XCTAssertTrue(player.isPlaying)
        call.phase = "connected"
        controller.update(call)
        XCTAssertFalse(player.isPlaying)
        XCTAssertNil(media.configurations.last?.callID, "Answer alone cannot start capture before CallKit activation")
        controller.provider(provider, didActivate: session)
        XCTAssertEqual(media.configurations.last?.callID, call.callId)
        controller.end()
    }

    @MainActor
    func testCallKitDeactivationSuppressesTonesUntilReactivationAndNextCallCanRing() {
        let player = CallToneProbe()
        let controller = IrisCallController(dispatch: { _ in }, showError: { XCTFail($0) },
            mediaForTesting: CallMediaProbe(), tonesForTesting: IrisCallTones(makePlayer: { _ in player }))
        let provider = CXProvider(configuration: CXProviderConfiguration())
        let session = AVAudioSession.sharedInstance()
        controller.provider(provider, didDeactivate: session)
        var call = connectedCall(id: "first")
        call.outgoing = true
        call.phase = "ringing"
        controller.update(call)
        controller.outgoingAudioSessionConfigured()
        XCTAssertTrue(player.isPlaying)
        controller.provider(provider, didDeactivate: session)
        XCTAssertFalse(player.isPlaying)
        let attempts = player.playCount
        controller.update(call)
        XCTAssertEqual(player.playCount, attempts, "Snapshot updates cannot restart an interrupted session")
        controller.provider(provider, didActivate: session)
        XCTAssertTrue(player.isPlaying)
        controller.end()
        XCTAssertFalse(player.isPlaying)
        controller.update(call)
        XCTAssertFalse(player.isPlaying, "Stale state after local hangup cannot restart the tone")
        call.callId = "second"
        controller.update(call)
        XCTAssertFalse(player.isPlaying)
        controller.outgoingAudioSessionConfigured()
        XCTAssertTrue(player.isPlaying)
        controller.providerDidReset(provider)
        XCTAssertFalse(player.isPlaying)
    }

    @MainActor
    func testSecondPushCannotReplaceConnectedCallOrStopItsMedia() {
        let media = CallMediaProbe()
        let controller = IrisCallController(dispatch: { _ in }, showError: { _ in }, mediaForTesting: media)
        controller.update(connectedCall(id: "active"))
        let stops = media.stopCount
        var invite = connectedCall(id: "second")
        invite.phase = "incoming"
        var completed = false
        controller.receivePushInvite(invite) { completed = true }
        XCTAssertTrue(completed)
        XCTAssertEqual(controller.call?.callId, "active")
        XCTAssertEqual(media.stopCount, stops)
    }

    @MainActor
    func testPushInviteSurvivesColdLaunchSnapshotsAndDismissesAfterCancellation() {
        let controller = IrisCallController(dispatch: { _ in }, showError: { _ in }, mediaForTesting: CallMediaProbe())
        var invite = connectedCall(id: "push-call")
        invite.phase = "incoming"
        var completed = false
        controller.receivePushInvite(invite) { completed = true }
        XCTAssertTrue(completed, "PushKit completion follows the local call report without waiting for FIPS")
        controller.update(nil)
        XCTAssertEqual(controller.call?.callId, invite.callId)
        controller.update(invite)
        controller.update(nil)
        XCTAssertNil(controller.call)
        XCTAssertNil(controller.presentedCall)
    }

    @MainActor
    func testBlockedPreferenceDismissesPushBeforeCoreRecoveryAndCannotBeAnswered() async {
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0) }, showError: { _ in },
                                            mediaForTesting: CallMediaProbe())
        var invite = connectedCall(id: "blocked-push")
        invite.phase = "incoming"
        controller.receivePushInvite(invite) {}
        var preferences = makeAppState(rev: 1).preferences
        preferences.blockedOwnerPubkeys = [invite.chatId]
        controller.update(nil, preferences: preferences)
        XCTAssertNil(controller.call)
        XCTAssertNil(controller.presentedCall)
        controller.answer()
        await Task.yield()
        XCTAssertEqual(actions, [.endCall(callId: invite.callId)])
        // Even a queued pre-block snapshot must not revive the native call.
        controller.update(invite)
        XCTAssertNil(controller.call)
        XCTAssertNil(controller.presentedCall)
    }

    @MainActor
    func testCallsDisabledDuringPushRecoveryDismissesPendingCall() {
        let controller = IrisCallController(dispatch: { _ in }, showError: { _ in },
                                            mediaForTesting: CallMediaProbe())
        var invite = connectedCall(id: "disabled-push")
        invite.phase = "incoming"
        controller.receivePushInvite(invite) {}
        var preferences = makeAppState(rev: 1).preferences
        preferences.voiceCallsEnabled = false
        preferences.videoCallsEnabled = false
        controller.update(nil, preferences: preferences)
        XCTAssertNil(controller.call)
        XCTAssertNil(controller.presentedCall)
    }

    @MainActor
    func testDecliningPushInviteClearsPendingStartupProtection() {
        let controller = IrisCallController(dispatch: { _ in }, showError: { _ in }, mediaForTesting: CallMediaProbe())
        var invite = connectedCall(id: "push-decline")
        invite.phase = "incoming"
        controller.receivePushInvite(invite) {}
        controller.end()
        controller.update(nil)
        XCTAssertNil(controller.call)
        XCTAssertNil(controller.presentedCall)
    }
#endif

    @MainActor
    func testVideoCallWithCameraOffDoesNotBecomeADisallowedVoiceCall() {
        var actions: [AppAction] = []
        let media = CallMediaProbe()
        let controller = IrisCallController(dispatch: { actions.append($0) }, showError: { _ in },
                                            mediaForTesting: media)
        var preferences = makeAppState(rev: 1).preferences
        preferences.voiceCallsEnabled = false
        preferences.videoCallsEnabled = true
        var call = connectedCall(id: "video-camera-off")
        call.video = false
        controller.update(call, preferences: preferences)
        XCTAssertEqual(controller.call?.callId, call.callId)
        XCTAssertEqual(media.started.map(\.callID), [call.callId])
        XCTAssertTrue(actions.isEmpty)
    }

    @MainActor
    func testBlockingWhileAnswerPermissionIsPendingCannotAnswer() async {
        let prompted = expectation(description: "permission pending")
        var resume: CheckedContinuation<Bool, Never>?
        var actions: [AppAction] = []
        let controller = IrisCallController(dispatch: { actions.append($0) }, showError: { _ in },
            mediaForTesting: CallMediaProbe(), permissionAccess: IrisCallPermissions(
                status: { _ in .notDetermined }, request: { _ in
                    await withCheckedContinuation { resume = $0; prompted.fulfill() }
                }))
        var invite = connectedCall(id: "blocked-during-permission")
        invite.phase = "incoming"
        invite.video = false
        controller.update(invite)
        controller.answer(voiceOnly: true)
        await fulfillment(of: [prompted], timeout: 1)
        var preferences = makeAppState(rev: 1).preferences
        preferences.blockedOwnerPubkeys = [invite.chatId]
        controller.update(nil, preferences: preferences)
        resume?.resume(returning: true)
        await Task.yield()
        XCTAssertNil(controller.call)
        XCTAssertFalse(actions.contains { if case .answerCall = $0 { return true }; return false })
        XCTAssertFalse(actions.contains { if case .answerCallWithVoice = $0 { return true }; return false })
    }

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
