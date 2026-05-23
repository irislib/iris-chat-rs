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

struct SupportBundleShareItem: Identifiable {
    let id = UUID()
    let url: URL
}

struct SupportBundleShareSheet: View {
    @Environment(\.dismiss) private var dismiss
    let item: SupportBundleShareItem

    var body: some View {
        IrisScrollScreen {
            HStack {
                Spacer()
                IrisModalCloseButton(action: { dismiss() })
                    .accessibilityIdentifier("supportBundleCloseButton")
            }

            IrisSectionCard {
                CardHeader(title: "Debug dump")

                ShareLink(item: item.url) {
                    Label("Share debug dump", systemImage: "square.and.arrow.up")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
            }
        }
        .irisModalSurface()
        .presentationDetents([.medium])
    }
}

enum ProfileQrTab: String, CaseIterable, Identifiable {
    case code
    case scan

    var id: String { rawValue }

    var title: String {
        switch self {
        case .code: return "Code"
        case .scan: return "Scan"
        }
    }
}

struct QrModalCodeContent {
    let value: String
    let label: String
    let helperText: String
    let codeAccessibilityIdentifier: String

    static func profile(account: AccountSnapshot) -> QrModalCodeContent {
        QrModalCodeContent(
            value: irisChatProfileURL(npub: account.npub).absoluteString,
            label: "",
            helperText: "",
            codeAccessibilityIdentifier: "myProfileQrCode"
        )
    }
}

struct ProfileQrModal: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    let codeContent: QrModalCodeContent
    let closeSettings: (() -> Void)?
    @State private var selectedTab: ProfileQrTab

    init(
        manager: AppManager,
        account: AccountSnapshot,
        initialTab: ProfileQrTab = .code,
        codeContent: QrModalCodeContent? = nil,
        closeSettings: (() -> Void)? = nil
    ) {
        self.manager = manager
        self.account = account
        self.codeContent = codeContent ?? .profile(account: account)
        self.closeSettings = closeSettings
        _selectedTab = State(initialValue: initialTab)
    }

    var body: some View {
        ZStack {
            BackgroundFill()

            VStack(spacing: 0) {
                header

                if selectedTab == .code {
                    ProfileQrCodePane(manager: manager, account: account, codeContent: codeContent)
                } else {
                    ProfileQrScanPane { code in
                        handleScannedCode(code)
                    }
                }
            }
        }
        .accessibilityIdentifier("profileQrModal")
        .irisModalSurface()
    }

    private var header: some View {
        HStack(spacing: 0) {
            Color.clear
                .frame(width: 72, height: 44)

            Spacer(minLength: 8)

            Picker("", selection: $selectedTab) {
                ForEach(ProfileQrTab.allCases) { tab in
                    Text(tab.title)
                        .tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .frame(width: 218)
            .accessibilityIdentifier("profileQrTabs")

            Spacer(minLength: 8)

            IrisModalCloseButton(action: { dismiss() })
                .frame(width: 72, height: 44, alignment: .trailing)
                .accessibilityIdentifier("profileQrDoneButton")
        }
        .padding(.horizontal, 10)
        .padding(.top, 6)
        .padding(.bottom, 4)
    }

    private func handleScannedCode(_ raw: String) {
        guard let action = actionForScannedCode(raw) else {
            return
        }
        manager.dispatch(action)
        dismiss()
        DispatchQueue.main.async {
            closeSettings?()
        }
    }

    private func actionForScannedCode(_ raw: String) -> AppAction? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        if let shortcut = classifyChatInput(input: trimmed) {
            switch shortcut {
            case let .directPeer(peerInput, _, _, _):
                return .createChat(peerInput: peerInput)
            case let .invite(inviteInput, _):
                return .acceptInvite(inviteInput: inviteInput)
            }
        }

        let normalized = normalizePeerInput(input: trimmed)
        guard !normalized.isEmpty, isValidPeerInput(input: normalized) else {
            return nil
        }
        return .createChat(peerInput: normalized)
    }
}

struct ProfileQrCodePane: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    let codeContent: QrModalCodeContent
    @State private var copiedUserID = false
    @State private var copyResetTask: Task<Void, Never>?

