import AVFoundation
import SwiftUI
#if os(iOS)
import CallKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
struct IrisCallPermissions {
    var status: (AVMediaType) -> AVAuthorizationStatus = { AVCaptureDevice.authorizationStatus(for: $0) }
    var request: (AVMediaType) async -> Bool = { await AVCaptureDevice.requestAccess(for: $0) }

    func error(video: Bool) async -> String? {
        // Device discovery can stall the main thread. The media engine opens
        // capture hardware on its own queue after the call is answered.
        for type in video ? [AVMediaType.audio, .video] : [.audio] {
            switch status(type) {
            case .authorized:
                continue
            case .notDetermined:
                if await request(type) { continue }
            default:
                break
            }
            return type == .audio
                ? "Allow microphone access in Settings to call."
                : "Allow camera access in Settings for video calls."
        }
        return nil
    }
}

@MainActor
final class IrisCallController: NSObject, ObservableObject {
    nonisolated let localSurface = IrisCallVideoSurface()
    nonisolated let remoteSurface = IrisCallVideoSurface()
    @Published private(set) var quality = IrisCallQuality.automatic
    @Published private(set) var customKilobits = 2_000
    @Published private(set) var speakerEnabled = false
    @Published private(set) var presentedCall: CallSnapshot?
    @Published private(set) var startingVideo: Bool?
    private var dismissalTask: Task<Void, Never>?
    private(set) var call: CallSnapshot?
    private let dispatch: (AppAction) -> Void
    private let sendMedia: ((AppAction, @escaping () -> Bool) -> Void)?
    private nonisolated let sendGate = IrisCallSendGate()
    private nonisolated let outgoingSlots = DispatchSemaphore(value: 4)
    private let showError: (String) -> Void
    private var audioActive = true
    private var mediaCallID: String?
    private var endingCallID: String?
    private var pendingMuted: Bool?
    private var pendingVideo: Bool?
    private var permissionRequestID: UUID?
    private let permissionAccess: IrisCallPermissions
    private let mediaForTesting: IrisCallMediaHandling?
#if os(macOS)
    private let desktopAlerts = IrisDesktopCallAlerts()
#endif
    private lazy var media: IrisCallMediaHandling = mediaForTesting ?? IrisCallMediaEngine(
        send: { [weak self, outgoingSlots, sendGate] id, kind, timestamp, key, data, captureAllowed in
            guard outgoingSlots.wait(timeout: .now()) == .success else { return }
            let ticket = sendGate.permission(callID: id, kind: kind)
            let allowed = { captureAllowed() && ticket() }
            Task { @MainActor in
                defer { outgoingSlots.signal() }
                guard let self, self.call?.callId == id, self.endingCallID != id,
                      self.call?.phase == "connected", self.audioActive,
                      kind == 1 ? !(self.pendingMuted ?? self.call?.muted ?? true) :
                        (self.pendingVideo ?? self.call?.video ?? false) else { return }
                let action = AppAction.sendCallMedia(callId: id, kind: kind, timestampUs: timestamp, keyFrame: key, data: data)
                if let sendMedia = self.sendMedia { sendMedia(action, allowed) }
                else if allowed() { self.dispatch(action) }
            }
        },
        frame: { [localSurface, remoteSurface] id, local, pixel in
            (local ? localSurface : remoteSurface).present(pixel, callID: id)
        },
        connectionChanged: { [weak self] id, connected in
            Task { @MainActor in
                guard let self, self.call?.callId == id, self.endingCallID != id else { return }
                self.dispatch(.setCallMediaConnected(callId: id, connected: connected))
            }
        },
        requestKeyFrame: { [weak self] id in
            Task { @MainActor in
                guard let self, self.call?.callId == id, self.endingCallID != id else { return }
                self.dispatch(.requestCallKeyFrame(callId: id))
            }
        },
        failed: { [weak self] id, message in
            Task { @MainActor in
                guard let self, self.call?.callId == id, self.endingCallID != id else { return }
                self.showError(message)
                self.end()
            }
        }
    )
#if os(iOS)
    private var pendingPushCallID: String?
    private var pushRecoveryTask: Task<Void, Never>?
    private var pushReportCompletion: (() -> Void)?
    private var provider: CXProvider?
    private let systemCalls = CXCallController()
    private var systemCallID: UUID?
    private var systemCallCoreID: String?
    private var systemOutgoing = false
    private var systemConnected = false
    private var answerWithVoice = false
    private var fallbackAudioSessionActive = false
#endif

