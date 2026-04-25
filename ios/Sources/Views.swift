import Foundation
import SwiftUI

private let irisSourceURL = URL(string: "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs")!
private let irisSourceLabel = "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs"
private let disappearingMessageOptions: [(String, UInt64?)] = [
    ("Off", nil),
    ("5 minutes", 300),
    ("1 hour", 3_600),
    ("24 hours", 86_400),
    ("1 week", 604_800),
    ("1 month", 2_592_000),
    ("3 months", 7_776_000),
]

private func proxiedImageURL(
    _ rawURL: String?,
    preferences: PreferencesSnapshot,
    width: UInt32? = nil,
    height: UInt32? = nil,
    square: Bool = false
) -> String? {
    guard let rawURL else {
        return nil
    }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
        return nil
    }
    return proxiedImageUrl(
        originalSrc: trimmed,
        preferences: preferences,
        width: width,
        height: height,
        square: square
    )
}

private enum SecretExportKind: Identifiable {
    case owner
    case device

    var id: String {
        switch self {
        case .owner: return "owner"
        case .device: return "device"
        }
    }
}

struct RootView: View {
    @ObservedObject var manager: AppManager

    var body: some View {
        IrisTheme {
            ZStack(alignment: .top) {
                BackgroundFill()

                NavigationShell(
                    title: screenTitle(manager.activeScreen),
                    canGoBack: manager.canNavigateBack,
                    onBack: manager.navigateBack,
                    networkStatus: manager.state.networkStatus,
                    backBadgeCount: backUnreadCount,
                    leading: topBarLeadingItem,
                    trailing: topBarTrailingItem
                ) {
                    content
                }

                if let toast = manager.toastMessage {
                    ToastView(text: toast)
                        .padding(.top, 14)
                }

                if manager.bootstrapInFlight {
                    LoadingOverlay()
                }
            }
        }
    }

    @ViewBuilder
    private var content: some View {
        switch manager.activeScreen {
        case .welcome:
            WelcomeScreen(manager: manager)
        case .createAccount:
            CreateAccountScreen(manager: manager)
        case .restoreAccount:
            RestoreAccountScreen(manager: manager)
        case .addDevice:
            AddDeviceScreen(manager: manager, awaitingApproval: false)
        case .chatList:
            ChatListScreen(manager: manager)
        case .newChat:
            NewChatScreen(manager: manager)
        case .newGroup:
            NewGroupScreen(manager: manager)
        case .createInvite:
            CreateInviteScreen(manager: manager)
        case .joinInvite:
            JoinInviteScreen(manager: manager)
        case .settings:
            SettingsScreen(manager: manager)
        case .chat(let chatId):
            ChatScreen(manager: manager, chatId: chatId)
        case .groupDetails(let groupId):
            GroupDetailsScreen(manager: manager, groupId: groupId)
        case .deviceRoster:
            DeviceRosterScreen(manager: manager)
        case .awaitingDeviceApproval:
            AddDeviceScreen(manager: manager, awaitingApproval: true)
        case .deviceRevoked:
            DeviceRevokedScreen(manager: manager)
        }
    }

    private var topBarLeadingItem: AnyView {
        guard case .chatList = manager.activeScreen, let account = manager.state.account else {
            return AnyView(EmptyView())
        }

        return AnyView(
            Button(action: { manager.dispatch(.pushScreen(screen: .settings)) }) {
                IrisAvatar(
                    label: account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName,
                    emphasize: true,
                    imageURL: proxiedImageURL(account.pictureUrl, preferences: manager.state.preferences, width: 88, height: 88, square: true)
                )
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("chatListProfileButton")
        )
    }

    private var topBarTrailingItem: AnyView {
        guard case .chat(let chatId) = manager.activeScreen,
              let chat = manager.state.currentChat,
              chat.chatId == chatId else {
            return AnyView(EmptyView())
        }

        return AnyView(
            Menu {
                if chat.kind == .group, let groupId = chat.groupId {
                    Button("Group details") {
                        manager.dispatch(.pushScreen(screen: .groupDetails(groupId: groupId)))
                    }
                }
                Menu("Disappearing messages") {
                    ForEach(disappearingMessageOptions, id: \.0) { label, ttlSeconds in
                        Button(action: {
                            manager.dispatch(.setChatMessageTtl(chatId: chat.chatId, ttlSeconds: ttlSeconds))
                        }) {
                            if chat.messageTtlSeconds == ttlSeconds {
                                Label(label, systemImage: "checkmark")
                            } else {
                                Text(label)
                            }
                        }
                    }
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 16, weight: .semibold))
                    .frame(width: 40, height: 40)
            }
            .buttonStyle(IrisSecondaryButtonStyle(compact: true))
            .accessibilityIdentifier("chatOverflowButton")
        )
    }

    private var backUnreadCount: UInt64 {
        guard case .chat(let activeChatId) = manager.activeScreen else {
            return 0
        }
        return manager.state.chatList
            .filter { $0.chatId != activeChatId }
            .reduce(UInt64(0)) { $0 + $1.unreadCount }
    }

    private func screenTitle(_ screen: Screen) -> String {
        switch screen {
        case .welcome: return "Welcome"
        case .createAccount: return "Create Account"
        case .restoreAccount: return "Restore Account"
        case .addDevice: return "Add Device"
        case .chatList: return "Chats"
        case .newChat: return "New Chat"
        case .newGroup: return "New Group"
        case .createInvite: return "Invite"
        case .joinInvite: return "Join Chat"
        case .settings: return "Settings"
        case .chat:
            return manager.state.currentChat?.displayName ?? "Chat"
        case .groupDetails:
            return "Group"
        case .deviceRoster:
            return "Manage Devices"
        case .awaitingDeviceApproval:
            return "Finish Linking"
        case .deviceRevoked:
            return "Device Revoked"
        }
    }
}

struct NavigationShell<Content: View>: View {
    let title: String
    let canGoBack: Bool
    let onBack: () -> Void
    let networkStatus: NetworkStatusSnapshot?
    let backBadgeCount: UInt64
    let leading: AnyView
    let trailing: AnyView
    let content: () -> Content

    init(
        title: String,
        canGoBack: Bool,
        onBack: @escaping () -> Void,
        networkStatus: NetworkStatusSnapshot? = nil,
        backBadgeCount: UInt64 = 0,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView()),
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.networkStatus = networkStatus
        self.backBadgeCount = backBadgeCount
        self.leading = leading
        self.trailing = trailing
        self.content = content
    }

    var body: some View {
        VStack(spacing: 0) {
            IrisTopBar(
                title: title,
                canGoBack: canGoBack,
                onBack: onBack,
                backBadgeCount: backBadgeCount,
                leading: leading,
                trailing: trailing
            )

            if shouldShowRelayStatusDots(networkStatus) {
                RelayStatusDots(status: networkStatus)
                    .padding(.top, 8)
                    .transition(.opacity)
                    .accessibilityIdentifier("relayStatusDots")
            }

            content()
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
    }
}

private struct RelayStatusDots: View {
    @Environment(\.irisPalette) private var palette
    let status: NetworkStatusSnapshot?

    var body: some View {
        HStack(spacing: 5) {
            ForEach(0..<relayDotCount(status), id: \.self) { _ in
                Circle()
                    .fill(relayStatusColor(status, palette: palette))
                    .frame(width: 7, height: 7)
            }
        }
        .accessibilityLabel(relayStatusLabel(status))
    }
}

private func shouldShowRelayStatusDots(_ status: NetworkStatusSnapshot?) -> Bool {
    guard let status else { return false }
    return !status.relayUrls.isEmpty
}

private func relayDotCount(_ status: NetworkStatusSnapshot?) -> Int {
    min(max(status?.relayUrls.count ?? 1, 1), 3)
}

private func relayStatusColor(_ status: NetworkStatusSnapshot?, palette: IrisPalette) -> Color {
    guard let status, !status.relayUrls.isEmpty else {
        return palette.muted.opacity(0.55)
    }
    if status.syncing || status.pendingOutboundCount > 0 || status.pendingGroupControlCount > 0 {
        return Color(red: 234.0 / 255.0, green: 179.0 / 255.0, blue: 8.0 / 255.0)
    }
    return Color(red: 34.0 / 255.0, green: 197.0 / 255.0, blue: 94.0 / 255.0)
}

private func relayStatusLabel(_ status: NetworkStatusSnapshot?) -> String {
    guard let status else { return "Relays offline" }
    if status.syncing || status.pendingOutboundCount > 0 || status.pendingGroupControlCount > 0 {
        return "Relays syncing"
    }
    return "Relays connected"
}

private struct OwnerPresentation {
    let primary: String
    let secondary: String?
}

private func trimmedText(_ value: String?) -> String? {
    guard let value else { return nil }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}

private func primaryDisplayName(displayName: String, fallback: String) -> String {
    trimmedText(displayName) ?? fallbackProfileNameForIdentity(fallback)
}

private func secondaryDisplayName(_ secondary: String?, primary: String) -> String? {
    guard let secondary = trimmedText(secondary) else {
        return nil
    }
    return secondary.caseInsensitiveCompare(primary) == .orderedSame ? nil : secondary
}

private func sameOwner(_ owner: String, hex: String?, npub: String?) -> Bool {
    let rawOwner = owner.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let normalizedOwner = normalizePeerInput(input: owner).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let candidates = [hex, npub]
        .compactMap(trimmedText)
        .map { $0.lowercased() }
    return candidates.contains(rawOwner) || candidates.contains(normalizedOwner)
}

private func fallbackProfileNameForIdentity(_ identity: String) -> String {
    let adjectives = [
        "Amber", "Bright", "Calm", "Clear", "Golden", "Lunar",
        "Nova", "Quiet", "Silver", "Solar", "Velvet", "Wild"
    ]
    let nouns = [
        "Aurora", "Comet", "Echo", "Falcon", "Harbor", "Listener",
        "Otter", "Raven", "Signal", "Sparrow", "Tide", "Voyager"
    ]
    let trimmed = identity.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return "Quiet Listener" }

    let hash = trimmed.utf8.reduce(UInt32(0)) { partial, byte in
        partial &* 31 &+ UInt32(byte)
    }
    let adjective = adjectives[Int(hash) % adjectives.count]
    let noun = nouns[(Int(hash) / adjectives.count) % nouns.count]
    return "\(adjective) \(noun)"
}

struct WelcomeScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    var body: some View {
        IrisScrollScreen {
            IrisAdaptiveColumns {
                IrisSectionCard(accent: true) {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("welcomeChooserCard")

                    Text("Iris Chat")
                        .font(.system(.largeTitle, design: .rounded, weight: .bold))
                        .foregroundStyle(palette.textPrimary)
                    Button("Create account") {
                        manager.dispatch(.pushScreen(screen: .createAccount))
                    }
                    .buttonStyle(IrisPrimaryButtonStyle())
                    .accessibilityIdentifier("welcomeCreateAction")

                    Button("Restore account") {
                        manager.dispatch(.pushScreen(screen: .restoreAccount))
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("welcomeRestoreAction")

                    Button("Add this device") {
                        manager.dispatch(.pushScreen(screen: .addDevice))
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("welcomeAddDeviceAction")
                }
            } trailing: {
                IrisSectionCard(accent: manager.trustedTestBuildEnabled()) {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("welcomeSecondaryCard")

                    CardHeader(
                        title: manager.trustedTestBuildEnabled() ? "Trusted test build" : "How this works",
                        subtitle: manager.trustedTestBuildEnabled()
                            ? "This beta uses a controlled relay set and should not be used for sensitive conversations."
                            : nil
                    )

                    if manager.trustedTestBuildEnabled() {
                        Text(manager.buildSummaryText())
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(palette.muted)
                    }
                }
            }
        }
    }
}

struct CreateAccountScreen: View {
    @ObservedObject var manager: AppManager
    @State private var displayName = ""

