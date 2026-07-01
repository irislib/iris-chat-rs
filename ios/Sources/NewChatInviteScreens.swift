import Foundation
import Combine
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

struct NewChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @State private var peerInput = ""
    @State private var submittedInput: String?
    @State private var qrModalTab: ProfileQrTab?

    private var trimmedInput: String {
        peerInput.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedPeerInput: String {
        normalizePeerInput(input: peerInput)
    }

    private var validPeerInput: Bool {
        !normalizedPeerInput.isEmpty && isValidPeerInput(input: normalizedPeerInput)
    }

    private var inputShortcut: ChatInputShortcut? {
        classifyChatInput(input: trimmedInput)
    }

    private var looksLikeInviteLink: Bool {
        if case .invite = inputShortcut { return true }
        return false
    }

    var body: some View {
        IrisScrollScreen {
            VStack(spacing: 18) {
                newChatCard
                joinChatCard
                newGroupRow
            }
        }
        .sheet(item: $qrModalTab) { tab in
            if let account = manager.state.account {
                ProfileQrModal(
                    manager: manager,
                    account: account,
                    initialTab: tab,
                    codeContent: newChatQrContent,
                    closeSettings: nil
                )
                .irisModalSurface()
#if os(iOS)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
#elseif os(macOS)
                .frame(minWidth: 420, minHeight: 560)
#endif
                .irisDismissOnMacOutsideClick { qrModalTab = nil }
            }
        }
        .irisOnChange(of: peerInput) { _ in
            autoProceedIfReady()
        }
        .task {
            if manager.state.publicInvite == nil && !manager.state.busy.creatingInvite {
                manager.dispatch(.createPublicInvite)
            }
        }
    }

    private var newChatCard: some View {
        IrisSectionCard {
            Text("Create chat")
                .font(.system(.title2, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)
                .frame(maxWidth: .infinity, alignment: .center)

            if let invite = manager.state.publicInvite {
                Text("Share an invite to start a chat")
                    .font(.system(.footnote, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity, alignment: .center)

                HStack(spacing: 10) {
                    NewChatInviteCopyButton(value: invite.url, manager: manager)

                    ShareLink(item: invite.url) {
                        NewChatInviteActionLabel(systemImage: "square.and.arrow.up", title: "Share")
                    }
                    .frame(maxWidth: .infinity)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("newChatInviteShareButton")

                    Button(action: { qrModalTab = .code }) {
                        NewChatInviteActionLabel(systemImage: "qrcode", title: "Show")
                    }
                    .frame(maxWidth: .infinity)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("newChatInviteQrButton")
                }
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
            }
        }
    }

    private var joinChatCard: some View {
        IrisSectionCard {
            Text("Join Chat")
                .font(.system(.title2, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)
                .frame(maxWidth: .infinity, alignment: .center)

            TextField("Paste invite or user ID", text: $peerInput)
                .irisIdentifierInputModifiers()
                .textFieldStyle(.plain)
                .irisInputField()
                .accessibilityIdentifier("newChatPeerInput")

            if irisSupportsQrScanning {
                Button(action: { qrModalTab = .scan }) {
                    HStack(spacing: 8) {
                        Image(systemName: "qrcode.viewfinder")
                        Text("Scan code")
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("newChatScanQrButton")
            }
        }
    }

    private var newGroupRow: some View {
        Button(action: { manager.dispatch(.pushScreen(screen: .newGroup)) }) {
            HStack(spacing: 12) {
                Image(systemName: "person.3.fill")
                    .font(.system(.body, weight: .semibold))
                    .frame(width: 22)
                    .foregroundStyle(palette.textPrimary)
                Text("Create group")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.system(.footnote, weight: .semibold))
                    .foregroundStyle(palette.muted)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 13)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(palette.panel)
                    .overlay(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .stroke(palette.border, lineWidth: 1)
                    )
            )
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("newChatNewGroupButton")
    }

    private var newChatQrContent: QrModalCodeContent? {
        guard let invite = manager.state.publicInvite else { return nil }
        return QrModalCodeContent(
            value: invite.url,
            label: "chat with me on iris",
            helperText: "",
            codeAccessibilityIdentifier: "newChatInviteQrCode"
        )
    }

    private func autoProceedIfReady() {
        if validPeerInput, submittedInput != normalizedPeerInput {
            submittedInput = normalizedPeerInput
            manager.dispatch(.createChat(peerInput: normalizedPeerInput))
            return
        }
        if looksLikeInviteLink, submittedInput != trimmedInput {
            submittedInput = trimmedInput
            manager.dispatch(.acceptInvite(inviteInput: trimmedInput))
        }
    }

}

struct NewChatInviteActionLabel: View {
    let systemImage: String
    let title: String

    var body: some View {
        VStack(spacing: 4) {
            Image(systemName: systemImage)
                .font(.system(.body, weight: .semibold))
            Text(title)
                .font(.system(.caption, design: .rounded, weight: .semibold))
                .lineLimit(1)
                .minimumScaleFactor(0.75)
                .allowsTightening(true)
        }
        .frame(maxWidth: .infinity, minHeight: 38)
    }
}

struct NewChatInviteCopyButton: View {
    let value: String
    @ObservedObject var manager: AppManager
    @State private var copied = false
    @State private var resetTask: Task<Void, Never>?

    var body: some View {
        Button(action: copy) {
            ZStack {
                NewChatInviteActionLabel(systemImage: "doc.on.doc", title: "Copy")
                    .opacity(copied ? 0 : 1)
                NewChatInviteActionLabel(systemImage: "checkmark", title: "Copied")
                    .opacity(copied ? 1 : 0)
                    .accessibilityHidden(true)
            }
        }
        .frame(maxWidth: .infinity)
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .accessibilityLabel(copied ? "Copied" : "Copy")
        .accessibilityIdentifier("newChatInviteCopyButton")
        .onDisappear {
            resetTask?.cancel()
            resetTask = nil
        }
    }

    private func copy() {
        manager.copyToClipboard(value)
        resetTask?.cancel()
        withAnimation(.easeInOut(duration: 0.15)) {
            copied = true
        }
        resetTask = Task {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                withAnimation(.easeInOut(duration: 0.15)) {
                    copied = false
                }
            }
        }
    }
}

func shouldAutoSubmitSecret(current: String) -> Bool {
    guard !current.isEmpty else {
        return false
    }
    let lower = current.lowercased()
    if lower.hasPrefix("nsec1") {
        return current.count >= 63
    }
    let hexDigits = CharacterSet(charactersIn: "0123456789abcdefABCDEF")
    if current.count == 64, current.unicodeScalars.allSatisfy({ hexDigits.contains($0) }) {
        return true
    }
    return false
}

struct CreateInviteScreen: View {
    @ObservedObject var manager: AppManager

    var body: some View {
        IrisScrollScreen {
            VStack(spacing: 14) {
                if manager.state.busy.creatingInvite && manager.state.publicInvite == nil {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 24)
                } else if let invite = manager.state.publicInvite {
                    QrCodeImage(text: invite.url)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .accessibilityIdentifier("createInviteQrCode")

                    HStack(spacing: 10) {
                        Button("Copy") {
                            manager.copyToClipboard(invite.url)
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("createInviteCopyButton")

                        ShareLink(item: invite.url) {
                            HStack(spacing: 8) {
                                Image(systemName: "square.and.arrow.up")
                                Text("Share")
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .accessibilityIdentifier("createInviteShareButton")
                    }
                }

                Button(manager.state.busy.creatingInvite ? "Creating…" : "New invite") {
                    manager.dispatch(.createPublicInvite)
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .disabled(manager.state.busy.creatingInvite)
                .accessibilityIdentifier("createInviteRefreshButton")
            }
        }
        .task {
            if manager.state.publicInvite == nil {
                manager.dispatch(.createPublicInvite)
            }
        }
    }
}

struct JoinInviteScreen: View {
    @ObservedObject var manager: AppManager
    @State private var inviteInput = ""
    @State private var showingScanner = false

    private var normalizedInviteInput: String {
        inviteInput.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard(accent: true) {
                CardHeader(title: "Join chat")

                TextField("Invite", text: $inviteInput)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("joinInviteInput")

                HStack(spacing: 10) {
                    Button("Paste") {
                        submitInviteInput(PlatformClipboard.string() ?? "")
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("joinInvitePasteButton")

                    if irisSupportsQrScanning {
                        Button("Scan code") { showingScanner = true }
                            .buttonStyle(IrisSecondaryButtonStyle())
                            .accessibilityIdentifier("joinInviteScanQrButton")
                    }
                }

                Button(manager.state.busy.acceptingInvite ? "Joining…" : "Join chat") {
                    submitInviteInput(inviteInput)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(normalizedInviteInput.isEmpty || manager.state.busy.acceptingInvite)
                .accessibilityIdentifier("joinInviteAcceptButton")
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                submitInviteInput(code)
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
    }

    private func submitInviteInput(_ raw: String) {
        let normalized = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        inviteInput = normalized
        showingScanner = false
        guard !normalized.isEmpty, !manager.state.busy.acceptingInvite else {
            return
        }
        manager.dispatch(.acceptInvite(inviteInput: normalized))
    }
}