    init(dispatch: @escaping (AppAction) -> Void,
         showError: @escaping (String) -> Void,
         sendMedia: ((AppAction, @escaping () -> Bool) -> Void)? = nil,
         mediaForTesting: IrisCallMediaHandling? = nil,
         permissionAccess: IrisCallPermissions? = nil) {
        self.dispatch = dispatch
        self.showError = showError
        self.sendMedia = sendMedia
        self.mediaForTesting = mediaForTesting
        self.permissionAccess = permissionAccess ?? IrisCallPermissions()
        super.init()
#if os(iOS) && !targetEnvironment(simulator)
        guard mediaForTesting == nil else { return }
        let configuration = CXProviderConfiguration()
        configuration.supportsVideo = true
        configuration.supportedHandleTypes = [.generic]
        configuration.maximumCallsPerCallGroup = 1
        configuration.includesCallsInRecents = false
        let provider = CXProvider(configuration: configuration)
        provider.setDelegate(self, queue: .main)
        self.provider = provider
        audioActive = false
#endif
    }

    func start(chatID: String, video: Bool) {
        guard permissionRequestID == nil, call == nil || call?.phase == "ended" else { return }
        let requestID = UUID()
        permissionRequestID = requestID
        startingVideo = video
        Task { @MainActor [weak self] in
            guard let self else { return }
            let error = await self.permissionAccess.error(video: video)
            guard self.permissionRequestID == requestID else { return }
            self.permissionRequestID = nil
            self.startingVideo = nil
            if let error { self.showError(error); return }
            self.dispatch(.startCall(chatId: chatID, video: video))
        }
    }

    func answer(voiceOnly: Bool = false) {
        guard let call, call.phase == "incoming", endingCallID != call.callId else { return }
#if os(iOS)
        if let systemCallID {
            answerWithVoice = voiceOnly
            systemCalls.request(CXTransaction(action: CXAnswerCallAction(call: systemCallID))) { [weak self] error in
                if error != nil {
                    Task { @MainActor in self?.answerInApp(callID: call.callId, voiceOnly: voiceOnly) }
                }
            }
            return
        }
#endif
        answerInApp(callID: call.callId, voiceOnly: voiceOnly)
    }

    private func answerInApp(callID: String, voiceOnly: Bool, completion: ((Bool) -> Void)? = nil) {
        Task { @MainActor [weak self] in
            guard let self, let current = self.call, current.callId == callID,
                  current.phase == "incoming", self.endingCallID != callID else { completion?(false); return }
            guard await self.permissions(video: current.video && !voiceOnly),
                  self.call?.callId == callID, self.call?.phase == "incoming", self.endingCallID != callID else {
                completion?(false)
                self.dispatch(.endCall(callId: callID))
                return
            }
            if voiceOnly { self.dispatch(.answerCallWithVoice(callId: callID)) }
            else { self.dispatch(.answerCall(callId: callID)) }
            completion?(true)
        }
    }

    func end() {
#if os(iOS)
        pendingPushCallID = nil
        pushRecoveryTask?.cancel()
        pushRecoveryTask = nil
#endif
        permissionRequestID = nil
        startingVideo = nil
        guard let call else { return }
        endingCallID = call.callId
        presentedCall = nil
        dismissalTask?.cancel()
        dismissalTask = nil
        sendGate.update(callID: nil, muted: true, video: false)
        // Stop capture immediately; don't wait for a network round trip.
        media.stop()
        mediaCallID = nil
        localSurface.setCallID(nil)
        remoteSurface.setCallID(nil)
#if os(iOS)
        updateSystemCall(nil)
#elseif os(macOS)
        desktopAlerts.update(nil)
#endif
        dispatch(.endCall(callId: call.callId))
    }