    var body: some View {
        IrisScrollScreen {
            onboardingBackButton

            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("createAccountScreen")

                CardHeader(
                    title: "Create account"
                )

                TextField("Display name", text: $displayName)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("signupNameField")

                Button(manager.state.busy.creatingAccount ? "Creating…" : "Create account") {
                    manager.createAccount(name: displayName)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(
                    displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    manager.state.busy.creatingAccount
                )
                .accessibilityIdentifier("generateKeyButton")
            }
        }
    }

    private var onboardingBackButton: some View {
        Button("Back") {
            manager.dispatch(.updateScreenStack(stack: []))
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .accessibilityIdentifier("onboardingBackButton")
    }
}

struct RestoreAccountScreen: View {
    @ObservedObject var manager: AppManager
    @State private var restoreInput = ""

    var body: some View {
        IrisScrollScreen {
            onboardingBackButton

            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("restoreAccountScreen")

                CardHeader(
                    title: "Restore account",
                    subtitle: "Use your owner secret key to recover your account on this device."
                )

                TextField("Owner nsec", text: $restoreInput)
                    .irisIdentifierInputModifiers()
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("importKeyField")

                Button(manager.state.busy.restoringSession ? "Restoring…" : "Restore account") {
                    manager.restoreSession(ownerNsec: restoreInput)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(
                    restoreInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    manager.state.busy.restoringSession
                )
                .accessibilityIdentifier("importKeyButton")
            }
        }
    }

    private var onboardingBackButton: some View {
        Button("Back") {
            manager.dispatch(.updateScreenStack(stack: []))
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .accessibilityIdentifier("onboardingBackButton")
    }
}

struct AddDeviceScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let awaitingApproval: Bool

    @State private var ownerInput = ""
    @State private var showingScanner = false

    private var normalizedOwnerInput: String {
        normalizePeerInput(input: ownerInput)
    }

    private var validOwnerInput: Bool {
        !normalizedOwnerInput.isEmpty && isValidPeerInput(input: normalizedOwnerInput)
    }

    var body: some View {
        IrisScrollScreen {
            if !awaitingApproval {
                onboardingBackButton
            }

            IrisAdaptiveColumns {
                IrisSectionCard {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("addDeviceScreen")

                    CardHeader(
                        title: awaitingApproval ? "Finish linking" : "Add this device",
                        subtitle: awaitingApproval
                            ? "Approve this device on the owner device. If it does not appear there yet, use the approval QR as a fallback."
                            : "Scan or paste the owner code from your primary device. This device will create its own invite and then wait for approval there."
                    )

                    if awaitingApproval, let account = manager.state.account {
                        MonoValue(label: "User ID", value: account.npub, identifier: "awaitingApprovalOwnerNpub")
                        MonoValue(label: "Device ID", value: account.deviceNpub, identifier: "awaitingApprovalDeviceNpub")

                        HStack(spacing: 10) {
                            Button("Copy device ID") {
                                manager.copyToClipboard(account.deviceNpub)
                            }
                            .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                        }

                        Button("Logout") {
                            manager.logout()
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                    } else {
                        TextField("User ID or hex", text: $ownerInput)
                            .irisIdentifierInputModifiers()
                            .textFieldStyle(.plain)
                            .irisInputField()
                            .accessibilityIdentifier("linkOwnerInput")

                        if !ownerInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !validOwnerInput {
                            Text("Scanned or pasted owner key is not valid.")
                                .font(.system(.footnote, design: .rounded))
                                .foregroundStyle(.red)
                        }

                        VStack(spacing: 10) {
                            Button("Paste") {
                                ownerInput = normalizePeerInput(input: PlatformClipboard.string() ?? "")
                            }
                            .buttonStyle(IrisSecondaryButtonStyle())
                            .accessibilityIdentifier("linkOwnerPasteButton")

                            if irisSupportsQrScanning {
                                Button("Scan owner QR") { showingScanner = true }
                                    .buttonStyle(IrisSecondaryButtonStyle())
                                    .accessibilityIdentifier("linkOwnerScanQrButton")
                            }

                            Button(manager.state.busy.linkingDevice ? "Continuing…" : "Continue") {
                                manager.startLinkedDevice(ownerInput: normalizedOwnerInput)
                            }
                            .buttonStyle(IrisPrimaryButtonStyle())
                            .disabled(!validOwnerInput || manager.state.busy.linkingDevice)
                            .accessibilityIdentifier("linkExistingAccountButton")
                        }
                    }
                }
            } trailing: {
                addDeviceQrPanel
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                ownerInput = normalizePeerInput(input: code)
                showingScanner = false
            }
        }
    }

    private var onboardingBackButton: some View {
        Button("Back") {
            manager.dispatch(.updateScreenStack(stack: []))
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .accessibilityIdentifier("onboardingBackButton")
    }

    @ViewBuilder
    private var addDeviceQrPanel: some View {
        if awaitingApproval, let account = manager.state.account {
            let qr = DeviceApprovalQr.encode(ownerInput: account.npub, deviceInput: account.deviceNpub)

            IrisSectionCard(accent: true) {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("awaitingApprovalScreen")

                CardHeader(
                    title: "Approval QR",
                    subtitle: "Approve this device from Manage Devices on the owner device, or scan this QR there as a fallback."
                )

                ZStack {
                    QrCodeImage(text: qr)
                        .frame(width: 240, height: 240)
                    Color.clear
                        .accessibilityIdentifier("awaitingApprovalDeviceQrCode")
                }
                .frame(maxWidth: .infinity)

                Button("Copy approval QR") {
                    manager.copyToClipboard(qr)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .accessibilityIdentifier("awaitingApprovalCopyDeviceButton")
            }
        } else {
            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("addDeviceQrPlaceholder")

                CardHeader(
                    title: "Approval QR",
                    subtitle: "After you continue, the approval QR for this device will appear here so the owner can authorize it."
                )

                VStack(spacing: 10) {
                    Image(systemName: "qrcode")
                        .font(.system(size: 56, weight: .medium))
                        .foregroundStyle(palette.muted)
                    Text("QR placeholder")
                        .font(.system(.footnote, design: .rounded))
                        .foregroundStyle(palette.muted)
                }
                .frame(maxWidth: .infinity, minHeight: 240)
            }
        }
    }
}

struct ChatListScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    var body: some View {
        IrisScrollScreen {
            if IrisLayout.usesDesktopChrome, let account = manager.state.account {
                IrisAdaptiveColumns {
                    chatListHeroCard
                } trailing: {
                    accountCard(account)
                }
            } else {
                chatListHeroCard
                if let account = manager.state.account {
                    accountCard(account)
                }
            }

            if manager.state.chatList.isEmpty {
                Text("No chats yet")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 18)
            } else {
                IrisSectionCard {
                    ForEach(Array(manager.state.chatList.enumerated()), id: \.element.chatId) { index, chat in
                        IrisChatRow(
                            title: chat.displayName,
                            preview: chat.isTyping ? "Typing" : (chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"),
                            subtitle: chat.kind == .group ? chat.subtitle : nil,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs),
                            unreadCount: chat.unreadCount,
                            onTap: {
                                manager.dispatch(.openChat(chatId: chat.chatId))
                            }
                        )
                        .accessibilityIdentifier("chatRow-\(String(chat.chatId.prefix(12)))")

                        if index < manager.state.chatList.count - 1 {
                            Divider()
                                .overlay(palette.border)
                        }
                    }
                }
            }
        }
    }

    private var newChatButton: some View {
        Button {
            manager.dispatch(.pushScreen(screen: .newChat))
        } label: {
            Label("New chat", systemImage: "message.fill")
        }
        .buttonStyle(IrisPrimaryButtonStyle())
        .accessibilityIdentifier("chatListNewChatButton")
    }

    private var newGroupButton: some View {
        Button {
            manager.dispatch(.pushScreen(screen: .newGroup))
        } label: {
            Label("New group", systemImage: "person.3.fill")
        }
        .buttonStyle(IrisSecondaryButtonStyle())
        .accessibilityIdentifier("chatListNewGroupButton")
    }