    private var username: String {
        account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                qrCard
                    .frame(maxWidth: 420)

                HStack(spacing: 26) {
                    ProfileQrActionButton(
                        systemImage: copiedUserID ? "checkmark" : "doc.on.doc.fill",
                        title: copiedUserID ? "Copied" : "Copy"
                    ) {
                        copyUserID()
                    }
                    .accessibilityIdentifier("profileQrCopyButton")

                    ShareLink(item: codeContent.value) {
                        ProfileQrActionLabel(systemImage: "square.and.arrow.up", title: "Share")
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("profileQrShareButton")
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 24)
            .padding(.top, 28)
            .padding(.bottom, 34)
        }
        .scrollIndicators(.hidden)
        .accessibilityIdentifier("profileQrCodeTab")
        .onDisappear {
            copyResetTask?.cancel()
            copyResetTask = nil
        }
    }

    private var qrCard: some View {
        VStack(spacing: 18) {
            VStack(spacing: 8) {
                IrisAvatar(
                    label: username,
                    size: 72,
                    emphasize: true,
                    pictureUrl: account.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager,
                    loadedImageIdentifier: "profileQrAvatarImage"
                )
                Text(username)
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .foregroundStyle(Color(red: 0.04, green: 0.11, blue: 0.22))
                    .lineLimit(1)
                    .multilineTextAlignment(.center)
            }

            GeometryReader { proxy in
                let side = max(proxy.size.width, 0)
                QrCodeImage(text: codeContent.value, size: side)
                    .accessibilityIdentifier(codeContent.codeAccessibilityIdentifier)
            }
            .aspectRatio(1, contentMode: .fit)
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(Color.white)
            )

            if !codeContent.label.isEmpty {
                Text(codeContent.label)
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .foregroundStyle(Color(red: 0.04, green: 0.11, blue: 0.22))
                    .lineLimit(1)
                    .multilineTextAlignment(.center)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 22)
        .frame(maxWidth: .infinity)
        .background(
            RoundedRectangle(cornerRadius: 26, style: .continuous)
                .fill(Color(red: 0.83, green: 0.91, blue: 1.0))
        )
    }

    private func copyUserID() {
        manager.copyToClipboard(codeContent.value)
        copyResetTask?.cancel()
        withAnimation(.spring(response: 0.24, dampingFraction: 0.78)) {
            copiedUserID = true
        }
        copyResetTask = Task {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                withAnimation(.easeInOut(duration: 0.18)) {
                    copiedUserID = false
                }
            }
        }
    }

}

struct ProfileQrScanPane: View {
    @Environment(\.irisPalette) private var palette
    let onCode: (String) -> Void

    var body: some View {
        VStack(spacing: 14) {
            QrScannerSheet(onCode: onCode)
                .frame(maxWidth: .infinity, minHeight: 420)
                .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
                .accessibilityIdentifier("profileQrScanner")

            Text("Scan a chat QR code.")
                .font(.system(.footnote, design: .rounded))
                .foregroundStyle(palette.muted)
        }
        .padding(.horizontal, 18)
        .padding(.top, 22)
        .padding(.bottom, 28)
        .accessibilityIdentifier("profileQrScanTab")
    }
}

struct ProfileQrActionButton: View {
    let systemImage: String
    let title: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ProfileQrActionLabel(systemImage: systemImage, title: title)
        }
        .buttonStyle(.irisPlain)
    }
}

struct ProfileQrActionLabel: View {
    @Environment(\.irisPalette) private var palette
    let systemImage: String
    let title: String