    func toggleMuted() {
        guard let call, endingCallID != call.callId else { return }
        let muted = !(pendingMuted ?? call.muted)
        pendingMuted = muted
        updateHardware()
        dispatch(.setCallMuted(muted: muted))
#if os(iOS)
        if let systemCallID {
            systemCalls.request(CXTransaction(action: CXSetMutedCallAction(call: systemCallID, muted: muted))) { _ in }
        }
#endif
    }

    func toggleCamera() {
        guard let call, call.videoCapable, endingCallID != call.callId else { return }
        if pendingVideo ?? call.video {
            pendingVideo = false
            updateHardware()
            dispatch(.setCallVideoEnabled(enabled: false))
            return
        }
        Task { @MainActor [weak self] in
            guard let self, await self.permissions(video: true), self.call?.callId == call.callId,
                  self.endingCallID != call.callId else { return }
            self.pendingVideo = true
            self.updateHardware()
            self.dispatch(.setCallVideoEnabled(enabled: true))
        }
    }

    func toggleSpeaker() {
#if os(iOS)
        do {
            try AVAudioSession.sharedInstance().overrideOutputAudioPort(speakerEnabled ? .none : .speaker)
            speakerEnabled.toggle()
        } catch { showError("Couldn’t change the speaker.") }
#endif
    }

    func setQuality(_ quality: IrisCallQuality, customKilobits: Int? = nil) {
        let custom = min(10_000, max(100, customKilobits ?? self.customKilobits))
        dispatch(.setCallQuality(quality: quality.rawValue, maxBitrateBps: UInt32(custom * 1_000)))
    }

    func update(_ snapshot: CallSnapshot?, preferences: PreferencesSnapshot? = nil) {
#if os(iOS)
        if pendingPushCallID != nil {
            // Startup may emit several empty snapshots before the queued push
            // reaches the restored core. Keep the system call visible meanwhile.
            if snapshot == nil { return }
            self.pendingPushCallID = nil
            pushRecoveryTask?.cancel()
            pushRecoveryTask = nil
        }
#endif
        IrisAudioActivity.setCallActive(snapshot != nil && snapshot?.phase != "ended")
        let previousID = call?.callId
        call = snapshot
        if let snapshot, snapshot.phase != "ended" {
            permissionRequestID = nil
            startingVideo = nil
        }
        if previousID != snapshot?.callId {
            pendingMuted = nil
            pendingVideo = nil
        }
        if pendingMuted == snapshot?.muted { pendingMuted = nil }
        if pendingVideo == snapshot?.video { pendingVideo = nil }
        if let preferences {
            quality = IrisCallQuality(rawValue: preferences.callQuality) ?? .automatic
            customKilobits = Int(preferences.callMaxBitrateBps / 1_000)
        }
        if snapshot?.callId != endingCallID { endingCallID = nil }
        if previousID != snapshot?.callId || snapshot?.phase == "ended" || snapshot == nil {
            media.stop()
            localSurface.setCallID(nil)
            remoteSurface.setCallID(nil)
        }
        if let snapshot, snapshot.phase == "connected", endingCallID != snapshot.callId {
            media.setQuality(quality, customKilobits: customKilobits)
            media.adapt(targetBitrate: snapshot.targetBitrateBps, keyFrameGeneration: snapshot.keyFrameGeneration)
            media.start(IrisCallMediaSession(callID: snapshot.callId, videoCapable: snapshot.videoCapable))
            remoteSurface.setCallID(snapshot.remoteVideo ? snapshot.callId : nil)
        }
#if os(iOS)
        updateSystemCall(snapshot?.callId == endingCallID ? nil : snapshot)
#elseif os(macOS)
        if mediaForTesting == nil {
            desktopAlerts.update(snapshot?.callId == endingCallID ? nil : snapshot)
        }
#endif
        updateHardware()
        updatePresentation(snapshot, previousID: previousID)
    }

