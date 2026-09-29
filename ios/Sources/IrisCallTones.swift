import AVFoundation

@MainActor
protocol IrisCallTonePlaying: AnyObject {
    var isPlaying: Bool { get }
    var numberOfLoops: Int { get set }
    func play() -> Bool
    func stop()
}

extension AVAudioPlayer: IrisCallTonePlaying {}

/// Output only: microphone and camera capture still wait for an answered call.
@MainActor
final class IrisCallTones {
    private var player: IrisCallTonePlaying?
    private var key: String?
    private let makePlayer: (URL) throws -> IrisCallTonePlaying

    init(makePlayer: @escaping (URL) throws -> IrisCallTonePlaying = { try AVAudioPlayer(contentsOf: $0) }) {
        self.makePlayer = makePlayer
    }

    func update(_ call: CallSnapshot?) {
        let tone = Self.tone(for: call)
        let next = tone.flatMap { name in call.map { "\($0.callId):\(name)" } }
        guard next != key || (next != nil && player?.isPlaying != true) else { return }
        player?.stop()
        player = nil
        key = nil
        guard let tone, let url = Bundle.main.url(forResource: "call-\(tone)", withExtension: "wav") else { return }
        do {
            let player = try makePlayer(url)
            player.numberOfLoops = -1
            guard player.play() else { player.stop(); return }
            self.player = player
            key = next
        } catch { /* Retry on the next session or call update. */ }
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