    private var chatListHeroCard: some View {
        IrisSectionCard(accent: true) {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("chatListHeroCard")

            Text("Conversations")
                .font(.system(.title3, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)

            HStack(spacing: 10) {
                newChatButton
                newGroupButton
            }
        }
    }

    private func accountCard(_ account: AccountSnapshot) -> some View {
        IrisSectionCard {
            HStack(spacing: 14) {
                IrisAvatar(
                    label: account.displayName.isEmpty ? account.npub : account.displayName,
                    emphasize: true,
                    imageURL: proxiedImageURL(account.pictureUrl, preferences: manager.state.preferences, width: 96, height: 96, square: true)
                )
                VStack(alignment: .leading, spacing: 4) {
                    Text(account.displayName.isEmpty ? "Your account" : account.displayName)
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
        }
    }
}

struct NewChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @State private var peerInput = ""
    @State private var showingScanner = false

    private var normalizedPeerInput: String {
        normalizePeerInput(input: peerInput)
    }

    private var validPeerInput: Bool {
        !normalizedPeerInput.isEmpty && isValidPeerInput(input: normalizedPeerInput)
    }

    var body: some View {
        IrisScrollScreen {
            VStack(spacing: 18) {
                directChatForm
                secondaryActions
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                handleScannedCode(code)
                showingScanner = false
            }
        }
    }

    private var directChatForm: some View {
        VStack(alignment: .leading, spacing: 12) {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("newChatPrimaryCard")

            Text("User ID")
                .font(.system(.headline, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.textPrimary)

            TextField("User ID, hex, or link", text: $peerInput)
                .irisIdentifierInputModifiers()
                .textFieldStyle(.plain)
                .irisInputField()
                .accessibilityIdentifier("newChatPeerInput")

            if !peerInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !validPeerInput {
                Text("Invalid user ID.")
                    .font(.system(.footnote, design: .rounded))
                    .foregroundStyle(.red)
            }

            HStack(spacing: 10) {
                pasteButton
                scanButton
            }
        }
    }

    private var secondaryActions: some View {
        VStack(spacing: 8) {
            newChatActionRow(
                title: "Join with invite",
                systemImage: "link",
                identifier: "newChatJoinInviteButton"
            ) {
                manager.dispatch(.pushScreen(screen: .joinInvite))
            }

            newChatActionRow(
                title: "Create invite",
                systemImage: "square.and.arrow.up",
                identifier: "newChatCreateInviteButton"
            ) {
                manager.dispatch(.pushScreen(screen: .createInvite))
            }

            newChatActionRow(
                title: "New group",
                systemImage: "person.3.fill",
                identifier: "newChatNewGroupButton"
            ) {
                manager.dispatch(.pushScreen(screen: .newGroup))
            }
        }
    }

    private var pasteButton: some View {
        Button("Paste") {
            handleNewChatInput(PlatformClipboard.string() ?? "")
        }
        .buttonStyle(IrisSecondaryButtonStyle())
        .accessibilityIdentifier("newChatPasteButton")
    }

    private var scanButton: some View {
        Group {
            if irisSupportsQrScanning {
                Button("Scan QR") { showingScanner = true }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("newChatScanQrButton")
            }
        }
    }

    private func newChatActionRow(
        title: String,
        systemImage: String,
        identifier: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.system(.body, weight: .semibold))
                    .frame(width: 22)
                    .foregroundStyle(palette.accent)
                Text(title)
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
        .buttonStyle(.plain)
        .accessibilityIdentifier(identifier)
    }

    private func handleScannedCode(_ code: String) {
        handleNewChatInput(code)
    }

    private func handleNewChatInput(_ raw: String) {
        let normalized = normalizePeerInput(input: raw)
        if !normalized.isEmpty, isValidPeerInput(input: normalized) {
            peerInput = normalized
            manager.dispatch(.createChat(peerInput: normalized))
            return
        }

        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            peerInput = trimmed
            manager.dispatch(.acceptInvite(inviteInput: trimmed))
        }
    }
}

struct CreateInviteScreen: View {
    @ObservedObject var manager: AppManager
    @State private var shareText: String?

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard(accent: true) {
                CardHeader(title: "Invite")

                if manager.state.busy.creatingInvite && manager.state.publicInvite == nil {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 24)
                } else if let invite = manager.state.publicInvite {
                    QrCodeImage(text: invite.url)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .accessibilityIdentifier("createInviteQrCode")

                    MonoValue(label: "Invite link", value: invite.url, identifier: "createInviteUrl")

                    HStack(spacing: 10) {
                        Button("Copy") {
                            manager.copyToClipboard(invite.url)
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("createInviteCopyButton")

                        Button("Share") {
                            shareText = invite.url
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
        .sheet(item: Binding(
            get: { shareText.map(SharePayload.init(text:)) },
            set: { shareText = $0?.text }
        )) { payload in
            ShareSheet(text: payload.text)
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

                TextField("Invite link", text: $inviteInput)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("joinInviteInput")

                HStack(spacing: 10) {
                    Button("Paste") {
                        inviteInput = PlatformClipboard.string() ?? ""
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("joinInvitePasteButton")

                    if irisSupportsQrScanning {
                        Button("Scan QR") { showingScanner = true }
                            .buttonStyle(IrisSecondaryButtonStyle())
                            .accessibilityIdentifier("joinInviteScanQrButton")
                    }
                }

                Button(manager.state.busy.acceptingInvite ? "Joining…" : "Join chat") {
                    manager.dispatch(.acceptInvite(inviteInput: normalizedInviteInput))
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(normalizedInviteInput.isEmpty || manager.state.busy.acceptingInvite)
                .accessibilityIdentifier("joinInviteAcceptButton")
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                inviteInput = code
                showingScanner = false
            }
        }
    }
}

struct NewGroupScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    @State private var name = ""
    @State private var memberInput = ""
    @State private var selectedOwners = Set<String>()
    @State private var showingScanner = false

    private var normalizedMemberInput: String {
        normalizePeerInput(input: memberInput)
    }

    private var localOwnerHex: String? {
        manager.state.account?.publicKeyHex
    }

    private var existingDirectChats: [ChatThreadSnapshot] {
        manager.state.chatList.filter { chat in
            chat.kind == .direct && chat.chatId != localOwnerHex
        }
    }

    private var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
        !selectedOwners.isEmpty &&
        !manager.state.busy.creatingGroup
    }

    private func ownerPresentation(for owner: String) -> OwnerPresentation {
        if let chat = existingDirectChats.first(where: { sameOwner(owner, hex: $0.chatId, npub: $0.subtitle) }) {
            let primary = primaryDisplayName(displayName: chat.displayName, fallback: normalizePeerInput(input: owner))
            return OwnerPresentation(
                primary: primary,
                secondary: secondaryDisplayName(chat.subtitle, primary: primary)
            )
        }

        if let account = manager.state.account, sameOwner(owner, hex: account.publicKeyHex, npub: account.npub) {
            let primary = primaryDisplayName(displayName: account.displayName, fallback: account.npub)
            return OwnerPresentation(primary: primary, secondary: nil)
        }

        let normalized = normalizePeerInput(input: owner)
        return OwnerPresentation(primary: fallbackProfileNameForIdentity(normalized), secondary: nil)
    }

    var body: some View {
        IrisScrollScreen {
            IrisAdaptiveColumns {
                IrisSectionCard(accent: true) {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("newGroupPrimaryCard")

                    CardHeader(title: "Create group")

                    TextField("Weekend plans", text: $name)
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("newGroupNameInput")
                }
            } trailing: {
                IrisSectionCard {
                    CardHeader(title: "Add members")

                    TextField("User ID, hex, or nostr:…", text: $memberInput)
                        .irisIdentifierInputModifiers()
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("newGroupMemberInput")

                    VStack(spacing: 10) {
                        pasteMemberButton
                        scanMemberButton
                        addMemberButton
                    }

                    if !selectedOwners.isEmpty {
                        FlowWrap(spacing: 8, lineSpacing: 8) {
                            ForEach(selectedOwners.sorted(), id: \.self) { owner in
                                let presentation = ownerPresentation(for: owner)
                                SelectedMemberChip(
                                    title: presentation.primary,
                                    subtitle: presentation.secondary,
                                    onRemove: { selectedOwners.remove(owner) }
                                )
                            }
                        }
                    }
                }
            }

            if !existingDirectChats.isEmpty {
                IrisSectionCard {
                    CardHeader(title: "Existing chats")

                    ForEach(Array(existingDirectChats.enumerated()), id: \.element.chatId) { index, chat in
                        Button {
                            if selectedOwners.contains(chat.chatId) {
                                selectedOwners.remove(chat.chatId)
                            } else {
                                selectedOwners.insert(chat.chatId)
                            }
                        } label: {
                            HStack(spacing: 12) {
                                IrisAvatar(label: chat.displayName, size: 38, emphasize: selectedOwners.contains(chat.chatId))
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(chat.displayName)
                                        .font(.system(.headline, design: .rounded, weight: .semibold))
                                        .foregroundStyle(palette.textPrimary)
                                    if let subtitle = secondaryDisplayName(chat.subtitle, primary: chat.displayName) {
                                        Text(subtitle)
                                            .font(.system(.footnote, design: .rounded))
                                            .foregroundStyle(palette.muted)
                                    }
                                }
                                Spacer()
                                Image(systemName: selectedOwners.contains(chat.chatId) ? "checkmark.circle.fill" : "circle")
                                    .foregroundStyle(selectedOwners.contains(chat.chatId) ? palette.accent : palette.muted)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)

                        if index < existingDirectChats.count - 1 {
                            Divider().overlay(palette.border)
                        }
                    }
                }
            }

            Button(manager.state.busy.creatingGroup ? "Creating…" : "Create group") {
                manager.dispatch(
                    .createGroup(
                        name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                        memberInputs: selectedOwners.sorted()
                    )
                )
            }
            .buttonStyle(IrisPrimaryButtonStyle())
            .disabled(!canCreate)
            .accessibilityIdentifier("newGroupCreateButton")
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                addMember(code)
                showingScanner = false
            }
        }
    }

    private var pasteMemberButton: some View {
        Button("Paste") {
            memberInput = normalizePeerInput(input: PlatformClipboard.string() ?? "")
        }
        .buttonStyle(IrisSecondaryButtonStyle())
        .accessibilityIdentifier("newGroupPasteButton")
    }

    private var scanMemberButton: some View {
        Group {
            if irisSupportsQrScanning {
                Button("Scan QR") { showingScanner = true }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("newGroupScanQrButton")
            }
        }
    }

    private var addMemberButton: some View {
        Button("Add") {
            addMember(normalizedMemberInput)
        }
        .buttonStyle(IrisPrimaryButtonStyle())
        .disabled(!isValidPeerInput(input: normalizedMemberInput))
        .accessibilityIdentifier("newGroupAddMemberButton")
    }

    private func addMember(_ raw: String) {
        let normalized = normalizePeerInput(input: raw)
        guard !normalized.isEmpty, isValidPeerInput(input: normalized) else {
            return
        }
        guard normalized != localOwnerHex else {
            return
        }
        selectedOwners.insert(normalized)
        memberInput = ""
    }
}

struct ChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chatId: String

