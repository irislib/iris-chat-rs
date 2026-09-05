import SwiftUI

struct IrisDirectChatCapabilityBar: View {
    @Environment(\.irisPalette) private var palette
    let state: DirectChatCapabilityState
    let onRetry: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            if state == .checking {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: state == .unavailable ? "message" : "wifi.exclamationmark")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.muted)
            }
            Text(message)
                .font(.system(.subheadline, design: .rounded, weight: .medium))
                .foregroundStyle(palette.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if state != .checking {
                Button("Check again", action: onRetry)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("directChatCapabilityRetryButton")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity)
        .background(.regularMaterial)
        .accessibilityIdentifier("directChatCapabilityBar")
    }

    private var message: String {
        switch state {
        case .checking:
            return "Checking whether this person can receive messages…"
        case .unavailable:
            return "This person can’t receive Iris messages yet."
        case .checkFailed:
            return "Couldn’t check messaging availability."
        case .available:
            return ""
        }
    }
}
