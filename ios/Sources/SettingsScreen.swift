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

struct SettingsScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @Binding var focusedSection: SettingsFocusSection?
    let modalClose: (() -> Void)?
    @State private var pendingSecretExport: SecretExportKind?
    @State private var showingDeleteProfileConfirmation = false
    @State private var showingDeleteLocalDataConfirmation = false
    @State private var showingProfileQr = false
    @State private var profileName = ""
    @State private var profileAbout = ""
    @State private var profilePictureViewerItem: IrisProfilePictureViewerItem?
    @State private var newRelayURL = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""
    @State private var selectedPage: SettingsPage?
    @State private var supportBundleBusy = false
    @State private var supportBundleShareItem: SupportBundleShareItem?
    @State private var deviceRosterInput = ""
    @State private var showingDeviceRosterScanner = false

    init(
        manager: AppManager,
        focusedSection: Binding<SettingsFocusSection?>,
        modalClose: (() -> Void)? = nil
    ) {
        self.manager = manager
        self._focusedSection = focusedSection
        self.modalClose = modalClose
    }

    var body: some View {
        settingsBody
            // Settings contains copyable values like version, user ID,
            // device key, server URLs, and build metadata. Buttons and
            // Links still receive taps; inert Text can be selected.
            .textSelection(.enabled)
            .irisModalSurface()
    }

    @ViewBuilder
    private var settingsBody: some View {
        ZStack {
            BackgroundFill()
            settingsScreenMarker

            if IrisLayout.usesDesktopChrome {
                desktopSettingsLayout
            } else {
                mobileSettingsLayout
            }
        }
        .irisProfilePictureViewer(
            item: $profilePictureViewerItem,
            preferences: manager.state.preferences,
            manager: manager
        )
        .sheet(item: $supportBundleShareItem) { item in
            SupportBundleShareSheet(item: item)
        }
        .sheet(isPresented: $showingDeviceRosterScanner) {
            QrScannerSheet { code in
                let resolved = submitDeviceAuthorizationScan(code, manager: manager)
                deviceRosterInput = resolved.errorMessage == nil ? "" : code
                showingDeviceRosterScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingDeviceRosterScanner = false }
        }
        .sheet(isPresented: $showingProfileQr) {
            if let account = manager.state.account {
                ProfileQrModal(
                    manager: manager,
                    account: account,
                    closeSettings: modalClose
                )
                .irisModalSurface()
#if os(iOS)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
#elseif os(macOS)
                .frame(minWidth: 420, minHeight: 560)
#endif
                .irisDismissOnMacOutsideClick { showingProfileQr = false }
            }
        }
        .onAppear(perform: applyFocusedSection)
        .irisOnChange(of: focusedSection) { _ in
            applyFocusedSection()
        }
        .alert(item: $pendingSecretExport) { _ in
            Alert(
                title: Text("Export Secret Key"),
                message: Text("Your secret key gives full access to your profile. Never share it with anyone. Store it securely."),
                primaryButton: .cancel(Text("Cancel")),
                secondaryButton: .default(Text("Copy")) {
                    guard let secret = manager.exportOwnerNsec(), !secret.isEmpty else {
                        manager.showSecretExportUnavailable()
                        return
                    }
                    manager.copyToClipboard(secret)
                }
            )
        }
        .alert("Delete profile?", isPresented: $showingDeleteProfileConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete profile", role: .destructive) {
                manager.deleteProfileAndLocalData()
            }
            .accessibilityIdentifier("myProfileConfirmDeleteProfileButton")
        } message: {
            Text("This clears your public profile, then removes local data from this device.")
        }
        .alert("Delete all local data?", isPresented: $showingDeleteLocalDataConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.resetAppState()
            }
            .accessibilityIdentifier("myProfileConfirmDeleteLocalDataButton")
        } message: {
            Text("This removes secret keys, messages, and cached files from this device. Your public profile is not changed.")
        }
    }

    private var settingsScreenMarker: some View {
        Text("Settings")
            .frame(width: 1, height: 1)
            .opacity(0.001)
            .allowsHitTesting(false)
            .accessibilityIdentifier("settingsScreen")
            .accessibilityLabel("Settings")
    }

    private var desktopSettingsLayout: some View {
        HStack(spacing: 0) {
            ScrollView {
                settingsMenu
                    .padding(.horizontal, 18)
                    .padding(.vertical, 18)
            }
            .frame(width: 312)

            Rectangle()
                .fill(palette.border)
                .frame(width: 1)

            settingsPageScroll(selectedPage ?? .profile, showsBackButton: false)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder
    private var mobileSettingsLayout: some View {
        if modalClose != nil {
            modalSettingsLayout
        } else if let selectedPage {
            settingsPageScroll(selectedPage, showsBackButton: true)
        } else {
            IrisScrollScreen {
                settingsMenu
            }
        }
    }

    private var modalSettingsLayout: some View {
        VStack(spacing: 0) {
            settingsModalHeader

            if let selectedPage {
                settingsPageScroll(selectedPage, showsBackButton: false)
            } else {
                IrisScrollScreen {
                    settingsMenu
                }
            }
        }
    }

    private var settingsModalHeader: some View {
        HStack(spacing: 0) {
            if selectedPage != nil {
                IrisModalBackButton {
                    selectedPage = nil
                }
                .frame(width: 72, height: 44, alignment: .leading)
                .accessibilityIdentifier("settingsSubpageBackButton")
            } else {
                Color.clear
                    .frame(width: 72, height: 44)
            }

            Spacer(minLength: 8)

            Text(selectedPage?.title ?? "Settings")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .lineLimit(1)
                .frame(maxWidth: .infinity)

            Spacer(minLength: 8)

            if let modalClose {
                IrisModalCloseButton(accessibilityIdentifier: "settingsCloseButton", action: modalClose)
                    .frame(width: 72, height: 44, alignment: .trailing)
            } else {
                Color.clear
                    .frame(width: 72, height: 44)
            }
        }
        .padding(.horizontal, 10)
        .padding(.top, 6)
        .padding(.bottom, 4)
    }

    private var settingsMenu: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let account = manager.state.account {
                SettingsProfileMenuRow(
                    account: account,
                    preferences: manager.state.preferences,
                    manager: manager,
                    showQr: { showingProfileQr = true }
                ) {
                    selectedPage = .profile
                }
            }

            SettingsMenuSection {
                ForEach(SettingsPage.primaryMenuPages) { page in
                    SettingsMenuRow(page: page, selected: selectedPage == page) {
                        selectedPage = page
                    }
                }
            }

            SettingsMenuSection {
                ForEach(SettingsPage.infoMenuPages) { page in
                    SettingsMenuRow(page: page, selected: selectedPage == page) {
                        selectedPage = page
                    }
                }
            }

            SettingsMenuSection(title: "Advanced") {
                ForEach(SettingsPage.advancedMenuPages) { page in
                    SettingsMenuRow(page: page, selected: selectedPage == page) {
                        selectedPage = page
                    }
                }
            }
        }
    }

    private func settingsPageScroll(_ page: SettingsPage, showsBackButton: Bool) -> some View {
        IrisScrollScreen {
            if showsBackButton {
                IrisModalBackButton {
                    selectedPage = nil
                }
                .accessibilityIdentifier("settingsSubpageBackButton")
            }

            settingsPageContent(page)
        }
    }

    @ViewBuilder
    private func settingsPageContent(_ page: SettingsPage) -> some View {
        switch page {
        case .profile:
            if let account = manager.state.account {
                ProfileEditorCard(
                    manager: manager,
                    account: account,
                    profileName: $profileName,
                    profileAbout: $profileAbout,
                    openProfilePicture: { profilePictureViewerItem = $0 },
                    showQrCode: { showingProfileQr = true }
                )
            }

        case .devices:
            DeviceRosterContent(
                manager: manager,
                deviceInput: $deviceRosterInput,
                showingScanner: $showingDeviceRosterScanner
            )

        case .messaging:
            IrisSectionCard {
                CardHeader(title: "Messaging")

                Toggle(
                    "Typing indicators",
                    isOn: Binding(
                        get: { manager.state.preferences.sendTypingIndicators },
                        set: { enabled in
                            manager.dispatch(.setTypingIndicatorsEnabled(enabled: enabled))
                        }
                    )
                )
                .irisControlTint()
                .accessibilityIdentifier("myProfileTypingIndicatorsToggle")

                Toggle(
                    "Received / seen",
                    isOn: Binding(
                        get: { manager.state.preferences.sendReadReceipts },
                        set: { enabled in
                            manager.dispatch(.setReadReceiptsEnabled(enabled: enabled))
                        }
                    )
                )
                .irisControlTint()
                .accessibilityIdentifier("myProfileReadReceiptsToggle")

                Toggle(
                    "Accept message requests from unknowns",
                    isOn: Binding(
                        get: { manager.state.preferences.acceptUnknownDirectMessages },
                        set: { enabled in
                            manager.dispatch(.setAcceptUnknownDirectMessages(enabled: enabled))
                        }
                    )
                )
                .irisControlTint()
                .accessibilityIdentifier("myProfileAcceptUnknownMessagesToggle")

                if PlatformStartupAtLogin.isSupported {
                    Toggle(
                        "Open at login",
                        isOn: Binding(
                            get: { manager.state.preferences.startupAtLoginEnabled },
                            set: { enabled in
                                manager.setStartupAtLoginEnabled(enabled)
                            }
                        )
                    )
                    .irisControlTint()
                    .accessibilityIdentifier("myProfileStartupAtLoginToggle")
                }
            }

        case .notifications:
            IrisSectionCard {
                CardHeader(title: "Notifications")
                NotificationsSettingsSection(manager: manager)
            }

        case .media:
            IrisSectionCard {
                CardHeader(title: "Media")
                ImageProxySettingsSection(manager: manager)
            }

        case .nearby:
            #if os(iOS) || os(macOS)
            IrisSectionCard {
                CardHeader(title: "Nearby")
                NearbySettingsRows(manager: manager, service: manager.nearbyIris)
            }
            MailbagSettingsCard(manager: manager, service: manager.nearbyIris)
            #endif

        case .messageServers:
            IrisSectionCard {
                NostrRelaySettingsSection(
                    manager: manager,
                    newRelayURL: $newRelayURL,
                    editingRelayURL: $editingRelayURL,
                    editingRelayDraft: $editingRelayDraft
                )
            }

        case .security:
            IrisSectionCard {
                CardHeader(title: "Keys")

                if manager.state.account?.hasOwnerSigningAuthority == true {
                    Button {
                        pendingSecretExport = .owner
                    } label: {
                        Label("Export secret key", systemImage: "key.fill")
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("myProfileExportOwnerKeyButton")
                }
            }

        case .updates:
            #if os(macOS)
            IrisSectionCard {
                DesktopUpdateSettingsSection(buildSummary: manager.buildSummaryText(), updates: manager.updates)
            }
            #else
            EmptyView()
            #endif

        case .about:
            IrisSectionCard {
                CardHeader(title: "About")

                if manager.trustedTestBuildEnabled() {
                    IrisInfoPill("Test build", tint: .orange)
                }

                HStack(spacing: 10) {
                    Image(systemName: "info.circle.fill")
                        .foregroundStyle(palette.textPrimary)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Version")
                            .font(.system(.headline, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                        Text(manager.buildSummaryText())
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                            .accessibilityIdentifier("myProfileVersionValue")
                    }
                    Spacer()
                }

                Link(destination: irisSourceURL) {
                    HStack(spacing: 10) {
                        Image(systemName: "chevron.left.forwardslash.chevron.right")
                            .foregroundStyle(palette.textPrimary)
                        VStack(alignment: .leading, spacing: 3) {
                            Text("Source code")
                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                .foregroundStyle(palette.textPrimary)
                            Text(irisSourceLabel)
                                .font(.system(.body, design: .rounded))
                                .foregroundStyle(palette.muted)
                                .accessibilityIdentifier("myProfileSourceCodeValue")
                        }
                        Spacer()
                    }
                }
                .accessibilityIdentifier("myProfileSourceCodeButton")
            }

        case .legal:
            IrisSectionCard {
                CardHeader(title: "Legal")

                SettingsExternalLinkRow(
                    title: "Privacy Policy",
                    subtitle: "chat.iris.to/privacy",
                    systemImage: "hand.raised.fill",
                    destination: irisPrivacyURL,
                    accessibilityIdentifier: "myProfilePrivacyPolicyButton"
                )

                Divider().overlay(palette.border)

                SettingsExternalLinkRow(
                    title: "Terms of Use",
                    subtitle: "chat.iris.to/terms",
                    systemImage: "doc.text.fill",
                    destination: irisTermsURL,
                    accessibilityIdentifier: "myProfileTermsButton"
                )

                Divider().overlay(palette.border)

                SettingsExternalLinkRow(
                    title: "Child Safety",
                    subtitle: "chat.iris.to/csae",
                    systemImage: "shield.lefthalf.filled",
                    destination: irisChildSafetyURL,
                    accessibilityIdentifier: "myProfileChildSafetyButton"
                )

                Divider().overlay(palette.border)

                if let contactURL = irisMailtoURL(
                    to: irisSupportEmail,
                    subject: "Iris Chat support",
                    body: ""
                ) {
                    SettingsExternalLinkRow(
                        title: "Contact",
                        subtitle: irisSupportEmail,
                        systemImage: "envelope.fill",
                        destination: contactURL,
                        accessibilityIdentifier: "myProfileContactButton"
                    )
                }
            }

        case .support:
            IrisSectionCard {
                CardHeader(title: "Support")
                Toggle(
                    "Debug logging",
                    isOn: Binding(
                        get: { manager.state.preferences.debugLoggingEnabled },
                        set: { enabled in
                            manager.dispatch(.setDebugLoggingEnabled(enabled: enabled))
                        }
                    )
                )
                .irisControlTint()
                .accessibilityIdentifier("myProfileDebugLoggingToggle")

                Text("Build \(manager.buildSummaryText())")
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.textPrimary)
                if let networkStatus = manager.state.networkStatus {
                    Text(
                        "Network \(networkStatus.syncing ? "syncing" : "idle") · " +
                            "\(networkStatus.connectedRelayCount)/\(networkStatus.relayUrls.count) connected · " +
                            "\(networkStatus.recentEventCount) updates"
                    )
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .accessibilityIdentifier("myProfileNetworkStatusValue")

                    if let category = networkStatus.lastDebugCategory {
                        Text("Last debug \(category)")
                            .font(.system(.caption, design: .rounded))
                            .foregroundStyle(palette.muted)
                    }
                }

                Button {
                    shareSupportBundle()
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "square.and.arrow.up")
                        Text(supportBundleBusy ? "Preparing…" : "Share debug dump")
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(supportBundleBusy)
                .accessibilityIdentifier("myProfileShareSupportBundleButton")

                Button("Copy debug dump") {
                    copySupportBundle()
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .disabled(supportBundleBusy)
                .accessibilityIdentifier("myProfileCopySupportBundleButton")
            }

        case .accountData:
            IrisSectionCard {
                CardHeader(
                    title: "Account data",
                    subtitle: "Manage your profile and data on this device."
                )

                Button("Delete profile", role: .destructive) {
                    showingDeleteProfileConfirmation = true
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .disabled(manager.state.account?.hasOwnerSigningAuthority != true)
                .accessibilityIdentifier("myProfileDeleteProfileButton")

                Button("Delete all local data", role: .destructive) {
                    showingDeleteLocalDataConfirmation = true
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("myProfileDeleteLocalDataButton")
            }
        }
    }

    private func applyFocusedSection() {
        guard let focusedSection else {
            if IrisLayout.usesDesktopChrome, selectedPage == nil {
                selectedPage = .profile
            }
            return
        }
        switch focusedSection {
        case .messageServers:
            selectedPage = .messageServers
        case .messaging:
            selectedPage = .messaging
        }
        self.focusedSection = nil
    }

    private func shareSupportBundle() {
        guard !supportBundleBusy else { return }
        supportBundleBusy = true
        Task {
            let json = await manager.supportBundleJsonAsync()
            supportBundleBusy = false
            guard let url = writeSupportBundleTempFile(json) else {
                manager.copyToClipboard(json)
                return
            }
            supportBundleShareItem = SupportBundleShareItem(url: url)
        }
    }

    private func copySupportBundle() {
        guard !supportBundleBusy else { return }
        supportBundleBusy = true
        Task {
            let json = await manager.supportBundleJsonAsync()
            supportBundleBusy = false
            manager.copyToClipboard(json)
        }
    }

    private func writeSupportBundleTempFile(_ json: String) -> URL? {
        let filename = "iris-chat-debug-dump-\(Int(Date().timeIntervalSince1970)).json"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(filename)
        do {
            try Data(json.utf8).write(to: url, options: .atomic)
            return url
        } catch {
            return nil
        }
    }

}