    @State private var draft = ""
    @State private var selectedAttachments: [StagedAttachment] = []
    @State private var isNearBottom = true
    @State private var shouldFollowLatest = true
    @State private var forceScrollToLatest = false
    @State private var timelineViewportMaxY: CGFloat = 0
    @State private var timelineBottomMaxY: CGFloat = .greatestFiniteMagnitude
    @State private var initialScrollPending = true
    @State private var renderedMessageCount = 0
    @State private var replyTarget: ChatMessageSnapshot?
    @State private var imageViewerItem: ImageViewerItem?
    @State private var lastTypingSentAt: Date?

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var body: some View {
        VStack(spacing: 0) {
            Group {
                if let chat {
                    VStack(spacing: 0) {
                        ScrollViewReader { proxy in
                            ZStack(alignment: .bottomTrailing) {
                                ScrollView {
                                    let visibleMessages = chat.messages
                                    LazyVStack(spacing: 0) {
                                        ForEach(Array(visibleMessages.enumerated()), id: \.element.id) { index, message in
                                            let previous = index > 0 ? visibleMessages[index - 1] : nil
                                            let next = index + 1 < visibleMessages.count ? visibleMessages[index + 1] : nil
                                            let showDayChip = previous == nil || !irisSameTimelineDay(previous!.createdAtSecs, message.createdAtSecs)
                                            let isFirstInCluster = irisStartsMessageCluster(
                                                previous: previous,
                                                message: message,
                                                chatKind: chat.kind
                                            )
                                            let isLastInCluster = next.map {
                                                irisStartsMessageCluster(
                                                    previous: message,
                                                    message: $0,
                                                    chatKind: chat.kind
                                                )
                                            } ?? true

                                            ChatMessageRow(
                                                message: message,
                                                chatKind: chat.kind,
                                                showDayChip: showDayChip,
                                                isFirstInCluster: isFirstInCluster,
                                                isLastInCluster: isLastInCluster,
                                                reactions: message.reactions,
                                                onReply: {
                                                    replyTarget = message
                                                },
                                                onReact: { emoji in
                                                    manager.dispatch(
                                                        .toggleReaction(
                                                            chatId: chatId,
                                                            messageId: message.id,
                                                            emoji: emoji
                                                        )
                                                    )
                                                },
                                                onDelete: {
                                                    manager.dispatch(
                                                        .deleteLocalMessage(chatId: chatId, messageId: message.id)
                                                    )
                                                    if replyTarget?.id == message.id {
                                                        replyTarget = nil
                                                    }
                                                },
                                                downloadAttachment: { attachment in
                                                    await manager.downloadAttachment(attachment)
                                                },
                                                openAttachment: { attachment in
                                                    await manager.openAttachment(attachment)
                                                },
                                                onOpenImage: { data, filename in
                                                    imageViewerItem = ImageViewerItem(data: data, filename: filename)
                                                }
                                            )
                                            .id(message.id)
                                        }

                                        Color.clear
                                            .frame(height: 1)
                                            .id(ChatTimelineAnchor.bottom)
                                            .background(
                                                GeometryReader { geometry in
                                                    Color.clear.preference(
                                                        key: ChatTimelineBottomMaxYPreferenceKey.self,
                                                        value: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name)).maxY
                                                    )
                                                }
                                            )
                                            .accessibilityHidden(true)
                                    }
                                    .padding(.horizontal, IrisLayout.usesDesktopChrome ? 18 : 14)
                                    .padding(.vertical, 10)
                                    .accessibilityIdentifier("chatTimeline")
                                }
                                .coordinateSpace(name: ChatTimelineCoordinateSpace.name)
                                .overlay {
                                    GeometryReader { geometry in
                                        Color.clear.preference(
                                            key: ChatTimelineViewportMaxYPreferenceKey.self,
                                            value: geometry.frame(in: .named(ChatTimelineCoordinateSpace.name)).maxY
                                        )
                                    }
                                }
                                .irisInteractiveKeyboardDismiss()
                                .irisOnChange(of: chatId) { _ in
                                    initialScrollPending = true
                                    isNearBottom = true
                                    shouldFollowLatest = true
                                    forceScrollToLatest = false
                                    renderedMessageCount = 0
                                }
                                .onPreferenceChange(ChatTimelineViewportMaxYPreferenceKey.self) { value in
                                    timelineViewportMaxY = value
                                    let nearBottom = chatTimelineIsNearBottom(
                                        viewportMaxY: value,
                                        bottomMaxY: timelineBottomMaxY
                                    )
                                    isNearBottom = nearBottom
                                    if chat.messages.count == renderedMessageCount {
                                        shouldFollowLatest = nearBottom
                                    }
                                }
                                .onPreferenceChange(ChatTimelineBottomMaxYPreferenceKey.self) { value in
                                    timelineBottomMaxY = value
                                    let nearBottom = chatTimelineIsNearBottom(
                                        viewportMaxY: timelineViewportMaxY,
                                        bottomMaxY: value
                                    )
                                    isNearBottom = nearBottom
                                    if chat.messages.count == renderedMessageCount {
                                        shouldFollowLatest = nearBottom
                                    }
                                }
                                .task(id: chat.messages.last?.id) {
                                    guard !chat.messages.isEmpty else {
                                        initialScrollPending = true
                                        shouldFollowLatest = true
                                        forceScrollToLatest = false
                                        renderedMessageCount = 0
                                        return
                                    }
                                    let messageCount = chat.messages.count
                                    let messageCountIncreased = messageCount > renderedMessageCount
                                    let shouldScroll = initialScrollPending
                                        || forceScrollToLatest
                                        || (messageCountIncreased && shouldFollowLatest)
                                    renderedMessageCount = messageCount
                                    if shouldScroll {
                                        scrollToBottom(proxy: proxy, animated: !initialScrollPending)
                                        initialScrollPending = false
                                        shouldFollowLatest = true
                                    }
                                    if forceScrollToLatest {
                                        forceScrollToLatest = false
                                    }
                                }
                                .task(id: forceScrollToLatest) {
                                    guard forceScrollToLatest, !chat.messages.isEmpty else {
                                        return
                                    }
                                    scrollToBottom(proxy: proxy, animated: true)
                                }

                                if !isNearBottom && !chat.messages.isEmpty {
                                    Button {
                                        shouldFollowLatest = true
                                        scrollToBottom(proxy: proxy, animated: true)
                                    } label: {
                                        Image(systemName: "arrow.down")
                                            .font(.system(size: 18, weight: .bold))
                                            .foregroundStyle(palette.onAccent)
                                            .frame(width: 48, height: 48)
                                            .background(
                                                Circle()
                                                    .fill(palette.accent)
                                            )
                                    }
                                    .padding(.trailing, 18)
                                    .padding(.bottom, 18)
                                    .buttonStyle(.plain)
                                    .shadow(color: .black.opacity(0.16), radius: 16, y: 10)
                                    .accessibilityIdentifier("chatJumpToBottom")
                                }
                            }
                        }

                        if !chat.typingIndicators.isEmpty {
                            IrisTypingIndicatorRow(indicators: chat.typingIndicators)
                        }

                        if let replyTarget {
                            IrisReplyComposerStrip(message: replyTarget) {
                                self.replyTarget = nil
                            }
                        }

                        IrisComposerBar(
                            draft: $draft,
                            attachments: $selectedAttachments,
                            placeholder: "Message",
                            isSending: manager.state.busy.sendingMessage,
                            isUploading: manager.state.busy.uploadingAttachment,
                            onDraftChange: {
                                sendTypingIfNeeded()
                            },
                            onAttach: { urls in
                                do {
                                    selectedAttachments.append(
                                        contentsOf: try manager.stageOutgoingAttachments(urls)
                                    )
                                } catch {
                                    manager.showAttachmentOpenError()
                                }
                            }
                        ) {
                            let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !text.isEmpty || !selectedAttachments.isEmpty else { return }
                            shouldFollowLatest = true
                            forceScrollToLatest = true
                            let outgoingText = replyEncodedMessage(reply: replyTarget, text: text)
                            replyTarget = nil
                            if selectedAttachments.isEmpty {
                                draft = ""
                                manager.dispatch(.sendMessage(chatId: chatId, text: outgoingText))
                            } else {
                                let attachments = selectedAttachments
                                selectedAttachments = []
                                draft = ""
                                manager.sendAttachments(chatId: chatId, attachments: attachments, caption: outgoingText)
                            }
                        }
                    }
                } else {
                    VStack(spacing: 0) {
                        Spacer()
                        IrisSectionCard {
                            Text("Loading chat…")
                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                .foregroundStyle(palette.textPrimary)
                        }
                        .padding(.horizontal, 16)
                        Spacer()
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay {
            if let imageViewerItem {
                IrisImageViewer(item: imageViewerItem) {
                    self.imageViewerItem = nil
                }
            }
        }
        .task(id: seenReceiptToken(for: chat)) {
            guard let chat else { return }
            let incomingIds = chat.messages
                .filter { !$0.isOutgoing && $0.delivery != .seen }
                .map(\.id)
            guard !incomingIds.isEmpty else { return }
            manager.dispatch(.markMessagesSeen(chatId: chat.chatId, messageIds: incomingIds))
        }
    }

    private func sendTypingIfNeeded() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let now = Date()
        if let lastTypingSentAt, now.timeIntervalSince(lastTypingSentAt) < 3 {
            return
        }
        lastTypingSentAt = now
        manager.dispatch(.sendTyping(chatId: chatId))
    }

    private func seenReceiptToken(for chat: CurrentChatSnapshot?) -> String {
        guard let chat else { return "" }
        return chat.messages
            .filter { !$0.isOutgoing && $0.delivery != .seen }
            .map(\.id)
            .joined(separator: ",")
    }

    private func scrollToBottom(proxy: ScrollViewProxy, animated: Bool) {
        DispatchQueue.main.async {
            if animated {
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo(ChatTimelineAnchor.bottom, anchor: .bottom)
                }
            } else {
                proxy.scrollTo(ChatTimelineAnchor.bottom, anchor: .bottom)
            }
        }
    }

}

private let irisMessageClusterGapSecs: UInt64 = 60

private func irisStartsMessageCluster(
    previous: ChatMessageSnapshot?,
    message: ChatMessageSnapshot,
    chatKind: ChatKind
) -> Bool {
    guard let previous else {
        return true
    }
    if !irisSameTimelineDay(previous.createdAtSecs, message.createdAtSecs) {
        return true
    }
    if previous.isOutgoing != message.isOutgoing {
        return true
    }
    if chatKind == .group && !message.isOutgoing && previous.author != message.author {
        return true
    }
    let gap = message.createdAtSecs >= previous.createdAtSecs
        ? message.createdAtSecs - previous.createdAtSecs
        : 0
    if gap <= irisMessageClusterGapSecs {
        return false
    }
    if chatKind == .direct {
        let previousMinute = previous.createdAtSecs / 60
        let messageMinute = message.createdAtSecs / 60
        if messageMinute >= previousMinute && messageMinute - previousMinute <= 1 {
            return false
        }
    }
    return true
}

private enum ChatTimelineCoordinateSpace {
    static let name = "chatTimelineCoordinateSpace"
}

private enum ChatTimelineAnchor {
    static let bottom = "chatTimelineBottom"
}

private struct ChatTimelineViewportMaxYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private struct ChatTimelineBottomMaxYPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = .greatestFiniteMagnitude

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private func chatTimelineIsNearBottom(viewportMaxY: CGFloat, bottomMaxY: CGFloat) -> Bool {
    guard viewportMaxY > 0, bottomMaxY.isFinite else {
        return true
    }
    return bottomMaxY <= viewportMaxY + 24
}

struct GroupDetailsScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let groupId: String

    @State private var groupName = ""
    @State private var memberInput = ""
    @State private var showingScanner = false

    private var normalizedMemberInput: String {
        normalizePeerInput(input: memberInput)
    }

    var body: some View {
        IrisScrollScreen {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("groupDetailsScreen")

            if let details = manager.state.groupDetails {
                IrisSectionCard(accent: true) {
                    CardHeader(
                        title: "Group settings",
                        subtitle: "Created by \(details.createdByDisplayName). Revision \(details.revision)."
                    )

                    TextField("Name", text: Binding(
                        get: { groupName.isEmpty ? details.name : groupName },
                        set: { groupName = $0 }
                    ))
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("groupDetailsNameInput")

                    if details.canManage {
                        Button(manager.state.busy.updatingGroup ? "Renaming…" : "Rename") {
                            let nextName = groupName.trimmingCharacters(in: .whitespacesAndNewlines)
                            manager.dispatch(.updateGroupName(groupId: groupId, name: nextName.isEmpty ? details.name : nextName))
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .disabled(manager.state.busy.updatingGroup)
                        .accessibilityIdentifier("groupDetailsRenameButton")
                    }
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Members",
                        subtitle: "\(details.members.count) people in this conversation."
                    )

                    ForEach(Array(details.members.enumerated()), id: \.element.ownerPubkeyHex) { index, member in
                        let primary = primaryDisplayName(displayName: member.displayName, fallback: member.npub)
                        HStack(alignment: .top, spacing: 12) {
                            IrisAvatar(label: primary, size: 38, emphasize: member.isLocalOwner)

                            VStack(alignment: .leading, spacing: 4) {
                                Text(primary)
                                    .font(.system(.headline, design: .rounded, weight: .semibold))
                                    .foregroundStyle(palette.textPrimary)
                                if member.isLocalOwner {
                                    IrisInfoPill("You")
                                }
                            }

                            Spacer()

                            if details.canManage && !member.isLocalOwner {
                                Button("Remove", role: .destructive) {
                                    manager.dispatch(.removeGroupMember(groupId: groupId, ownerPubkeyHex: member.ownerPubkeyHex))
                                }
                                .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                                .accessibilityIdentifier("groupDetailsRemoveMember-\(String(member.ownerPubkeyHex.prefix(12)))")
                            }
                        }

                        if index < details.members.count - 1 {
                            Divider().overlay(palette.border)
                        }
                    }
                }

                if details.canManage {
                    IrisSectionCard {
                        CardHeader(
                            title: "Add members",
                            subtitle: "Approve a new member by scan or paste."
                        )

                        TextField("Member user ID, hex, or nostr:…", text: $memberInput)
                            .irisIdentifierInputModifiers()
                            .textFieldStyle(.plain)
                            .irisInputField()
                            .accessibilityIdentifier("groupDetailsAddMemberInput")

                        VStack(spacing: 10) {
                            if irisSupportsQrScanning {
                                Button("Scan member QR") { showingScanner = true }
                                    .buttonStyle(IrisSecondaryButtonStyle())
                                    .accessibilityIdentifier("groupDetailsScanQrButton")
                            }

                            Button(manager.state.busy.updatingGroup ? "Adding…" : "Add members") {
                                manager.dispatch(.addGroupMembers(groupId: groupId, memberInputs: [normalizedMemberInput]))
                                memberInput = ""
                            }
                            .buttonStyle(IrisPrimaryButtonStyle())
                            .disabled(!isValidPeerInput(input: normalizedMemberInput) || manager.state.busy.updatingGroup)
                            .accessibilityIdentifier("groupDetailsAddMembersButton")
                        }
                    }
                }
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                memberInput = normalizePeerInput(input: code)
                showingScanner = false
            }
        }
    }
}

struct DeviceRosterScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @State private var deviceInput = ""
    @State private var showingScanner = false

