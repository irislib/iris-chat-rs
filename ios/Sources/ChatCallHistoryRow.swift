import SwiftUI

struct CallHistoryPresentation {
    let call: CallHistorySnapshot

    var title: String {
        let medium = call.video ? "video call" : "voice call"
        switch call.outcome {
        case "answered_elsewhere": return "Answered on another device"
        case "missed": return "Missed \(medium)"
        case "declined": return "Declined \(medium)"
        case "canceled": return "Canceled \(medium)"
        default: return "\(call.direction == "outgoing" ? "Outgoing" : "Incoming") \(medium)"
        }
    }

    var isMissed: Bool { call.outcome == "missed" }
    var icon: String { call.video ? "video.fill" : "phone.fill" }
    var directionIcon: String { call.direction == "outgoing" ? "arrow.up.right" : "arrow.down.left" }

    var duration: String? {
        guard call.outcome == "answered" else { return nil }
        let seconds = call.durationSecs
        let remainder = String(format: "%02llu", seconds % 60)
        if seconds >= 3_600 {
            return "\(seconds / 3_600):\(String(format: "%02llu", (seconds / 60) % 60)):\(remainder)"
        }
        return "\(seconds / 60):\(remainder)"
    }

    var detail: String {
        [irisMessageClock(call.startedAtSecs), duration].compactMap { $0 }.joined(separator: " · ")
    }
}

struct ChatCallHistoryRow: View {
    @Environment(\.irisPalette) private var palette
    let call: CallHistorySnapshot

    var body: some View {
        let presentation = CallHistoryPresentation(call: call)
        HStack(spacing: 12) {
            Image(systemName: presentation.icon)
                .font(.system(size: 19, weight: .medium))
                .frame(width: 28)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(presentation.title)
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 5) {
                    Image(systemName: presentation.directionIcon)
                        .accessibilityHidden(true)
                    Text(presentation.detail)
                }
                .font(.system(.caption, design: .rounded))
                .foregroundStyle(palette.muted)
            }
        }
        .foregroundStyle(presentation.isMissed ? Color.red : palette.muted)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(presentation.title), \(presentation.detail)")
        .accessibilityIdentifier("chatCallHistory-\(call.callId)")
    }
}