    private func updatePresentation(_ snapshot: CallSnapshot?, previousID: String?) {
        if snapshot?.callId != previousID {
            dismissalTask?.cancel()
            dismissalTask = nil
        }
        guard let snapshot else { presentedCall = nil; return }
        if snapshot.callId == endingCallID {
            presentedCall = nil
            // Core keeps the terminal snapshot until dismissed. The call record
            // has already been saved; clearing the overlay doesn't remove it.
            if snapshot.phase == "ended" { dispatch(.endCall(callId: snapshot.callId)) }
            return
        }
        presentedCall = snapshot
        guard snapshot.phase == "ended", dismissalTask == nil else { return }
        // Match Signal: local hangup closes immediately, remote end/failure
        // remains briefly so the person can read what happened.
        dismissalTask = Task { @MainActor [weak self] in
            do { try await Task.sleep(nanoseconds: 1_500_000_000) }
            catch { return }
            guard let self, self.call?.callId == snapshot.callId,
                  self.call?.phase == "ended" else { return }
            self.end()
        }
    }

    func receiveMedia(callID: String, kind: UInt8, sequence: UInt32, timestampUs: UInt64, keyFrame: Bool, data: Data) {
        guard call?.callId == callID, call?.phase == "connected", endingCallID != callID,
              kind == 1 || call?.remoteVideo == true else { return }
        media.receive(callID: callID, kind: kind, sequence: sequence, timestampUs: timestampUs, keyFrame: keyFrame, data: data)
    }

    private func updateHardware() {
        let active = call?.phase == "connected" && audioActive && call?.callId != endingCallID
        let id = active ? call?.callId : nil
        mediaCallID = id
        sendGate.update(callID: id, muted: pendingMuted ?? call?.muted ?? true,
                        video: active && (pendingVideo ?? call?.video ?? false))
        localSurface.setCallID(active && (pendingVideo ?? call?.video ?? false) ? id : nil)
        media.configure(callID: id, muted: pendingMuted ?? call?.muted ?? true,
                        video: active && (pendingVideo ?? call?.video ?? false))
    }

    private func permissions(video: Bool) async -> Bool {
        if let error = await permissionAccess.error(video: video) {
            showError(error)
            return false
        }
        return true
    }

#if os(iOS)
    private func configureAudioSession(video: Bool) throws {
        try AVAudioSession.sharedInstance().setCategory(.playAndRecord,
            mode: video ? .videoChat : .voiceChat, options: [.allowBluetooth])
        try AVAudioSession.sharedInstance().setPreferredSampleRate(48_000)
        try AVAudioSession.sharedInstance().setPreferredIOBufferDuration(0.02)
        speakerEnabled = video
    }