    private var resolvedInput: ResolvedDeviceAuthorizationInput? {
        guard let roster = manager.state.deviceRoster else {
            return nil
        }
        return resolveDeviceAuthorizationInput(
            rawInput: deviceInput,
            ownerNpub: roster.ownerNpub,
            ownerPublicKeyHex: roster.ownerPublicKeyHex
        )
    }

    private var isCurrentDeviceRegistered: Bool {
        guard let roster = manager.state.deviceRoster else {
            return false
        }
        return roster.devices.contains { $0.devicePubkeyHex == roster.currentDevicePublicKeyHex }
    }

    private var deviceAccessSubtitle: String {
        guard let roster = manager.state.deviceRoster else {
            return ""
        }
        if roster.canManageDevices {
            return "Scan a link invite from the new device, or paste a device ID as fallback."
        }
        if isCurrentDeviceRegistered {
            return "Read-only on this device. Use a session with your main Secret Key to add or remove devices."
        }
        return "This linked-device session is read-only and is not registered. Sign in here with your main Secret Key if you want to register this device."
    }

    var body: some View {
        IrisScrollScreen {
            if let roster = manager.state.deviceRoster {
                IrisSectionCard(accent: true) {
                    CardHeader(
                        title: "Owner devices",
                        subtitle: roster.canManageDevices ? "This device can approve and remove linked devices." : "This device can view linked devices only."
                    )

                    MonoValue(label: "User ID", value: roster.ownerNpub, identifier: "deviceRosterOwnerNpub")
                    MonoValue(label: "Device ID", value: roster.currentDeviceNpub, identifier: "deviceRosterCurrentDeviceNpub")
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Approve a new device",
                        subtitle: deviceAccessSubtitle
                    )

                    TextField("Device ID, hex, or approval code", text: $deviceInput)
                        .irisIdentifierInputModifiers()
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("deviceRosterAddInput")

                    if let error = resolvedInput?.errorMessage {
                        Text(error)
                            .font(.system(.footnote, design: .rounded))
                            .foregroundStyle(.red)
                    }

                    VStack(spacing: 10) {
                        if irisSupportsQrScanning {
                            Button("Scan QR") { showingScanner = true }
                                .buttonStyle(IrisSecondaryButtonStyle())
                                .accessibilityIdentifier("deviceRosterScanButton")
                        }
                        Button(manager.state.busy.updatingRoster ? "Authorizing…" : "Authorize") {
                            let normalized = resolvedInput?.deviceInput ?? ""
                            manager.addAuthorizedDevice(deviceInput: normalized)
                            deviceInput = ""
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .disabled(
                            roster.canManageDevices == false ||
                            manager.state.busy.updatingRoster ||
                            (resolvedInput?.deviceInput.isEmpty ?? true)
                        )
                        .accessibilityIdentifier("deviceRosterAddButton")
                    }
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Device Access",
                        subtitle: "\(roster.devices.count) linked device(s)."
                    )

                    if roster.devices.isEmpty {
                        Text("No registered devices")
                            .font(.system(.headline, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .accessibilityIdentifier("deviceRosterEmptyState")
                        Text("Authorized device keys will appear here after the roster is published.")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                    } else {
                        ForEach(Array(roster.devices.enumerated()), id: \.element.devicePubkeyHex) { index, device in
                            DeviceRosterRow(manager: manager, device: device, canManageDevices: roster.canManageDevices)
                            if index < roster.devices.count - 1 {
                                Divider().overlay(palette.border)
                            }
                        }
                    }
                }
            } else {
                IrisSectionCard {
                    Text("No roster available.")
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                deviceInput = code
                showingScanner = false
            }
        }
    }
}

private struct DeviceRosterRow: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let device: DeviceEntrySnapshot
    let canManageDevices: Bool
    @State private var showingRemoveConfirmation = false

    private var displayTitle: String {
        device.isCurrentDevice ? PlatformDeviceLabels.currentDeviceLabel : "Linked device"
    }

    private var displaySubtitle: String {
        let client = device.isCurrentDevice ? PlatformDeviceLabels.currentClientLabel : "Iris Chat"
        return "\(client) - \(device.deviceNpub)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                IrisAvatar(label: displayTitle, size: 36, emphasize: device.isCurrentDevice)
                VStack(alignment: .leading, spacing: 4) {
                    Text(displayTitle)
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Text(displaySubtitle)
                        .font(.system(.footnote, design: .monospaced))
                        .foregroundStyle(palette.muted)
                        .lineLimit(2)
                }
            }

            HStack(spacing: 8) {
                IrisInfoPill(device.isAuthorized ? "Authorized" : "Pending", tint: device.isAuthorized ? .green : .orange)
                if device.isStale {
                    IrisInfoPill("Stale", tint: .red)
                }
            }

            if canManageDevices && !device.isCurrentDevice {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 10) {
                        if !device.isAuthorized {
                            approveButton
                        }
                        removeButton
                    }
                    VStack(spacing: 10) {
                        if !device.isAuthorized {
                            approveButton
                        }
                        removeButton
                    }
                }
            }
        }
        .accessibilityIdentifier("deviceRosterRow-\(String(device.devicePubkeyHex.prefix(12)))")
    }

    private var approveButton: some View {
        Button(manager.state.busy.updatingRoster ? "Approving…" : "Approve") {
            manager.addAuthorizedDevice(deviceInput: device.devicePubkeyHex)
        }
        .buttonStyle(IrisPrimaryButtonStyle())
        .disabled(manager.state.busy.updatingRoster)
        .accessibilityIdentifier("deviceRosterApprove-\(String(device.devicePubkeyHex.prefix(12)))")
    }

    private var removeButton: some View {
        Button("Remove device", role: .destructive) {
            showingRemoveConfirmation = true
        }
        .buttonStyle(IrisSecondaryButtonStyle())
        .disabled(manager.state.busy.updatingRoster)
        .accessibilityIdentifier("deviceRosterRemove-\(String(device.devicePubkeyHex.prefix(12)))")
        .alert("Delete Device?", isPresented: $showingRemoveConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.removeAuthorizedDevice(devicePubkeyHex: device.devicePubkeyHex)
            }
            .accessibilityIdentifier("deviceRosterConfirmRemove-\(String(device.devicePubkeyHex.prefix(12)))")
        } message: {
            Text("This device will no longer be authorized for encrypted messaging.")
        }
    }
}

struct DeviceRevokedScreen: View {
    @ObservedObject var manager: AppManager

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard(accent: true) {
                Text("This device has been removed from the roster.")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)

                Text("Acknowledge this state to return to a fresh shell.")
                    .font(.system(.body, design: .rounded))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)

                Button("Acknowledge") {
                    manager.dispatch(.acknowledgeRevokedDevice)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .accessibilityIdentifier("deviceRevokedLogoutButton")
            }
            .accessibilityIdentifier("deviceRevokedScreen")
        }
    }
}

