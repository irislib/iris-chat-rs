import SwiftUI

let chatMuteDurations: [(label: String, seconds: UInt64)] = [
    ("1 hour", 3_600), ("8 hours", 28_800), ("1 day", 86_400), ("1 week", 604_800),
]

func chatMuteDeadline(seconds: UInt64, now: Date = Date()) -> UInt64 {
    UInt64(max(0, now.timeIntervalSince1970)) + seconds
}

struct ChatMuteOptions: View {
    let manager: AppManager
    let chatId: String
    let muted: Bool

    var body: some View {
        if muted {
            Button("Unmute") { manager.dispatch(.setChatMuted(chatId: chatId, muted: false)) }
        }
        ForEach(chatMuteDurations, id: \.seconds) { duration in
            Button(duration.label) {
                manager.dispatch(.setChatMuteUntil(chatId: chatId, untilSecs: chatMuteDeadline(seconds: duration.seconds)))
            }
        }
        Button("Always") { manager.dispatch(.setChatMuted(chatId: chatId, muted: true)) }
    }
}
