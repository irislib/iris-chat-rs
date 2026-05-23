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

#if os(iOS) || os(macOS)
struct NearbySettingsRows: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            settingsToggle(
                title: "Nearby",
                isOn: Binding(
                    get: { manager.state.preferences.nearbyEnabled },
                    set: { manager.setNearbyEnabled($0) }
                ),
                accessibilityID: "myProfileNearbyEnabledSwitch"
            )

            if manager.state.preferences.nearbyEnabled {
                settingsToggle(
                    title: "Bluetooth",
                    isOn: Binding(
                        get: { manager.state.preferences.nearbyBluetoothEnabled },
                        set: { manager.setNearbyBluetoothEnabled($0) }
                    ),
                    accessibilityID: "myProfileNearbyBluetoothSwitch"
                )

                settingsToggle(
                    title: "Wi-Fi",
                    isOn: Binding(
                        get: { manager.state.preferences.nearbyLanEnabled },
                        set: { manager.setNearbyLanEnabled($0) }
                    ),
                    accessibilityID: "myProfileNearbyLanSwitch"
                )
            }

            settingsToggle(
                title: "Show in chat list",
                isOn: Binding(
                    get: { manager.state.preferences.nearbyShowInChatList },
                    set: { enabled in
                        manager.dispatch(.setNearbyShowInChatList(enabled: enabled))
                    }
                ),
                accessibilityID: "myProfileNearbyShowInChatListSwitch"
            )
        }
        .onAppear {
            service.startBluetoothStateMonitoring()
        }
    }

    private func settingsToggle(
        title: String,
        isOn: Binding<Bool>,
        accessibilityID: String
    ) -> some View {
        HStack(spacing: 12) {
            Text(title)
                .font(.system(.body, design: .rounded))
                .foregroundStyle(palette.textPrimary)
            Spacer()
            Toggle("", isOn: isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .irisControlTint()
                .accessibilityIdentifier(accessibilityID)
        }
    }
}
#endif

#if os(macOS)
struct DesktopUpdateSettingsSection: View {
    @Environment(\.irisPalette) private var palette
    let buildSummary: String
    @ObservedObject var updates: DesktopUpdateController

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            CardHeader(title: "Updates")

            HStack(spacing: 10) {
                Image(systemName: "info.circle.fill")
                    .foregroundStyle(palette.textPrimary)
                VStack(alignment: .leading, spacing: 3) {
                    Text("Current version")
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Text(buildSummary)
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .accessibilityIdentifier("desktopCurrentVersionValue")
                }
                Spacer()
            }

            Toggle("Check automatically", isOn: $updates.autoCheck)
                .irisControlTint()
                .accessibilityIdentifier("desktopAutoCheckUpdatesToggle")

            Toggle("Install automatically", isOn: $updates.autoInstall)
                .irisControlTint()
                .accessibilityIdentifier("desktopAutoInstallUpdatesToggle")

            HStack(spacing: 10) {
                Button {
                    updates.check()
                } label: {
                    Label(updates.checking ? "Checking" : "Check for updates", systemImage: "arrow.clockwise")
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .disabled(updates.checking || updates.installing)
                .accessibilityIdentifier("desktopCheckForUpdatesButton")

                if updates.available {
                    Button {
                        updates.install()
                    } label: {
                        Label(updates.installing ? "Installing" : "Install update", systemImage: "square.and.arrow.down.fill")
                    }
                    .buttonStyle(IrisPrimaryButtonStyle())
                    .disabled(!updates.canInstall)
                    .accessibilityIdentifier("desktopInstallUpdateSettingsButton")
                }
            }

            if !updates.status.isEmpty {
                Text(updates.status)
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
                    .accessibilityIdentifier("desktopUpdateStatusText")
            }
        }
    }
}
#endif

