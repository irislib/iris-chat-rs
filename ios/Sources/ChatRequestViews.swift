import SwiftUI

struct IrisBlockedComposerBar: View {
    @Environment(\.irisPalette) private var palette
    let onUnblock: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 10) {
                Image(systemName: "nosign")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.red)
                Text("User blocked")
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                Spacer(minLength: 0)
            }
            HStack(spacing: 8) {
                Button("Delete chat", role: .destructive, action: onDelete)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("blockedDeleteChatButton")
                Button("Unblock", action: onUnblock)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("blockedUnblockButton")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .background(.regularMaterial)
        .accessibilityIdentifier("blockedComposerBar")
    }
}

/// Message-request gate shown in place of the composer when the user
/// hasn't replied to a stranger yet. Mirrors Signal's pattern — the
/// recipient can read the message and decide whether to engage. While
/// this bar is up, the Rust core suppresses outgoing delivered / read
/// receipts so the sender gets no signal about whether the message
/// was seen.
struct IrisMessageRequestBar: View {
    @Environment(\.irisPalette) private var palette
    let displayName: String
    let onAccept: () -> Void
    let onBlock: () -> Void
    let onBlockAndReport: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text("Message request from \(displayName)")
                .font(.system(.footnote, design: .rounded, weight: .medium))
                .foregroundStyle(palette.muted)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 14)

            HStack(spacing: 8) {
                requestButton(
                    "Block",
                    accessibilityId: "messageRequestBlockButton",
                    role: .destructive,
                    destructive: true,
                    action: onBlock
                )
                requestButton(
                    "Block and report",
                    accessibilityId: "messageRequestBlockAndReportButton",
                    role: nil,
                    destructive: true,
                    action: onBlockAndReport
                )
                requestButton(
                    "Accept",
                    accessibilityId: "messageRequestAcceptButton",
                    role: nil,
                    emphasized: true,
                    action: onAccept
                )
            }
            .padding(.horizontal, 14)
        }
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity)
        .background(.regularMaterial)
        .accessibilityIdentifier("messageRequestBar")
    }

    @ViewBuilder
    private func requestButton(
        _ label: String,
        accessibilityId: String,
        role: ButtonRole?,
        emphasized: Bool = false,
        destructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            Text(label)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .frame(maxWidth: .infinity, minHeight: 36)
                .foregroundStyle(emphasized ? Color.white : (destructive ? Color.red : palette.textPrimary))
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(emphasized ? palette.accent : palette.panelAlt)
                )
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(accessibilityId)
    }
}