struct SettingsScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @State private var shareText: String?
    @State private var pendingSecretExport: SecretExportKind?
    @State private var showingDeleteAllConfirmation = false
    @State private var profileName = ""
    @State private var profilePictureURL = ""
    @State private var profilePictureViewerURL: URL?
    @State private var newRelayURL = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""

    var body: some View {
        ZStack {
            BackgroundFill()

            IrisScrollScreen {
                LazyVGrid(columns: settingsColumns, spacing: 18) {
                    if let account = manager.state.account {
                        ProfileEditorCard(
                            manager: manager,
                            account: account,
                            profileName: $profileName,
                            profilePictureURL: $profilePictureURL,
                            openProfilePicture: { profilePictureViewerURL = $0 },
                            manageDevices: {
                                manager.dispatch(.pushScreen(screen: .deviceRoster))
                            }
                        )
                    }

                    if manager.trustedTestBuildEnabled() {
                        IrisSectionCard {
                            CardHeader(
                                title: "Trusted test build",
                                subtitle: "This build uses a controlled relay set and is intended for trusted testing only."
                            )
                        }
                    }

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
                        .accessibilityIdentifier("myProfileReadReceiptsToggle")

                        Toggle(
                            "Notifications",
                            isOn: Binding(
                                get: { manager.state.preferences.desktopNotificationsEnabled },
                                set: { enabled in
                                    manager.dispatch(.setDesktopNotificationsEnabled(enabled: enabled))
                                }
                            )
                        )
                        .accessibilityIdentifier("myProfileDesktopNotificationsToggle")

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
                            .accessibilityIdentifier("myProfileStartupAtLoginToggle")
                        }
                    }

                    IrisSectionCard {
                        CardHeader(title: "Media")
                        ImageProxySettingsSection(manager: manager)
                    }

                    IrisSectionCard {
                        CardHeader(title: "Relays")
                        NostrRelaySettingsSection(
                            manager: manager,
                            newRelayURL: $newRelayURL,
                            editingRelayURL: $editingRelayURL,
                            editingRelayDraft: $editingRelayDraft
                        )
                    }

                    IrisSectionCard {
                        CardHeader(title: "Security")

                        if manager.state.account?.hasOwnerSigningAuthority == true {
                            Button {
                                pendingSecretExport = .owner
                            } label: {
                                Label("Export secret key", systemImage: "key.fill")
                            }
                            .buttonStyle(IrisSecondaryButtonStyle())
                            .accessibilityIdentifier("myProfileExportOwnerKeyButton")
                        }

                        Button {
                            pendingSecretExport = .device
                        } label: {
                            Label("Export device key", systemImage: "key.fill")
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("myProfileExportDeviceKeyButton")
                    }

                    IrisSectionCard {
                        CardHeader(
                            title: "About",
                            subtitle: "Version and source details for this build."
                        )
                        HStack(spacing: 10) {
                            Image(systemName: "info.circle.fill")
                                .foregroundStyle(palette.accent)
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
                                    .foregroundStyle(palette.accent)
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

                    IrisSectionCard {
                        CardHeader(
                            title: "Support",
                            subtitle: "Capture a support bundle or inspect current build metadata."
                        )
                        Text("Build \(manager.buildSummaryText())")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.textPrimary)
                        Text("Relay set \(manager.relaySetIdText())")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)

                        if let networkStatus = manager.state.networkStatus {
                            Text(
                                "Network \(networkStatus.syncing ? "syncing" : "idle") · " +
                                    "\(networkStatus.relayUrls.count) relays · " +
                                    "\(networkStatus.recentEventCount) events"
                            )
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                            .accessibilityIdentifier("myProfileNetworkStatusValue")

                            Text(networkStatus.relayUrls.joined(separator: ", "))
                                .font(.system(.caption, design: .rounded))
                                .foregroundStyle(palette.muted)
                                .accessibilityIdentifier("myProfileRelayUrlsValue")

                            if let category = networkStatus.lastDebugCategory {
                                Text("Last debug \(category)")
                                    .font(.system(.caption, design: .rounded))
                                    .foregroundStyle(palette.muted)
                            }
                        }

                        Button("Share support bundle") {
                            shareText = manager.supportBundleJson()
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .accessibilityIdentifier("myProfileShareSupportBundleButton")

                        Button("Copy support bundle") {
                            manager.copyToClipboard(manager.supportBundleJson())
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("myProfileCopySupportBundleButton")

                    }

                    IrisSectionCard {
                        CardHeader(
                            title: "Danger Zone",
                            subtitle: "Local identity, keys, messages, and cached files are removed from this device."
                        )

                        Button("Logout", role: .destructive) {
                            manager.logout()
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("myProfileLogoutButton")

                        Button("Delete all data", role: .destructive) {
                            showingDeleteAllConfirmation = true
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("myProfileDeleteAllDataButton")
                    }
                }
            }
        }
        .overlay {
            if let profilePictureViewerURL {
                IrisProfilePictureViewer(url: profilePictureViewerURL) {
                    self.profilePictureViewerURL = nil
                }
            }
        }
        .accessibilityIdentifier("settingsScreen")
        .sheet(item: Binding(
            get: { shareText.map(SharePayload.init(text:)) },
            set: { shareText = $0?.text }
        )) { payload in
            ShareSheet(text: payload.text)
        }
        .alert(item: $pendingSecretExport) { exportKind in
            let isDeviceExport = exportKind == .device
            return Alert(
                title: Text(isDeviceExport ? "Export Device Key" : "Export Secret Key"),
                message: Text(isDeviceExport
                    ? "This device key only unlocks this linked device. Copy it from this device?"
                    : "Your secret key gives full access to your identity. Never share it with anyone. Store it securely."),
                primaryButton: .cancel(Text("Cancel")),
                secondaryButton: .default(Text(isDeviceExport ? "Copy Device Key" : "Copy")) {
                    let secret = isDeviceExport ? manager.exportDeviceNsec() : manager.exportOwnerNsec()
                    guard let secret, !secret.isEmpty else {
                        manager.showSecretExportUnavailable()
                        return
                    }
                    manager.copyToClipboard(secret)
                }
            )
        }
        .alert("Delete All Data?", isPresented: $showingDeleteAllConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete Everything", role: .destructive) {
                manager.resetAppState()
            }
            .accessibilityIdentifier("myProfileConfirmDeleteAllDataButton")
        } message: {
            Text("This permanently deletes your identity, keys, messages, and cached files from this device. This action cannot be undone.")
        }
    }

    private var settingsColumns: [GridItem] {
        #if os(macOS)
        [GridItem(.adaptive(minimum: 360, maximum: 520), spacing: 18, alignment: .top)]
        #else
        [GridItem(.flexible(), spacing: 18, alignment: .top)]
        #endif
    }

}

private struct ImageProxySettingsSection: View {
    @ObservedObject var manager: AppManager

    var body: some View {
        Toggle("Image proxy", isOn: imageProxyEnabled)
            .accessibilityIdentifier("myProfileImageProxyToggle")

        imageProxyTextField(
            title: "Proxy URL",
            text: imageProxyUrl,
            identifier: "myProfileImageProxyUrlInput"
        )

        imageProxyTextField(
            title: "Key hex",
            text: imageProxyKeyHex,
            identifier: "myProfileImageProxyKeyInput"
        )

        imageProxyTextField(
            title: "Salt hex",
            text: imageProxySaltHex,
            identifier: "myProfileImageProxySaltInput"
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
        identifier: String
    ) -> some View {
        TextField(title, text: text)
            .textFieldStyle(.roundedBorder)
            .autocorrectionDisabled()
            .accessibilityIdentifier(identifier)
    }
}

private struct NostrRelaySettingsSection: View {
    @ObservedObject var manager: AppManager
    @Binding var newRelayURL: String
    @Binding var editingRelayURL: String?
    @Binding var editingRelayDraft: String

    private var relayURLs: [String] {
        manager.state.networkStatus?.relayUrls ?? manager.state.preferences.nostrRelayUrls
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Nostr relays")
                .font(.system(.headline, design: .rounded, weight: .semibold))

            ForEach(relayURLs, id: \.self) { relayURL in
                relayRow(relayURL)
            }

            HStack(spacing: 8) {
                TextField("wss://relay.example", text: $newRelayURL)
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
                .accessibilityLabel("Add relay")
                .accessibilityIdentifier("myProfileAddRelayButton")
            }

            Button("Reset relays") {
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
                TextField("Relay URL", text: $editingRelayDraft)
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
                Text(relayURL)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .accessibilityIdentifier("myProfileRelayRow-\(relayIdentifier(relayURL))")

                Spacer(minLength: 8)

                Button {
                    editingRelayURL = relayURL
                    editingRelayDraft = relayURL
                } label: {
                    Image(systemName: "pencil")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Edit relay")

                Button(role: .destructive) {
                    manager.dispatch(.removeNostrRelay(relayUrl: relayURL))
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Delete relay")
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
}

private struct ProfileEditorCard: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    @Binding var profileName: String
    @Binding var profilePictureURL: String
    let openProfilePicture: (URL) -> Void
    let manageDevices: () -> Void

    var body: some View {
        IrisSectionCard(accent: true) {
            HStack(spacing: 14) {
                profileAvatar
                VStack(alignment: .leading, spacing: 4) {
                    Text(account.displayName.isEmpty ? "Owner profile" : account.displayName)
                        .font(.system(.title3, design: .rounded, weight: .bold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
            .onAppear {
                profileName = account.displayName
                profilePictureURL = account.pictureUrl ?? ""
            }
            .irisOnChange(of: account.displayName) { value in
                profileName = value
            }
            .irisOnChange(of: account.pictureUrl) { value in
                profilePictureURL = value ?? ""
            }

            TextField("Display name", text: $profileName)
                .textFieldStyle(.roundedBorder)
                .disabled(!account.hasOwnerSigningAuthority)
                .accessibilityIdentifier("myProfileDisplayNameInput")

            TextField("Profile picture URL", text: $profilePictureURL)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
                .disabled(!account.hasOwnerSigningAuthority)
                .accessibilityIdentifier("myProfilePictureUrlInput")

            Button("Save profile") {
                manager.updateProfileMetadata(name: profileName, pictureURL: profilePictureURL)
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .disabled(!account.hasOwnerSigningAuthority || normalizedProfileName.isEmpty || !profileMetadataChanged)
            .accessibilityIdentifier("myProfileSaveProfileButton")

            Button {
                manageDevices()
            } label: {
                Label("Manage devices", systemImage: "laptopcomputer.and.iphone")
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .accessibilityIdentifier("myProfileManageDevicesButton")

            QrCodeImage(text: account.npub, size: 220)
                .frame(maxWidth: .infinity, alignment: .center)
                .accessibilityIdentifier("myProfileQrCode")

            MonoValue(label: "Device ID", value: account.deviceNpub)

            VStack(spacing: 10) {
                Button("Copy user ID") { manager.copyToClipboard(account.npub) }
                    .buttonStyle(IrisSecondaryButtonStyle())
                Button("Copy device ID") { manager.copyToClipboard(account.deviceNpub) }
                    .buttonStyle(IrisSecondaryButtonStyle())
            }
        }
    }

    @ViewBuilder
    private var profileAvatar: some View {
        let label = account.displayName.isEmpty ? "Profile" : account.displayName
        let trimmedURL = account.pictureUrl?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let displayURL = proxiedImageURL(trimmedURL, preferences: manager.state.preferences, width: 1024, height: 1024)
        if !trimmedURL.isEmpty, let url = URL(string: displayURL ?? trimmedURL) {
            Button {
                openProfilePicture(url)
            } label: {
                IrisAvatar(
                    label: label,
                    size: 52,
                    emphasize: true,
                    imageURL: proxiedImageURL(trimmedURL, preferences: manager.state.preferences, width: 104, height: 104, square: true)
                )
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Open profile picture")
            .accessibilityIdentifier("myProfilePictureButton")
        } else {
            IrisAvatar(
                label: label,
                size: 52,
                emphasize: true,
                imageURL: proxiedImageURL(account.pictureUrl, preferences: manager.state.preferences, width: 104, height: 104, square: true)
            )
        }
    }

    private var profileMetadataChanged: Bool {
        normalizedProfileName != account.displayName.trimmingCharacters(in: .whitespacesAndNewlines) ||
            normalizedProfilePictureURL != (account.pictureUrl ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedProfileName: String {
        profileName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedProfilePictureURL: String {
        profilePictureURL.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

private struct IrisProfilePictureViewer: View {
    let url: URL
    let onClose: () -> Void

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.opacity(0.92)
                .ignoresSafeArea()
                .onTapGesture(perform: onClose)

            AsyncImage(url: url) { phase in
                switch phase {
                case .success(let image):
                    image
                        .resizable()
                        .scaledToFit()
                        .padding(22)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                default:
                    ProgressView()
                        .tint(.white)
                }
            }

            Button(action: onClose) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 30, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.9))
                    .padding(18)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close profile picture")
        }
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .accessibilityIdentifier("myProfilePictureViewer")
        .zIndex(20)
    }
}

private struct BackgroundFill: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        LinearGradient(
            colors: [
                palette.background,
                palette.background,
                palette.panelAlt.opacity(0.28)
            ],
            startPoint: .top,
            endPoint: .bottom
        )
        .ignoresSafeArea()
    }
}

private struct ToastView: View {
    @Environment(\.irisPalette) private var palette
    let text: String

    var body: some View {
        Text(text)
            .font(.system(.subheadline, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.textPrimary)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                    .fill(palette.panel)
                    .overlay(
                        RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                            .stroke(palette.border, lineWidth: 1)
                    )
            )
    }
}

private struct LoadingOverlay: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        ZStack {
            palette.background.opacity(0.4).ignoresSafeArea()
            VStack(spacing: 14) {
                ProgressView()
                    .progressViewStyle(.circular)
                Text("Loading")
                    .font(.system(.headline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 22)
            .background(
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .fill(palette.panel)
                    .overlay(
                        RoundedRectangle(cornerRadius: 24, style: .continuous)
                            .stroke(palette.border, lineWidth: 1)
                    )
            )
        }
    }
}

private struct CardHeader: View {
    @Environment(\.irisPalette) private var palette
    let title: String
    let subtitle: String?

    init(title: String, subtitle: String? = nil) {
        self.title = title
        self.subtitle = subtitle
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.system(.title3, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)
            if let subtitle {
                Text(subtitle)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.muted)
            }
        }
    }
}

private struct MonoValue: View {
    @Environment(\.irisPalette) private var palette
    let label: String
    let value: String
    let identifier: String?

    init(label: String, value: String, identifier: String? = nil) {
        self.label = label
        self.value = value
        self.identifier = identifier
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.system(.caption, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.muted)
            if let identifier {
                Text(value)
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(palette.textPrimary)
                    .textSelection(.enabled)
                    .accessibilityIdentifier(identifier)
            } else {
                Text(value)
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(palette.textPrimary)
                    .textSelection(.enabled)
            }
        }
    }
}

private struct SelectedMemberChip: View {
    @Environment(\.irisPalette) private var palette
    let title: String
    let subtitle: String?
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .lineLimit(1)
                if let subtitle {
                    Text(subtitle)
                        .font(.system(.caption2, design: .monospaced, weight: .medium))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }
            }
            Button(action: onRemove) {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .bold))
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("memberChipRemove")
        }
        .foregroundStyle(palette.textPrimary)
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                .fill(palette.panel)
                .overlay(
                    RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                        .stroke(palette.border, lineWidth: 1)
                )
        )
    }
}

private struct ChatMessageRow: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let chatKind: ChatKind
    let showDayChip: Bool
    let isFirstInCluster: Bool
    let isLastInCluster: Bool
    let reactions: [MessageReactionSnapshot]
    let onReply: () -> Void
    let onReact: (String) -> Void
    let onDelete: () -> Void
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var isHovering = false

    private var bodyParts: ReplyParsedMessage {
        parseReplyEncodedMessage(message.body)
    }

    var body: some View {
        VStack(spacing: 0) {
            if showDayChip {
                HStack {
                    Spacer()
                    IrisDayChip(text: irisTimelineDay(message.createdAtSecs))
                    Spacer()
                }
                .padding(.vertical, 14)
            }

            VStack(
                alignment: message.isOutgoing ? .trailing : .leading,
                spacing: 6
            ) {
                if chatKind == .group && !message.isOutgoing && isFirstInCluster {
                    Text(message.author)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }

                HStack(alignment: .center, spacing: 6) {
                    if IrisLayout.usesDesktopChrome && isHovering && message.isOutgoing {
                        ChatMessageActionDock(
                            onReact: { onReact("❤️") },
                            onReply: onReply,
                            onCopyInfo: {
                                PlatformClipboard.setString("Message \(message.id) · \(irisMessageClock(message.createdAtSecs))")
                            },
                            onDelete: onDelete
                        )
                    }

                    VStack(alignment: message.isOutgoing ? .trailing : .leading, spacing: 8) {
                        if let reply = bodyParts.reply {
                            ReplyPreviewView(reply: reply, isOutgoing: message.isOutgoing)
                        }
                        if !bodyParts.body.isEmpty {
                            Text(
                                linkedMessageAttributedString(
                                    bodyParts.body,
                                    linkColor: message.isOutgoing ? palette.onBubbleMine : palette.accentAlt
                                )
                            )
                            .font(.system(.body, design: .rounded))
                            .multilineTextAlignment(message.isOutgoing ? .trailing : .leading)
                        }
                        ForEach(Array(message.attachments.enumerated()), id: \.offset) { _, attachment in
                            ChatAttachmentView(
                                attachment: attachment,
                                isOutgoing: message.isOutgoing,
                                downloadAttachment: downloadAttachment,
                                openAttachment: openAttachment,
                                onOpenImage: onOpenImage
                            )
                        }
                    }
                    .foregroundStyle(message.isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 11)
                    .background(
                        RoundedRectangle(cornerRadius: 22, style: .continuous)
                            .fill(message.isOutgoing ? palette.bubbleMine : palette.bubbleTheirs)
                    )
                    .contextMenu {
                        Button("Reply", action: onReply)
                        Button("React 👍") { onReact("👍") }
                        Button("React ❤️") { onReact("❤️") }
                        Button("Copy") {
                            PlatformClipboard.setString(copyableMessageText(message))
                        }
                        Button("Delete locally", role: .destructive, action: onDelete)
                    }
                    .accessibilityIdentifier("chatMessage-\(message.id)")

                    if IrisLayout.usesDesktopChrome && isHovering && !message.isOutgoing {
                        ChatMessageActionDock(
                            onReact: { onReact("❤️") },
                            onReply: onReply,
                            onCopyInfo: {
                                PlatformClipboard.setString("Message \(message.id) · \(irisMessageClock(message.createdAtSecs))")
                            },
                            onDelete: onDelete
                        )
                    }
                }

                if !reactions.isEmpty {
                    ReactionRow(reactions: reactions, isOutgoing: message.isOutgoing)
                }

                if isLastInCluster {
                    HStack(spacing: 6) {
                        if message.expiresAtSecs != nil {
                            Image(systemName: "timer")
                                .font(.system(.caption2, design: .rounded, weight: .semibold))
                                .accessibilityLabel("Disappearing message")
                                .accessibilityIdentifier("chatMessageDisappearing-\(message.id)")
                        }
                        Text(irisMessageClock(message.createdAtSecs))
                            .font(.system(.caption2, design: .rounded, weight: .medium))
                        if message.isOutgoing {
                            IrisDeliveryGlyph(delivery: message.delivery)
                        }
                    }
                    .foregroundStyle(palette.muted)
                    .padding(.top, 1)
                }
            }
            .frame(maxWidth: .infinity, alignment: message.isOutgoing ? .trailing : .leading)
            .contentShape(Rectangle())
            .onHover { isHovering = $0 }
            .padding(.top, isFirstInCluster ? 10 : 4)
            .padding(.bottom, isLastInCluster ? 10 : 0)
        }
    }
}

private struct IrisDeliveryGlyph: View {
    @Environment(\.irisPalette) private var palette
    let delivery: DeliveryState

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: 11, weight: .bold))
            .foregroundStyle(tint)
            .accessibilityLabel(irisDeliveryLabel(delivery))
    }

    private var systemName: String {
        switch delivery {
        case .queued, .pending:
            return "clock"
        case .sent:
            return "checkmark"
        case .received:
            return "checkmark"
        case .seen:
            return "checkmark.circle.fill"
        case .failed:
            return "exclamationmark.circle.fill"
        }
    }

    private var tint: Color {
        switch delivery {
        case .received:
            return palette.accentAlt
        case .seen:
            return palette.accent
        case .failed:
            return .red
        default:
            return palette.muted
        }
    }
}