#if os(iOS) || os(macOS)
struct MailbagSettingsCard: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService

    var body: some View {
        IrisSectionCard {
            CardHeader(title: "Mailbag")

            Toggle(isOn: Binding(
                get: { manager.state.preferences.nearbyMailbagEnabled },
                set: { enabled in
                    manager.dispatch(.setNearbyMailbagEnabled(enabled: enabled))
                }
            )) {
                Text("Sync mailbag")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
            }
            .irisControlTint()
            .accessibilityIdentifier("mailbagSettingsToggle")

            Text("Anonymously carries messages by you and others over Bluetooth or Wi-Fi, so they keep moving where there's no internet. Turn off to stop reading and writing without losing what's already in the bag.")
                .font(.system(.caption, design: .rounded))
                .foregroundStyle(palette.muted)
                .fixedSize(horizontal: false, vertical: true)

            // The bag's contents survive the toggle so the user can
            // flip it off without losing work; clearing is a separate
            // action, only surfaced when there's actually something to
            // clear.
            if service.mailbagEventCount > 0 {
                Divider().overlay(palette.border)
                Button(role: .destructive) {
                    service.emptyMailbag()
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "trash")
                            .frame(width: 24)
                        Text("Empty mailbag (\(service.mailbagEventCount))")
                            .font(.system(.body, design: .rounded, weight: .semibold))
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(.red)
                    .padding(.vertical, 2)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)
                .accessibilityIdentifier("mailbagSettingsEmptyButton")
            }
        }
    }
}
#endif

struct NotificationsSettingsSection: View {
    @ObservedObject var manager: AppManager

    private static let defaultServerUrl = "https://notifications.iris.to"
    private static let projectUrl = URL(
        string: "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-notification-server"
    )!
    private static let projectLabel = "Notification server source code"

    var body: some View {
        Toggle("Enabled", isOn: enabled)
            .irisControlTint()
            .accessibilityIdentifier("myProfileDesktopNotificationsToggle")

        Toggle("Invite accepted", isOn: inviteAccepted)
            .irisControlTint()
            .accessibilityIdentifier("myProfileInviteAcceptedNotificationsToggle")

        TextField(Self.defaultServerUrl, text: serverUrl)
            .textFieldStyle(.roundedBorder)
            .autocorrectionDisabled()
            #if os(iOS)
            .keyboardType(.URL)
            .textInputAutocapitalization(.never)
            #endif
            .accessibilityIdentifier("myProfileNotificationsServerUrlInput")

        Link(destination: Self.projectUrl) {
            HStack(spacing: 8) {
                Image(systemName: "chevron.left.forwardslash.chevron.right")
                Text(Self.projectLabel)
                    .font(.system(.body, design: .rounded))
                Spacer()
            }
        }
        .accessibilityIdentifier("myProfileNotificationsServerProjectLink")
    }

    private var enabled: Binding<Bool> {
        Binding(
            get: { manager.state.preferences.desktopNotificationsEnabled },
            set: { enabled in manager.dispatch(.setDesktopNotificationsEnabled(enabled: enabled)) }
        )
    }

    private var inviteAccepted: Binding<Bool> {
        Binding(
            get: { manager.state.preferences.inviteAcceptanceNotificationsEnabled },
            set: { enabled in
                manager.dispatch(.setInviteAcceptanceNotificationsEnabled(enabled: enabled))
            }
        )
    }

    private var serverUrl: Binding<String> {
        Binding(
            get: { manager.state.preferences.mobilePushServerUrl },
            set: { value in manager.dispatch(.setMobilePushServerUrl(url: value)) }
        )
    }
}

struct ImageProxySettingsSection: View {
    @ObservedObject var manager: AppManager

    var body: some View {
        Toggle("Image proxy", isOn: imageProxyEnabled)
            .irisControlTint()
            .accessibilityIdentifier("myProfileImageProxyToggle")

        imageProxyTextField(
            title: "Proxy URL",
            text: imageProxyUrl,
            identifier: "myProfileImageProxyUrlInput"
        )

        imageProxyTextField(
            title: "Proxy key",
            text: imageProxyKeyHex,
            identifier: "myProfileImageProxyKeyInput",
            secure: true
        )

        imageProxyTextField(
            title: "Proxy salt",
            text: imageProxySaltHex,
            identifier: "myProfileImageProxySaltInput",
            secure: true
        )

        Button("Reset image proxy") {
            manager.dispatch(.resetImageProxySettings)
        }
        .buttonStyle(IrisSecondaryButtonStyle())
        .accessibilityIdentifier("myProfileResetImageProxyButton")
    }

    private var imageProxyEnabled: Binding<Bool> {
        Binding(
            get: { manager.state.preferences.imageProxyEnabled },
            set: { enabled in manager.dispatch(.setImageProxyEnabled(enabled: enabled)) }
        )
    }

