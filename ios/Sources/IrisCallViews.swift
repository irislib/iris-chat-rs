import SwiftUI

struct IrisChatCallButtons: View {
    @ObservedObject var manager: AppManager
    let chatID: String

    var body: some View {
        if !chatID.hasPrefix("group:"), !manager.isUserBlocked(chatID) {
            HStack(spacing: 16) {
                if manager.state.preferences.voiceCallsEnabled {
                    Button { manager.calls.start(chatID: chatID, video: false) } label: {
                        Image(systemName: "phone.fill")
                    }
                    .accessibilityLabel("Voice call")
                    .accessibilityIdentifier("startVoiceCallButton")
                }
                if manager.state.preferences.videoCallsEnabled {
                    Button { manager.calls.start(chatID: chatID, video: true) } label: {
                        Image(systemName: "video.fill")
                    }
                    .accessibilityLabel("Video call")
                    .accessibilityIdentifier("startVideoCallButton")
                }
            }
            .font(.system(size: 19, weight: .semibold))
            .buttonStyle(.plain)
            .disabled(manager.state.call != nil && manager.state.call?.phase != "ended")
        }
    }
}

struct IrisCallScreen: View {
    @ObservedObject var controller: IrisCallController
    let call: CallSnapshot
    let voiceEnabled: Bool

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                Color(red: 0.08, green: 0.075, blue: 0.12)
                if let image = controller.remoteImage, call.phase == "connected", call.remoteVideo {
                    Image(decorative: image, scale: 1)
                        .resizable().scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                LinearGradient(colors: [.black.opacity(0.5), .clear, .black.opacity(0.65)], startPoint: .top, endPoint: .bottom)
                VStack(spacing: 12) {
                    Text(call.peerName)
                        .font(.system(size: 30, weight: .semibold, design: .rounded))
                        .lineLimit(2)
                    status
                        .font(.subheadline)
                        .foregroundStyle(.white.opacity(0.75))
                    Spacer()
                    if controller.remoteImage == nil || !call.remoteVideo {
                        Text(String(call.peerName.prefix(1)).uppercased())
                            .font(.system(size: 64, weight: .medium, design: .rounded))
                            .frame(width: 132, height: 132)
                            .background(.white.opacity(0.1), in: Circle())
                    }
                    Spacer()
                    if call.remoteMuted && call.phase == "connected" {
                        Label("Microphone off", systemImage: "mic.slash.fill")
                            .font(.footnote).foregroundStyle(.white.opacity(0.7))
                    }
                    controls
                        .padding(.bottom, 20)
                }
                .padding(.horizontal, 24)
                .padding(.top, max(geometry.safeAreaInsets.top, 24) + 20)
                .padding(.bottom, geometry.safeAreaInsets.bottom)

                if let local = controller.localImage, call.video, call.phase == "connected" {
                    VStack {
                        HStack {
                            Spacer()
                            Image(decorative: local, scale: 1).resizable().scaledToFit()
                                .frame(width: 104, height: 138)
                                .background(.black).clipShape(RoundedRectangle(cornerRadius: 16))
                                .accessibilityLabel("Your camera")
                        }
                        Spacer()
                    }
                    .padding(.trailing, 18).padding(.top, 130 + geometry.safeAreaInsets.top)
                    .allowsHitTesting(false)
                }
            }
            .foregroundStyle(.white)
            .ignoresSafeArea()
            .accessibilityIdentifier("callScreen")
        }
    }

    @ViewBuilder private var status: some View {
        if call.phase == "incoming" { Text(call.videoCapable ? "Incoming video call" : "Incoming voice call") }
        else if call.phase == "outgoing" { Text("Calling…") }
        else if call.phase == "ended" { Text(call.endReason ?? "Call ended") }
        else if let seconds = call.connectedAtSecs {
            Text(Date(timeIntervalSince1970: TimeInterval(seconds)), style: .timer)
        } else { Text("Connected") }
    }

    @ViewBuilder private var controls: some View {
        if call.phase == "ended" {
            Button("Done") { controller.end() }
                .buttonStyle(.borderedProminent).tint(.white.opacity(0.2))
                .accessibilityIdentifier("dismissCallButton")
        } else if call.phase == "incoming" {
            VStack(spacing: 24) {
                HStack(spacing: 64) {
                    callButton("Decline", icon: "phone.down.fill", color: .red, id: "declineCallButton") { controller.end() }
                    callButton("Answer", icon: call.videoCapable ? "video.fill" : "phone.fill", color: .green, id: "answerCallButton") { controller.answer() }
                }
                if call.videoCapable && voiceEnabled {
                    Button("Answer with voice") { controller.answer(voiceOnly: true) }
                        .font(.subheadline.weight(.semibold))
                        .accessibilityIdentifier("answerCallWithVoiceButton")
                }
            }
        } else {
            HStack(alignment: .top, spacing: 20) {
                callButton(call.muted ? "Unmute" : "Mute", icon: call.muted ? "mic.slash.fill" : "mic.fill",
                           color: call.muted ? .white.opacity(0.35) : .white.opacity(0.16), id: "muteCallButton") { controller.toggleMuted() }
                if call.videoCapable {
                    callButton("Camera", icon: call.video ? "video.fill" : "video.slash.fill",
                               color: .white.opacity(0.16), id: "callCameraButton") { controller.toggleCamera() }
                }
#if os(iOS)
                callButton("Speaker", icon: controller.speakerEnabled ? "speaker.wave.3.fill" : "speaker.fill",
                           color: controller.speakerEnabled ? .white.opacity(0.35) : .white.opacity(0.16), id: "callSpeakerButton") { controller.toggleSpeaker() }
#endif
                callButton("End", icon: "phone.down.fill", color: .red, id: "endCallButton") { controller.end() }
            }
        }
    }

    private func callButton(_ label: String, icon: String, color: Color, id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: icon).font(.system(size: 23, weight: .semibold))
                    .frame(width: 58, height: 58).background(color, in: Circle())
                Text(label).font(.caption)
            }
        }
        .buttonStyle(.plain).accessibilityLabel(label).accessibilityIdentifier(id)
    }
}