private struct ChatMessageActionDock: View {
    @Environment(\.irisPalette) private var palette
    let onReact: () -> Void
    let onReply: () -> Void
    let onCopyInfo: () -> Void
    let onDelete: () -> Void

    var body: some View {
        HStack(spacing: 2) {
            dockButton("face.smiling.fill", action: onReact)
            dockButton("arrowshape.turn.up.left.fill", action: onReply)
            Menu {
                Button("Message info", action: onCopyInfo)
                Button("Delete message", role: .destructive, action: onDelete)
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .bold))
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
        }
        .padding(5)
        .background(
            Capsule(style: .continuous)
                .fill(palette.toolbar.opacity(0.96))
        )
    }

    private func dockButton(_ systemName: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: 26, height: 24)
        }
        .buttonStyle(.plain)
    }
}

private struct IrisTypingIndicatorRow: View {
    @Environment(\.irisPalette) private var palette
    let indicators: [TypingIndicatorSnapshot]

    private var label: String {
        guard let first = indicators.first else { return "" }
        if indicators.count == 1 {
            return "\(first.displayName) is typing"
        }
        return "\(first.displayName) and \(indicators.count - 1) more are typing"
    }

    var body: some View {
        HStack(spacing: 8) {
            HStack(spacing: 4) {
                Circle().frame(width: 5, height: 5)
                Circle().frame(width: 5, height: 5)
                Circle().frame(width: 5, height: 5)
            }
            .foregroundStyle(palette.muted)

            Text(label)
                .font(.system(.caption, design: .rounded, weight: .medium))
                .foregroundStyle(palette.muted)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 22 : 18)
        .padding(.vertical, 8)
        .background(Rectangle().fill(palette.toolbar))
        .accessibilityIdentifier("chatTypingIndicator")
    }
}

private struct ReactionRow: View {
    @Environment(\.irisPalette) private var palette
    let reactions: [MessageReactionSnapshot]
    let isOutgoing: Bool

    var body: some View {
        HStack(spacing: 5) {
            ForEach(reactions, id: \.emoji) { reaction in
                Text("\(reaction.emoji) \(reaction.count)")
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 4)
                    .background(
                        Capsule(style: .continuous)
                            .fill(reaction.reactedByMe ? palette.accent.opacity(0.18) : palette.panel)
                    )
            }
        }
        .frame(maxWidth: .infinity, alignment: isOutgoing ? .trailing : .leading)
    }
}

private struct IrisReplyComposerStrip: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Rectangle()
                .fill(palette.accent)
                .frame(width: 3)
                .clipShape(Capsule())
            VStack(alignment: .leading, spacing: 2) {
                Text(message.author)
                    .font(.system(.caption, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
                Text(replySnippet(for: message))
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
            }
            Spacer()
            Button(action: onCancel) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.muted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 18 : 16)
        .padding(.vertical, 8)
        .background(palette.toolbar)
        .accessibilityIdentifier("chatReplyComposer")
    }
}

private struct ReplyPreviewView: View {
    @Environment(\.irisPalette) private var palette
    let reply: ReplyPreview
    let isOutgoing: Bool

    var body: some View {
        HStack(spacing: 8) {
            Rectangle()
                .fill(isOutgoing ? palette.onBubbleMine.opacity(0.6) : palette.accent)
                .frame(width: 3)
                .clipShape(Capsule())
            VStack(alignment: .leading, spacing: 2) {
                Text(reply.author)
                    .font(.system(.caption, design: .rounded, weight: .bold))
                Text(reply.body)
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .lineLimit(2)
                    .opacity(0.82)
            }
        }
        .frame(maxWidth: 280, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
        )
    }
}

private enum ChatAttachmentCategory: String {
    case image = "Image"
    case video = "Video"
    case audio = "Audio"
    case archive = "Archive"
    case document = "Document"
    case file = "File"

    var systemIcon: String {
        switch self {
        case .image:
            return "photo.fill"
        case .video:
            return "play.rectangle.fill"
        case .audio:
            return "waveform"
        case .archive:
            return "archivebox.fill"
        case .document:
            return "doc.text.fill"
        case .file:
            return "doc.fill"
        }
    }
}

private let chatImageExtensions: Set<String> = ["gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif"]
private let chatVideoExtensions: Set<String> = ["avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts"]
private let chatAudioExtensions: Set<String> = ["aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"]
private let chatArchiveExtensions: Set<String> = ["7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip"]
private let chatDocumentExtensions: Set<String> = ["csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml"]

