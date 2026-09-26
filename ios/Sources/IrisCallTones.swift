import AVFoundation

/// Output only: microphone and camera capture still wait for an answered call.
@MainActor
final class IrisCallTones {
    private var player: AVAudioPlayer?
    private var key: String?

    func update(_ call: CallSnapshot?) {
        let tone = Self.tone(for: call)
        let next = tone.flatMap { name in call.map { "\($0.callId):\(name)" } }
        guard next != key else { return }
        player?.stop()
        player = nil
        key = next
        guard let tone, let url = Bundle.main.url(forResource: "call-\(tone)", withExtension: "wav") else { return }
        do {
            let player = try AVAudioPlayer(contentsOf: url)
            player.numberOfLoops = -1
            self.player = player
            player.play()
        } catch { key = nil }
    }

    static func tone(for call: CallSnapshot?) -> String? {
        guard let call, call.outgoing else { return nil }
        switch call.phase {
        case "outgoing": return "connecting"
        case "ringing": return "ringing"
        default: return nil
        }
    }
}