    func receivePushInvite(_ invite: CallSnapshot?, completion: @escaping () -> Void) {
        guard let invite else {
            // iOS requires every delivered VoIP push to be reported, even if it
            // expired or the sender was blocked since subscription registration.
            reportUnavailablePush(completion: completion)
            return
        }
        if systemCallCoreID == invite.callId {
            completion()
            return
        }
        if let call, call.phase != "ended", call.callId != invite.callId {
            // Report the push as required without replacing the active call.
            // The core will send the authenticated busy response over FIPS.
            reportUnavailablePush(completion: completion)
            return
        }
        pushReportCompletion = completion
        update(invite)
        pendingPushCallID = invite.callId
        pushRecoveryTask?.cancel()
        pushRecoveryTask = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 5_000_000_000) } catch { return }
            guard let self, self.pendingPushCallID == invite.callId else { return }
            self.pendingPushCallID = nil
            self.end()
        }
    }

    private func reportUnavailablePush(completion: @escaping () -> Void) {
        guard mediaForTesting == nil, let provider else { completion(); return }
        let id = UUID()
        let update = CXCallUpdate()
        update.localizedCallerName = "Iris call"
        provider.reportNewIncomingCall(with: id, update: update) { _ in
            provider.reportCall(with: id, endedAt: Date(), reason: .failed)
            completion()
        }
    }

    private func updateSystemCall(_ snapshot: CallSnapshot?) {
        // Controller tests supply a media backend and avoid all system devices.
        let pushCompletion = pushReportCompletion
        pushReportCompletion = nil
        guard mediaForTesting == nil else { pushCompletion?(); return }
        guard let provider else {
            pushCompletion?()
            if let snapshot, snapshot.phase == "connected", mediaCallID != snapshot.callId {
                do {
                    try configureAudioSession(video: snapshot.videoCapable)
                    try AVAudioSession.sharedInstance().setActive(true)
                    fallbackAudioSessionActive = true
                    if speakerEnabled { try AVAudioSession.sharedInstance().overrideOutputAudioPort(.speaker) }
                } catch { showError("Couldn’t start call audio.") }
            } else if fallbackAudioSessionActive && (snapshot == nil || snapshot?.phase == "ended") {
                fallbackAudioSessionActive = false
                try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            }
            return
        }
        guard let snapshot, snapshot.phase != "ended" else {
            if let systemCallID {
                provider.reportCall(with: systemCallID, endedAt: Date(), reason: .remoteEnded)
            }
            systemCallID = nil
            systemCallCoreID = nil
            systemConnected = false
            audioActive = false
            return
        }
        if systemCallCoreID != snapshot.callId {
            if let systemCallID { provider.reportCall(with: systemCallID, endedAt: Date(), reason: .remoteEnded) }
            let id = UUID()
            systemCallID = id
            systemCallCoreID = snapshot.callId
            systemOutgoing = snapshot.phase == "outgoing"
            systemConnected = false
            answerWithVoice = false
            let update = CXCallUpdate()
            update.remoteHandle = CXHandle(type: .generic, value: snapshot.peerName)
            update.localizedCallerName = snapshot.peerName
            update.hasVideo = snapshot.videoCapable
            update.supportsHolding = false
            update.supportsGrouping = false
            update.supportsUngrouping = false
            update.supportsDTMF = false
            if systemOutgoing {
                let action = CXStartCallAction(call: id, handle: update.remoteHandle!)
                action.isVideo = snapshot.videoCapable
                systemCalls.request(CXTransaction(action: action)) { [weak self] error in
                    guard error != nil else { return }
                    Task { @MainActor in self?.systemCallFailed(id: id) }
                }
                provider.reportCall(with: id, updated: update)
            } else {
                provider.reportNewIncomingCall(with: id, update: update) { [weak self] error in
                    pushCompletion?()
                    Task { @MainActor in
                        // Cancellation can beat CallKit's asynchronous report.
                        guard let self, self.systemCallID == id else {
                            if error == nil { provider.reportCall(with: id, endedAt: Date(), reason: .remoteEnded) }
                            return
                        }
                        if error != nil { self.systemCallFailed(id: id) }
                    }
                }
            }
        }
        if snapshot.phase == "connected", !systemConnected {
            systemConnected = true
            if let systemCallID {
                let update = CXCallUpdate()
                update.hasVideo = snapshot.videoCapable
                provider.reportCall(with: systemCallID, updated: update)
            }
            if systemOutgoing, let systemCallID { provider.reportOutgoingCall(with: systemCallID, connectedAt: Date()) }
        }
    }

    private func systemCallFailed(id: UUID) {
        guard systemCallID == id else { return }
        showError("Couldn’t start the system call.")
        end()
    }
#endif

}

#if os(iOS)
extension IrisCallController: @preconcurrency CXProviderDelegate {
    func providerDidReset(_ provider: CXProvider) { end(); audioActive = false }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        guard action.callUUID == systemCallID, let call else { action.fail(); return }
        do {
            try configureAudioSession(video: call.videoCapable)
            provider.reportOutgoingCall(with: action.callUUID, startedConnectingAt: Date())
            action.fulfill()
        } catch { action.fail(); end() }
    }

    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        guard action.callUUID == systemCallID, let call else { action.fail(); return }
        do { try configureAudioSession(video: call.videoCapable && !answerWithVoice) }
        catch { action.fail(); end(); return }
        answerInApp(callID: call.callId, voiceOnly: answerWithVoice) { success in
            if success { action.fulfill() } else { action.fail() }
        }
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        guard action.callUUID == systemCallID else { action.fulfill(); return }
        end()
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        guard action.callUUID == systemCallID, call?.phase != "ended" else { action.fail(); return }
        pendingMuted = action.isMuted
        updateHardware()
        dispatch(.setCallMuted(muted: action.isMuted))
        action.fulfill()
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        audioActive = true
        if speakerEnabled { try? audioSession.overrideOutputAudioPort(.speaker) }
        updateHardware()
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        audioActive = false
        updateHardware()
    }
}
#endif