    private var imageProxyUrl: Binding<String> {
        Binding(
            get: { manager.state.preferences.imageProxyUrl },
            set: { value in manager.dispatch(.setImageProxyUrl(url: value)) }
        )
    }

    private var imageProxyKeyHex: Binding<String> {
        Binding(
            get: { manager.state.preferences.imageProxyKeyHex },
            set: { value in manager.dispatch(.setImageProxyKeyHex(keyHex: value)) }
        )
    }

    private var imageProxySaltHex: Binding<String> {
        Binding(
            get: { manager.state.preferences.imageProxySaltHex },
            set: { value in manager.dispatch(.setImageProxySaltHex(saltHex: value)) }
        )
    }

    private func imageProxyTextField(
        title: String,
        text: Binding<String>,
        identifier: String,
        secure: Bool = false
    ) -> some View {
        Group {
            if secure {
                SecureField(title, text: text)
            } else {
                TextField(title, text: text)
            }
        }
        .textFieldStyle(.roundedBorder)
        .autocorrectionDisabled()
        .accessibilityIdentifier(identifier)
    }
}

struct NostrRelaySettingsSection: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @Binding var newRelayURL: String
    @Binding var editingRelayURL: String?
    @Binding var editingRelayDraft: String

    private var relayURLs: [String] {
        manager.state.networkStatus?.relayUrls ?? manager.state.preferences.nostrRelayUrls
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Message servers")
                .font(.system(.headline, design: .rounded, weight: .semibold))

            ForEach(relayURLs, id: \.self) { relayURL in
                relayRow(relayURL)
            }

            HStack(spacing: 8) {
                TextField("wss://server.example", text: $newRelayURL)
                    .textFieldStyle(.roundedBorder)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("myProfileNewRelayInput")

                Button {
                    manager.dispatch(.addNostrRelay(relayUrl: newRelayURL))
                    newRelayURL = ""
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityLabel("Add server")
                .accessibilityIdentifier("myProfileAddRelayButton")
            }

            Button("Reset servers") {
                manager.dispatch(.resetNostrRelays)
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .accessibilityIdentifier("myProfileResetRelaysButton")
        }
    }

    @ViewBuilder
    private func relayRow(_ relayURL: String) -> some View {
        if editingRelayURL == relayURL {
            HStack(spacing: 8) {
                TextField("Server URL", text: $editingRelayDraft)
                    .textFieldStyle(.roundedBorder)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("myProfileEditRelayInput-\(relayIdentifier(relayURL))")

                Button("Save") {
                    manager.dispatch(.updateNostrRelay(oldRelayUrl: relayURL, newRelayUrl: editingRelayDraft))
                    editingRelayURL = nil
                    editingRelayDraft = ""
                }
                .buttonStyle(IrisPrimaryButtonStyle())

                Button("Cancel") {
                    editingRelayURL = nil
                    editingRelayDraft = ""
                }
                .buttonStyle(IrisSecondaryButtonStyle())
            }
        } else {
            HStack(spacing: 8) {
                Circle()
                    .fill(relayRowStatusColor(relayURL, status: manager.state.networkStatus, palette: palette))
                    .frame(width: 8, height: 8)
                    .accessibilityHidden(true)

                Text(relayURL)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .accessibilityIdentifier("myProfileRelayRow-\(relayIdentifier(relayURL))")

                Spacer(minLength: 8)

                if let label = relayRowStatusLabel(relayURL, status: manager.state.networkStatus) {
                    Text(label)
                        .font(.system(.caption2, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }

                Button {
                    editingRelayURL = relayURL
                    editingRelayDraft = relayURL
                } label: {
                    Image(systemName: "pencil")
                }
                .buttonStyle(.irisPlain)
                .accessibilityLabel("Edit server")

                Button(role: .destructive) {
                    manager.dispatch(.removeNostrRelay(relayUrl: relayURL))
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.irisPlain)
                .accessibilityLabel("Delete server")
            }
        }
    }

    private func relayIdentifier(_ relayURL: String) -> String {
        relayURL
            .replacingOccurrences(of: "://", with: "-")
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ".", with: "-")
            .replacingOccurrences(of: ":", with: "-")
    }

    private func relayRowStatusColor(_ relayURL: String, status: NetworkStatusSnapshot?, palette: IrisPalette) -> Color {
        guard let status, status.relayUrls.contains(relayURL) else {
            return palette.muted.opacity(0.55)
        }
        switch relayConnection(relayURL, status: status)?.status {
        case "connected":
            return Color(red: 34.0 / 255.0, green: 197.0 / 255.0, blue: 94.0 / 255.0)
        case "connecting", "sleeping":
            return Color(red: 234.0 / 255.0, green: 179.0 / 255.0, blue: 8.0 / 255.0)
        case "offline", "blocked":
            return Color(red: 239.0 / 255.0, green: 68.0 / 255.0, blue: 68.0 / 255.0)
        default:
            return palette.muted.opacity(0.55)
        }
    }

    private func relayRowStatusLabel(_ relayURL: String, status: NetworkStatusSnapshot?) -> String? {
        switch relayConnection(relayURL, status: status)?.status {
        case "connected": return "Online"
        case "connecting": return "Connecting"
        case "sleeping": return "Waiting"
        case "offline": return "Offline"
        case "blocked": return "Blocked"
        default: return nil
        }
    }

    private func relayConnection(_ relayURL: String, status: NetworkStatusSnapshot?) -> RelayConnectionSnapshot? {
        status?.relayConnections.first { $0.url == relayURL }
    }
}

struct ProfileEditorCard: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    @Binding var profileName: String
    @Binding var profileAbout: String
    let openProfilePicture: (IrisProfilePictureViewerItem) -> Void
    let showQrCode: () -> Void
    @State private var showingProfilePicturePicker = false
    @State private var showingProfilePictureSourceMenu = false
    #if os(iOS)
    @State private var showingProfilePictureCamera = false
    #endif
    #if canImport(PhotosUI)
    @State private var showingProfilePicturePhotoPicker = false
    @State private var pickedProfilePicturePhotos: [PhotosPickerItem] = []
    #endif

    var body: some View {
        VStack(spacing: 14) {
            IrisSectionCard {
                VStack(spacing: 12) {
                    profileAvatar

                    Button(manager.state.busy.uploadingAttachment ? "Uploading..." : "Edit photo") {
                        presentProfilePictureSource()
                    }
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .disabled(!account.hasOwnerSigningAuthority || manager.state.busy.uploadingAttachment)
                    .accessibilityIdentifier("myProfileUploadPictureButton")
                }
                .frame(maxWidth: .infinity)
                .onAppear {
                    profileName = account.displayName
                    profileAbout = account.about ?? ""
                }
                .irisOnChange(of: account.displayName) { value in
                    profileName = value
                }
                .irisOnChange(of: account.about) { value in
                    profileAbout = value ?? ""
                }
            }

            IrisSectionCard {
                ProfileEditorInputRow(systemImage: "person", title: "Name") {
                    TextField("Name", text: $profileName)
                        .textFieldStyle(.plain)
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.textPrimary)
                        .disabled(!account.hasOwnerSigningAuthority)
                        .accessibilityIdentifier("myProfileDisplayNameInput")
                }

                Divider().overlay(palette.border)

                ProfileEditorInputRow(systemImage: "text.alignleft", title: "About", alignment: .top) {
                    TextField("About", text: $profileAbout, axis: .vertical)
                        .lineLimit(2...5)
                        .textFieldStyle(.plain)
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.textPrimary)
                        .disabled(!account.hasOwnerSigningAuthority)
                        .accessibilityIdentifier("myProfileAboutInput")
                }

                if profileMetadataChanged {
                    Button("Save") {
                        manager.updateProfileMetadata(name: profileName, pictureURL: account.pictureUrl, about: profileAbout)
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .disabled(!account.hasOwnerSigningAuthority || normalizedProfileName.isEmpty)
                    .accessibilityIdentifier("myProfileSaveProfileButton")
                }
            }

            IrisSectionCard {
                Button {
                    showQrCode()
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "qrcode")
                            .frame(width: 24)
                        Text("Show QR code")
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(.system(.footnote, weight: .semibold))
                            .foregroundStyle(palette.muted)
                    }
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)
                .accessibilityIdentifier("myProfileQrButton")

                Divider().overlay(palette.border)

                IrisCopyButton(
                    label: "Copy user ID",
                    value: account.npub,
                    style: .menuRow
                )
            }
        }
        .fileImporter(
            isPresented: $showingProfilePicturePicker,
            allowedContentTypes: [.image],
            allowsMultipleSelection: false
        ) { result in
            if case let .success(urls) = result, let url = urls.first {
                manager.uploadProfilePicture(fileURL: url)
            }
        }
        .confirmationDialog(
            "Choose a profile photo",
            isPresented: $showingProfilePictureSourceMenu,
            titleVisibility: .hidden
        ) {
            #if os(iOS)
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button("Take Photo") { showingProfilePictureCamera = true }
            }
            #endif
            #if canImport(PhotosUI)
            Button("Photo Library") { showingProfilePicturePhotoPicker = true }
            #endif
            Button("Files") { showingProfilePicturePicker = true }
            Button("Cancel", role: .cancel) {}
        }
        #if os(iOS)
        .sheet(isPresented: $showingProfilePictureCamera) {
            IrisCameraImagePicker { url in
                manager.uploadProfilePicture(fileURL: url)
            }
            .ignoresSafeArea()
        }
        #endif
        #if canImport(PhotosUI)
        .photosPicker(
            isPresented: $showingProfilePicturePhotoPicker,
            selection: $pickedProfilePicturePhotos,
            maxSelectionCount: 1,
            matching: .images
        )
        .irisOnChange(of: pickedProfilePicturePhotos) { items in
            handlePickedProfilePicturePhotos(items)
        }
        #endif
    }

    private func presentProfilePictureSource() {
        if let testPath = ProcessInfo.processInfo.environment["IRIS_UI_TEST_PROFILE_PICTURE_PATH"],
           !testPath.isEmpty {
            manager.uploadProfilePicture(fileURL: URL(fileURLWithPath: testPath))
            return
        }
        #if canImport(PhotosUI)
        showingProfilePictureSourceMenu = true
        #else
        showingProfilePicturePicker = true
        #endif
    }

    #if canImport(PhotosUI)
    private func handlePickedProfilePicturePhotos(_ items: [PhotosPickerItem]) {
        guard let item = items.first else { return }
        pickedProfilePicturePhotos = []
        Task {
            guard let url = await loadPickedPhotoItem(item, directoryName: "iris-profile-picks") else { return }
            await MainActor.run {
                manager.uploadProfilePicture(fileURL: url)
            }
        }
    }
    #endif

    @ViewBuilder
    private var profileAvatar: some View {
        let label = account.displayName.isEmpty ? "Profile" : account.displayName
        if let item = IrisProfilePictureViewerItem(
            label: label,
            pictureUrl: account.pictureUrl,
            accessibilityIdentifier: "myProfilePictureViewer"
        ) {
            Button {
                openProfilePicture(item)
            } label: {
                IrisAvatar(
                    label: label,
                    size: 104,
                    emphasize: false,
                    pictureUrl: account.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager,
                    loadedImageIdentifier: "myProfileAvatarImage"
                )
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Open profile picture")
            .accessibilityIdentifier("myProfilePictureButton")
        } else {
            IrisAvatar(
                label: label,
                size: 104,
                emphasize: false,
                pictureUrl: account.pictureUrl,
                preferences: manager.state.preferences,
                manager: manager,
                loadedImageIdentifier: "myProfileAvatarImage"
            )
        }
    }

    private var profileMetadataChanged: Bool {
        normalizedProfileName != account.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            || normalizedProfileAbout != (account.about ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedProfileName: String {
        profileName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedProfileAbout: String {
        profileAbout.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

struct ProfileEditorInputRow<Content: View>: View {
    @Environment(\.irisPalette) private var palette
    let systemImage: String
    let title: String
    var alignment: VerticalAlignment = .center
    let content: () -> Content

    init(
        systemImage: String,
        title: String,
        alignment: VerticalAlignment = .center,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.systemImage = systemImage
        self.title = title
        self.alignment = alignment
        self.content = content
    }

    var body: some View {
        HStack(alignment: alignment, spacing: 14) {
            Image(systemName: systemImage)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(palette.muted)
                .frame(width: 24, height: 24)
                .padding(.top, alignment == .top ? 2 : 0)

            VStack(alignment: .leading, spacing: 5) {
                Text(title)
                    .font(.system(.footnote, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                content()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