private func chatAttachmentCategory(from filename: String) -> ChatAttachmentCategory {
    let ext = filename
        .split(separator: ".")
        .last
        .map { String($0).lowercased() }

    guard let extensionValue = ext, !extensionValue.isEmpty else {
        return .file
    }

    if chatImageExtensions.contains(extensionValue) {
        return .image
    }
    if chatVideoExtensions.contains(extensionValue) {
        return .video
    }
    if chatAudioExtensions.contains(extensionValue) {
        return .audio
    }
    if chatArchiveExtensions.contains(extensionValue) {
        return .archive
    }
    if chatDocumentExtensions.contains(extensionValue) {
        return .document
    }
    return .file
}

private func chatAttachmentCategory(for attachment: MessageAttachmentSnapshot) -> ChatAttachmentCategory {
    if attachment.isImage {
        return .image
    }
    if attachment.isVideo {
        return .video
    }
    if attachment.isAudio {
        return .audio
    }
    return chatAttachmentCategory(from: attachment.filename)
}

private struct ChatAttachmentView: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, String) -> Void

    @State private var localImageData: Data?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false
    @State private var isOpeningAttachment = false

    private var localImage: PlatformImage? {
        guard let localImageData, !isAnimatedImage(data: localImageData, filename: attachment.filename) else {
            return nil
        }
        return PlatformImage(data: localImageData)
    }

    var body: some View {
        if attachment.isImage {
            Button {
                if let localImageData {
                    onOpenImage(localImageData, attachment.filename)
                } else {
                    Task {
                        await loadImageIfNeeded()
                        if let localImageData {
                            onOpenImage(localImageData, attachment.filename)
                        }
                    }
                }
            } label: {
                VStack(alignment: .leading, spacing: 7) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 16, style: .continuous)
                            .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                            .frame(width: 220, height: 150)
                        if let localImage {
                            Image(platformImage: localImage)
                                .resizable()
                                .scaledToFill()
                                .frame(width: 220, height: 150)
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                        } else if let localImageData, isAnimatedImage(data: localImageData, filename: attachment.filename) {
                            IrisAnimatedImageDataView(data: localImageData)
                                .frame(width: 220, height: 150)
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                                .allowsHitTesting(false)
                        } else if isLoadingImage {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Image(systemName: failedImageLoad ? "exclamationmark.triangle.fill" : "photo.fill")
                                .font(.system(size: 28, weight: .semibold))
                                .opacity(0.72)
                        }
                    }
                    Text(attachment.filename)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .lineLimit(1)
                        .frame(maxWidth: 220, alignment: .leading)
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel(attachment.filename)
            .task(id: attachment.htreeUrl) {
                await loadImageIfNeeded()
            }
        } else {
            let category = chatAttachmentCategory(for: attachment)

            Button {
                Task {
                    guard !isOpeningAttachment else { return }
                    isOpeningAttachment = true
                    await openAttachment(attachment)
                    isOpeningAttachment = false
                }
            } label: {
                HStack(spacing: 8) {
                    if isOpeningAttachment {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 20, height: 20)
                    } else {
                        Image(systemName: category.systemIcon)
                            .font(.system(size: 15, weight: .semibold))
                            .frame(width: 20, height: 20)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(attachment.filename)
                            .font(.system(.subheadline, design: .rounded, weight: .semibold))
                            .lineLimit(1)
                        Text(category.rawValue)
                            .font(.system(.caption, design: .rounded, weight: .medium))
                            .foregroundStyle(isOutgoing ? palette.onBubbleMine.opacity(0.6) : palette.onBubbleTheirs.opacity(0.6))
                            .lineLimit(1)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                )
            }
            .buttonStyle(.plain)
            .disabled(isOpeningAttachment)
            .contextMenu {
                Button("Copy link") {
                    PlatformClipboard.setString(attachment.htreeUrl)
                }
            }
            .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
        }
    }

    @MainActor
    private func loadImageIfNeeded() async {
        guard localImageData == nil, !isLoadingImage else {
            return
        }
        isLoadingImage = true
        failedImageLoad = false
        guard let data = await downloadAttachment(attachment) else {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        if !isAnimatedImage(data: data, filename: attachment.filename), PlatformImage(data: data) == nil {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        localImageData = data
        isLoadingImage = false
    }

}

private struct ImageViewerItem: Identifiable, Equatable {
    let id = UUID()
    let data: Data
    let filename: String

    var isAnimated: Bool {
        isAnimatedImage(data: data, filename: filename)
    }

    var image: PlatformImage? {
        guard !isAnimated else {
            return nil
        }
        return PlatformImage(data: data)
    }

    static func == (lhs: ImageViewerItem, rhs: ImageViewerItem) -> Bool {
        lhs.id == rhs.id
    }
}

private struct IrisImageViewer: View {
    let item: ImageViewerItem
    let onClose: () -> Void

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.opacity(0.92)
                .ignoresSafeArea()
                .onTapGesture(perform: onClose)
            if item.isAnimated {
                IrisAnimatedImageDataView(data: item.data)
                    .padding(22)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .allowsHitTesting(false)
            } else if let image = item.image {
                Image(platformImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(22)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .tint(.white)
            }

            Button(action: onClose) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 30, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.9))
                    .padding(18)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close image")
        }
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .zIndex(10)
    }
}

private func isAnimatedImage(data: Data, filename: String) -> Bool {
    if filename.lowercased().hasSuffix(".gif") {
        return true
    }
    let gifHeader = [UInt8](data.prefix(6))
    return gifHeader == Array("GIF87a".utf8) || gifHeader == Array("GIF89a".utf8)
}

private struct ReplyPreview {
    let author: String
    let body: String
}

private struct ReplyParsedMessage {
    let reply: ReplyPreview?
    let body: String
}

private func replyEncodedMessage(reply: ChatMessageSnapshot?, text: String) -> String {
    guard let reply else {
        return text
    }
    let snippet = replySnippet(for: reply)
    return "\(replyMessagePrefix)\(reply.author): \(snippet)\n\n\(text)"
}

private func parseReplyEncodedMessage(_ text: String) -> ReplyParsedMessage {
    guard text.hasPrefix(replyMessagePrefix) else {
        return ReplyParsedMessage(reply: nil, body: text)
    }
    let remaining = text.dropFirst(replyMessagePrefix.count)
    guard let separator = remaining.range(of: "\n\n") else {
        return ReplyParsedMessage(reply: nil, body: text)
    }
    let header = String(remaining[..<separator.lowerBound])
    let body = String(remaining[separator.upperBound...])
    let pieces = header.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
    guard pieces.count == 2 else {
        return ReplyParsedMessage(reply: nil, body: text)
    }
    return ReplyParsedMessage(
        reply: ReplyPreview(
            author: String(pieces[0]).trimmingCharacters(in: .whitespacesAndNewlines),
            body: String(pieces[1]).trimmingCharacters(in: .whitespacesAndNewlines)
        ),
        body: body
    )
}

private func replySnippet(for message: ChatMessageSnapshot) -> String {
    let parsed = parseReplyEncodedMessage(message.body)
    let source = parsed.body.isEmpty ? copyableMessageText(message) : parsed.body
    let normalized = source
        .replacingOccurrences(of: "\n", with: " ")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if normalized.isEmpty {
        return message.attachments.first?.filename ?? "Attachment"
    }
    return String(normalized.prefix(96))
}

private let replyMessagePrefix = "↩ "

private func linkedMessageAttributedString(_ text: String, linkColor: Color) -> AttributedString {
    var attributed = AttributedString()
    var cursor = text.startIndex
    for match in messageURLMatches(in: text) {
        if cursor < match.range.lowerBound {
            attributed.append(AttributedString(String(text[cursor..<match.range.lowerBound])))
        }
        var linked = AttributedString(String(text[match.range]))
        linked.link = match.url
        linked.foregroundColor = linkColor
        linked.underlineStyle = .single
        attributed.append(linked)
        cursor = match.range.upperBound
    }
    if cursor < text.endIndex {
        attributed.append(AttributedString(String(text[cursor...])))
    }
    return attributed
}

private func messageURLMatches(in text: String) -> [(range: Range<String.Index>, url: URL)] {
    var matches: [(Range<String.Index>, URL)] = []
    let pattern = #"\b((https?://|www\.)[^\s<]+)"#
    guard let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) else {
        return matches
    }
    let nsRange = NSRange(text.startIndex..<text.endIndex, in: text)
    regex.enumerateMatches(in: text, range: nsRange) { result, _, _ in
        guard
            let result,
            let range = Range(result.range(at: 1), in: text)
        else {
            return
        }
        let visible = String(text[range]).trimmingCharacters(in: messageURLTrailingPunctuation)
        guard !visible.isEmpty else {
            return
        }
        let end = text.index(range.lowerBound, offsetBy: visible.count)
        let lowercase = visible.lowercased()
        let normalized = lowercase.hasPrefix("http://") || lowercase.hasPrefix("https://")
            ? visible
            : "https://\(visible)"
        guard let url = URL(string: normalized) else {
            return
        }
        matches.append((range.lowerBound..<end, url))
    }
    return matches
}

private let messageURLTrailingPunctuation = CharacterSet(charactersIn: ".,;:!?)]")

private func copyableMessageText(_ message: ChatMessageSnapshot) -> String {
    var pieces: [String] = []
    if !message.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        pieces.append(message.body)
    }
    pieces.append(contentsOf: message.attachments.map(\.htreeUrl))
    return pieces.joined(separator: "\n")
}

private struct FlowWrap<Content: View>: View {
    let spacing: CGFloat
    let lineSpacing: CGFloat
    let content: () -> Content

    init(
        spacing: CGFloat = 8,
        lineSpacing: CGFloat = 8,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.spacing = spacing
        self.lineSpacing = lineSpacing
        self.content = content
    }

    var body: some View {
        ViewThatFits {
            HStack(alignment: .top, spacing: spacing, content: content)
            VStack(alignment: .leading, spacing: lineSpacing, content: content)
        }
    }
}

private struct SharePayload: Identifiable {
    let id = UUID()
    let text: String
}

#if canImport(UIKit)
private struct ShareSheet: UIViewControllerRepresentable {
    let text: String

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [text], applicationActivities: nil)
    }

    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}
#else
private struct ShareSheet: View {
    let text: String

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Share")
                .font(.system(.title3, design: .rounded, weight: .bold))

            Text("Use the system share panel or copy the payload to the clipboard.")
                .font(.system(.body, design: .rounded))
                .foregroundStyle(.secondary)

            ScrollView {
                Text(text)
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minHeight: 180, maxHeight: 280)
            .padding(12)
            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 18, style: .continuous))

            HStack(spacing: 12) {
                ShareLink(item: text) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(IrisPrimaryButtonStyle())

                Button("Copy") {
                    PlatformClipboard.setString(text)
                }
                .buttonStyle(IrisSecondaryButtonStyle())

                Spacer()

                Button("Close") {
                    dismiss()
                }
                .buttonStyle(IrisSecondaryButtonStyle())
            }
        }
        .padding(20)
        .frame(minWidth: 460)
    }
}
#endif