    var body: some View {
        VStack(spacing: 7) {
            Image(systemName: systemImage)
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .frame(width: 50, height: 50)
                .background(
                    Circle()
                        .fill(palette.panelAlt)
                )

            Text(title)
                .font(.system(.footnote, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
        }
        .contentShape(Rectangle())
    }
}

struct SettingsProfileMenuRow: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.colorScheme) private var colorScheme
    let account: AccountSnapshot
    let preferences: PreferencesSnapshot
    @ObservedObject var manager: AppManager
    let showQr: () -> Void
    let action: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Button(action: action) {
                HStack(spacing: 14) {
                    IrisAvatar(
                        label: account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName,
                        size: 54,
                        emphasize: true,
                        pictureUrl: account.pictureUrl,
                        preferences: preferences,
                        manager: manager,
                        loadedImageIdentifier: "myProfileAvatarImage"
                    )
                    VStack(alignment: .leading, spacing: 3) {
                        Text(account.displayName.isEmpty ? "Profile" : account.displayName)
                            .font(.system(.headline, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)
                        Text("My profile")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 8)
                }
                .contentShape(Rectangle())
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("settingsProfileRow")

            Button(action: showQr) {
                Image(systemName: "qrcode")
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(qrIconColor)
                    .frame(width: 36, height: 36)
                    .background(
                        Circle()
                            .fill(qrButtonBackground)
                    )
                    .contentShape(Circle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("QR code")
            .accessibilityIdentifier("settingsProfileQrButton")
        }
        .padding(18)
        .background(
            RoundedRectangle(cornerRadius: IrisLayout.sectionCornerRadius, style: .continuous)
                .fill(palette.panelAlt)
                .overlay(
                    RoundedRectangle(cornerRadius: IrisLayout.sectionCornerRadius, style: .continuous)
                        .stroke(palette.border, lineWidth: 1)
                )
        )
    }

    private var qrButtonBackground: Color {
        colorScheme == .dark
            ? Color(.sRGB, red: 74.0 / 255.0, green: 74.0 / 255.0, blue: 74.0 / 255.0, opacity: 1)
            : Color(.sRGB, red: 233.0 / 255.0, green: 233.0 / 255.0, blue: 233.0 / 255.0, opacity: 1)
    }

    private var qrIconColor: Color {
        colorScheme == .dark
            ? Color(.sRGB, red: 212.0 / 255.0, green: 212.0 / 255.0, blue: 212.0 / 255.0, opacity: 1)
            : palette.textPrimary
    }
}

struct SettingsMenuSection<Content: View>: View {
    @Environment(\.irisPalette) private var palette
    let title: String?
    let content: () -> Content

    init(title: String? = nil, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title {
                Text(title)
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .padding(.horizontal, 4)
            }
            VStack(spacing: 0, content: content)
                .background(
                    RoundedRectangle(cornerRadius: IrisLayout.sectionCornerRadius, style: .continuous)
                        .fill(palette.panel)
                        .overlay(
                            RoundedRectangle(cornerRadius: IrisLayout.sectionCornerRadius, style: .continuous)
                                .stroke(palette.border, lineWidth: 1)
                        )
                )
        }
    }
}

struct SettingsMenuRow: View {
    @Environment(\.irisPalette) private var palette
    let page: SettingsPage
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: page.systemImage)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 32, height: 32)
                Text(page.title)
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(palette.muted)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 13)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier(page.accessibilityID)
    }
}

struct SettingsExternalLinkRow: View {
    @Environment(\.irisPalette) private var palette
    let title: String
    let subtitle: String
    let systemImage: String
    let destination: URL
    let accessibilityIdentifier: String

    var body: some View {
        Link(destination: destination) {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 24)
                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Text(subtitle)
                        .font(.system(.footnote, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                Image(systemName: "arrow.up.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(palette.muted)
            }
            .padding(.vertical, 7)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier(accessibilityIdentifier)
    }
}
