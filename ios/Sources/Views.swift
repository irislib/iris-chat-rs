import Foundation
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

private let irisSourceURL = URL(string: "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs")!
private let irisSourceLabel = "Iris Chat source code"
private let irisPrivacyURL = URL(string: "https://chat.iris.to/privacy")!
private let irisTermsURL = URL(string: "https://chat.iris.to/terms")!
private let irisChildSafetyURL = URL(string: "https://chat.iris.to/csae")!
private let irisSupportEmail = "irismessenger@pm.me"
private func irisChatProfileURL(npub: String) -> URL {
    URL(string: "https://chat.iris.to/#/\(npub)")!
}
private let disappearingMessageOptions: [(String, UInt64?)] = [
    ("Off", nil),
    ("5 minutes", 300),
    ("1 hour", 3_600),
    ("24 hours", 86_400),
    ("1 week", 604_800),
    ("1 month", 2_592_000),
    ("3 months", 7_776_000),
]

// Compact label for the chat header subtitle when disappearing-messages is
// on. Tries the canonical menu options first so the wording matches what
// the user picked, then falls back to a generic unit-based string for any
// odd value that arrives over the wire.
private func irisDisappearingLabel(seconds: UInt64) -> String {
    for (label, value) in disappearingMessageOptions {
        if let v = value, v == seconds {
            return label
        }
    }
    if seconds < 3_600 { return "\(seconds / 60) min" }
    if seconds < 86_400 { return "\(seconds / 3_600) h" }
    if seconds < 604_800 { return "\(seconds / 86_400) d" }
    if seconds < 2_592_000 { return "\(seconds / 604_800) wk" }
    return "\(seconds / 2_592_000) mo"
}
private let offlineBannerGraceInterval: TimeInterval = 30

private func hasHttpPicture(_ url: String?) -> Bool {
    guard let trimmed = url?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else {
        return false
    }
    return trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://")
}

private func hasHashtreePicture(_ url: String?) -> Bool {
    guard let trimmed = url?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else {
        return false
    }
    return trimmed.hasPrefix("htree://") || trimmed.hasPrefix("nhash://")
}

private func irisMailtoURL(to email: String, subject: String, body: String) -> URL? {
    var components = URLComponents()
    components.scheme = "mailto"
    components.path = email
    components.queryItems = [
        URLQueryItem(name: "subject", value: subject),
        URLQueryItem(name: "body", value: body),
    ]
    return components.url
}

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

enum SettingsFocusSection: Hashable {
    case messageServers
    case messaging
}

#if os(iOS)
/// Posted with a `SettingsFocusSection` in `userInfo["focus"]` (or
/// `nil`) when a deep child wants the settings sheet opened on a
/// specific page. `IrisRoot` listens and flips its `@State`.
let irisOpenSettingsNotification = Notification.Name("to.iris.chat.open-settings")
#endif

private enum SettingsPage: String, CaseIterable, Identifiable {
    case profile
    case devices
    case messaging
    case notifications
    case media
    case nearby
    case messageServers
    case security
    case updates
    case about
    case legal
    case support
    case accountData

    var id: String { rawValue }

    var title: String {
        switch self {
        case .profile: return "Profile"
        case .devices: return "Devices"
        case .messaging: return "Messaging"
        case .notifications: return "Notifications"
        case .media: return "Media"
        case .nearby: return "Nearby"
        case .messageServers: return "Message servers"
        case .security: return "Security"
        case .updates: return "Updates"
        case .about: return "About"
        case .legal: return "Legal"
        case .support: return "Support"
        case .accountData: return "Account data"
        }
    }

    var systemImage: String {
        switch self {
        case .profile: return "person.crop.circle.fill"
        case .devices: return "laptopcomputer.and.iphone"
        case .messaging: return "bubble.left.and.bubble.right.fill"
        case .notifications: return "bell.fill"
        case .media: return "photo.fill"
        case .nearby: return "dot.radiowaves.left.and.right"
        case .messageServers: return "server.rack"
        case .security: return "lock.fill"
        case .updates: return "arrow.down.circle.fill"
        case .about: return "info.circle.fill"
        case .legal: return "doc.text.fill"
        case .support: return "wrench.and.screwdriver.fill"
        case .accountData: return "trash.fill"
        }
    }

    var accessibilityID: String {
        switch self {
        case .profile: return "settingsProfileRow"
        case .devices: return "settingsDevicesRow"
        case .messaging: return "settingsMessagingRow"
        case .notifications: return "settingsNotificationsRow"
        case .media: return "settingsMediaRow"
        case .nearby: return "settingsNearbyRow"
        case .messageServers: return "settingsMessageServersRow"
        case .security: return "settingsSecurityRow"
        case .updates: return "settingsUpdatesRow"
        case .about: return "settingsAboutRow"
        case .legal: return "settingsLegalRow"
        case .support: return "settingsSupportRow"
        case .accountData: return "settingsAccountDataRow"
        }
    }

    static var menuPages: [SettingsPage] {
        var pages: [SettingsPage] = [
            .devices,
            .messaging,
            .notifications,
            .media,
            .nearby,
            .messageServers,
            .security,
        ]
        #if os(macOS)
        pages.append(.updates)
        #endif
        pages.append(contentsOf: [.about, .legal, .support, .accountData])
        return pages
    }
}

struct RootView: View {
    @ObservedObject var manager: AppManager
    @State private var directChatInfoChatId: String?
    @State private var settingsFocus: SettingsFocusSection?
#if os(iOS)
    @State private var showingSettingsSheet = false
#endif
#if os(iOS) || os(macOS)
    @State private var showingNearbyIris = false
#endif

    var body: some View {
        IrisTheme {
            ZStack(alignment: .top) {
                BackgroundFill()

                if manager.bootstrapInFlight {
                    LoadingOverlay()
                } else if usesDesktopChatShell {
                    VStack(spacing: 0) {
#if os(macOS)
                        if manager.updates.available {
                            DesktopUpdateStripe(updates: manager.updates)
                        }
#endif
                        DesktopChatShell(
                            manager: manager,
                            directChatInfoChatId: $directChatInfoChatId,
                            onOpenNearby: openNearbyIris
                        )
                    }
                } else if let directChatInfoChatId {
                    NavigationShell(
                        title: "Details",
                        canGoBack: true,
                        onBack: { self.directChatInfoChatId = nil },
                        leading: AnyView(EmptyView()),
                        trailing: AnyView(EmptyView()),
                        offlineBanner: offlineBanner
                    ) {
                        DirectChatInfoScreen(
                            manager: manager,
                            chatId: directChatInfoChatId,
                            onClose: { self.directChatInfoChatId = nil }
                        )
                    }
                } else {
                    mobileNavigationContent
                }

                ToastOverlay(center: manager.toasts)
            }
#if os(iOS)
            .sheet(isPresented: $showingSettingsSheet) {
                SettingsScreen(
                    manager: manager,
                    focusedSection: $settingsFocus,
                    modalClose: { showingSettingsSheet = false }
                )
                .irisModalSurface()
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
            }
            .irisOnChange(of: manager.state.account?.publicKeyHex) { accountId in
                if accountId == nil {
                    showingSettingsSheet = false
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: irisOpenSettingsNotification)) { note in
                if let focus = note.userInfo?["focus"] as? SettingsFocusSection {
                    settingsFocus = focus
                }
                showingSettingsSheet = true
            }
#endif
#if os(iOS) || os(macOS)
            .sheet(isPresented: $showingNearbyIris) {
                NearbyIrisScreen(
                    manager: manager,
                    service: manager.nearbyIris,
                    onClose: { showingNearbyIris = false }
                )
                .irisModalSurface()
#if os(macOS)
                .frame(minWidth: 420, minHeight: 520)
#endif
                .irisDismissOnMacOutsideClick { showingNearbyIris = false }
            }
#endif
#if os(iOS) || os(macOS)
            .sheet(
                item: Binding(
                    get: { manager.pendingShare },
                    set: { value in
                        if value == nil {
                            manager.clearPendingShare()
                        }
                    }
                )
            ) { share in
#if os(iOS)
                ShareTargetSheet(manager: manager, share: share)
                    .irisModalSurface()
                    .presentationDetents([.large])
                    .presentationDragIndicator(.visible)
#elseif os(macOS)
                ShareTargetSheet(manager: manager, share: share)
                    .frame(minWidth: 380, minHeight: 420)
#endif
            }
            .onAppear {
                // Constructing the CBCentralManager triggers the iOS
                // Bluetooth permission alert on first use, so only
                // start the power-state monitor when the user has
                // previously opted into Nearby Bluetooth. The Nearby
                // settings page kicks off its own monitor in onAppear
                // (Views.swift NearbySettingsRows) when the user opens
                // it for the first time.
                if manager.state.preferences.nearbyBluetoothEnabled {
                    manager.nearbyIris.startBluetoothStateMonitoring()
                }
            }
#endif
        }
    }

    @ViewBuilder
    private var mobileNavigationContent: some View {
#if os(iOS)
        UIKitRouteNavigationHost(
            routes: currentNavigationRoutes,
            makeContent: { route in
                AnyView(screenChrome(for: route))
            },
            onStackChanged: { stack in
                manager.dispatch(.updateScreenStack(stack: stack))
            }
        )
#else
        if case .welcome = manager.activeScreen {
            WelcomeScreen(manager: manager)
        } else {
            screenChrome(for: currentNavigationRoute)
        }
#endif
    }

    @ViewBuilder
    private func screenChrome(for route: NavigationRoute) -> some View {
        let screen = route.screen
        if case .welcome = screen {
            WelcomeScreen(manager: manager)
        } else {
            NavigationShell(
                title: screenTitle(screen),
                subtitle: chatHeaderSubtitle(for: screen),
                subtitleSystemImage: chatHeaderSubtitleSystemImage(for: screen),
                isChatHeader: usesCompactTopBar(screen),
                centerTitle: usesCenteredTopBarTitle(screen),
                floatsHeader: usesFloatingTopBar(screen),
                canGoBack: route.depth > 0,
                onBack: manager.navigateBack,
                backBadgeCount: backUnreadCount(for: screen),
                leading: topBarLeadingItem(for: screen),
                trailing: topBarTrailingItem(for: screen),
                titleAccessoryLeading: chatHeaderTitleAvatar(for: screen),
                onTitleTap: chatHeaderOnTap(for: screen),
                offlineBanner: offlineBanner
            ) {
                content(for: screen)
            }
        }
    }

    private var usesDesktopChatShell: Bool {
        guard IrisLayout.usesDesktopChrome, manager.state.account != nil else {
            return false
        }
        switch manager.activeScreen {
        case .welcome, .createAccount, .restoreAccount, .addDevice, .awaitingDeviceApproval, .deviceRevoked:
            return false
        case .chatList, .newChat, .newGroup, .createInvite, .joinInvite, .settings, .chat, .groupDetails, .deviceRoster:
            return true
        }
    }

    private func usesCompactTopBar(_ screen: Screen) -> Bool {
        if case .chat = screen { return true }
        if case .chatList = screen { return true }
        return false
    }

    private func usesCenteredTopBarTitle(_ screen: Screen) -> Bool {
        if case .chatList = screen { return true }
        return false
    }

    private func usesFloatingTopBar(_ screen: Screen) -> Bool {
        if case .chat = screen { return true }
        if case .chatList = screen { return true }
        return false
    }

    private var currentNavigationRoutes: [NavigationRoute] {
        let root = NavigationRoute(screen: manager.state.router.defaultScreen, depth: 0)
        let stack = manager.state.router.screenStack.enumerated().map { index, screen in
            NavigationRoute(screen: screen, depth: index + 1)
        }
        return [root] + stack
    }

    private var currentNavigationRoute: NavigationRoute {
        NavigationRoute(
            screen: manager.activeScreen,
            depth: manager.state.router.screenStack.count
        )
    }

    @ViewBuilder
    private func content(for screen: Screen) -> some View {
        switch screen {
        case .welcome:
            WelcomeScreen(manager: manager)
        case .createAccount:
            CreateAccountScreen(manager: manager)
        case .restoreAccount:
            RestoreAccountScreen(manager: manager)
        case .addDevice:
            AddDeviceScreen(manager: manager, awaitingApproval: false)
        case .chatList:
            ChatListScreen(manager: manager, onOpenNearby: openNearbyIris)
        case .newChat:
            NewChatScreen(manager: manager)
        case .newGroup:
            NewGroupScreen(manager: manager)
        case .createInvite:
            CreateInviteScreen(manager: manager)
        case .joinInvite:
            JoinInviteScreen(manager: manager)
        case .settings:
            SettingsScreen(manager: manager, focusedSection: $settingsFocus)
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

    private func topBarLeadingItem(for screen: Screen) -> AnyView {
        guard case .chatList = screen, let account = manager.state.account else {
            return AnyView(EmptyView())
        }

        return AnyView(
            Button(action: { openSettings() }) {
                IrisAvatar(
                    label: account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName,
                    size: 40,
                    emphasize: true,
                    pictureUrl: account.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager,
                    loadedImageIdentifier: "chatListProfileAvatarImage"
                )
            }
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("chatListProfileButton")
        )
    }

    private func topBarTrailingItem(for screen: Screen) -> AnyView {
        if case .chatList = screen {
            return AnyView(
                NewChatCircleButton {
                    manager.dispatch(.pushScreen(screen: .newChat))
                }
            )
        }

        // Surface "Search in this chat" on the chat / group-details
        // pages. Tapping pops up an inline scoped-search sheet so the
        // user doesn't have to navigate back to the chat list and
        // retype the chat name.
        if let target = chatHeaderSearchTarget(for: screen) {
            return AnyView(
                InChatSearchButton(manager: manager, target: target)
            )
        }

        // The chat header avatar/title is the entry point to chat info now —
        // no overflow menu needed.
        return AnyView(EmptyView())
    }

    private func chatHeaderSearchTarget(for screen: Screen) -> InChatSearchTarget? {
        switch screen {
        case .chat:
            guard let chat = manager.state.currentChat else { return nil }
            return InChatSearchTarget(chatId: chat.chatId, displayName: chat.displayName)
        case .groupDetails(let groupId):
            let chatId = "group:\(groupId)"
            let name = manager.state.groupDetails?.name ?? "Group"
            return InChatSearchTarget(chatId: chatId, displayName: name)
        default:
            return nil
        }
    }

    private func backUnreadCount(for screen: Screen) -> UInt64 {
        guard case .chat(let activeChatId) = screen else {
            return 0
        }
        return manager.state.chatList
            .filter { $0.chatId != activeChatId }
            .reduce(UInt64(0)) { $0 + $1.unreadCount }
    }

    private func chatHeaderTitleAvatar(for screen: Screen) -> AnyView {
        guard case .chat = screen, let chat = manager.state.currentChat else {
            return AnyView(EmptyView())
        }
        return AnyView(
            IrisAvatar(
                label: chat.displayName,
                size: 40,
                pictureUrl: chat.pictureUrl,
                preferences: manager.state.preferences,
                manager: manager
            )
        )
    }

    private func chatHeaderOnTap(for screen: Screen) -> (() -> Void)? {
        guard case .chat = screen, let chat = manager.state.currentChat else {
            return nil
        }
        if let groupId = chat.groupId {
            return { [weak manager] in
                manager?.dispatch(.pushScreen(screen: .groupDetails(groupId: groupId)))
            }
        }
        // Direct chat — open the inline info sheet.
        let chatId = chat.chatId
        return {
            directChatInfoChatId = chatId
        }
    }

    private func chatHeaderSubtitle(for screen: Screen) -> String? {
        guard case .chat = screen, let chat = manager.state.currentChat else {
            return nil
        }
        if let ttl = chat.messageTtlSeconds, ttl > 0 {
            return irisDisappearingLabel(seconds: ttl)
        }
        if chat.isMuted {
            return "muted"
        }
        return nil
    }

    private func chatHeaderSubtitleSystemImage(for screen: Screen) -> String? {
        guard case .chat = screen, let chat = manager.state.currentChat else {
            return nil
        }
        if let ttl = chat.messageTtlSeconds, ttl > 0 {
            return "timer"
        }
        if chat.isMuted {
            return "bell.slash.fill"
        }
        return nil
    }

    private var offlineBanner: AnyView {
#if os(iOS)
        AnyView(
            OfflineStatusBanner(
                networkStatus: manager.state.networkStatus,
                nearbyService: manager.nearbyIris,
                appSceneIsActive: manager.appSceneIsActive,
                foregroundedAt: manager.lastForegroundedAt,
                onTap: {
                    openSettings(focusedSection: .messageServers)
                }
            )
        )
#else
        AnyView(EmptyView())
#endif
    }

    private func screenTitle(_ screen: Screen) -> String {
        switch screen {
        case .welcome: return "Welcome"
        case .createAccount: return "Create Profile"
        case .restoreAccount: return "Restore Profile"
        case .addDevice: return "Link Device"
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
            return "Device Removed"
        }
    }

    private func openSettings(focusedSection: SettingsFocusSection? = nil) {
        if let focusedSection {
            settingsFocus = focusedSection
        }
#if os(iOS)
        showingSettingsSheet = true
#else
        manager.dispatch(.pushScreen(screen: .settings))
#endif
    }

    private func openNearbyIris() {
#if os(iOS) || os(macOS)
        manager.prepareNearbyForUserTap()
        showingNearbyIris = true
#endif
    }
}

#if os(iOS) || os(macOS)
private struct ShareTargetSheet: View {
    @ObservedObject var manager: AppManager
    let share: PendingShare
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.irisPalette) private var palette
    @State private var searchText = ""
    @State private var selectedChatIds = Set<String>()

    private var filteredChats: [ChatThreadSnapshot] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return manager.state.chatList
        }
        return manager.state.chatList.filter { chat in
            chat.displayName.lowercased().contains(query)
                || (chat.subtitle?.lowercased().contains(query) ?? false)
                || (chat.lastMessagePreview?.lowercased().contains(query) ?? false)
        }
    }

    private var selectedChats: [ChatThreadSnapshot] {
        manager.state.chatList.filter { selectedChatIds.contains($0.chatId) }
    }

    private var selectedNamesText: String {
        selectedChats.map(\.displayName).joined(separator: ", ")
    }

    var body: some View {
        NavigationStack {
            Group {
                if manager.state.chatList.isEmpty {
                    emptyState
                } else {
                    chatList
                }
            }
            .navigationTitle(share.isForwarding ? "Forward" : "Choose recipients")
#if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search")
#elseif os(macOS)
            .searchable(text: $searchText, prompt: "Search")
#endif
#if os(iOS)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if !manager.state.chatList.isEmpty {
                    approvalFooter
                }
            }
#endif
            .toolbar {
#if os(iOS)
                ToolbarItem(placement: .confirmationAction) {
                    IrisModalCloseButton(accessibilityIdentifier: "shareTargetCloseButton") {
                        manager.clearPendingShare()
                        dismiss()
                    }
                }
#elseif os(macOS)
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        manager.clearPendingShare()
                        dismiss()
                    }
                }
                if !manager.state.chatList.isEmpty {
                    ToolbarItem(placement: .confirmationAction) {
                        Button {
                            sendSelectedAndDismiss()
                        } label: {
                            Text(sendButtonTitle)
                        }
                        .disabled(selectedChatIds.isEmpty)
                    }
                }
#endif
            }
            .background(palette.background)
            .onAppear(perform: preselectSuggestedChat)
            .irisOnChange(of: share.id) { _ in
                selectedChatIds.removeAll()
                preselectSuggestedChat()
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 18) {
            Text("Start a chat first")
                .font(.headline)
                .foregroundStyle(palette.textPrimary)
            Button("New chat") {
                manager.clearPendingShare()
                manager.dispatch(.pushScreen(screen: .newChat))
                dismiss()
            }
            .buttonStyle(.borderedProminent)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(palette.background)
    }

    private var chatList: some View {
        List {
            if filteredChats.isEmpty {
                Text("No matches")
                    .font(.system(size: 17, weight: .regular))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity, minHeight: 180, alignment: .center)
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(palette.background)
                    .listRowSeparator(.hidden)
            } else {
                Section {
                    ForEach(filteredChats, id: \.chatId) { chat in
                        shareTargetRow(chat)
                    }
                } header: {
                    Text(searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Recent chats" : "Chats")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .textCase(nil)
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(palette.background)
    }

    private var sendButtonTitle: String {
        selectedChatIds.count > 1 ? "Send (\(selectedChatIds.count))" : "Send"
    }

#if os(iOS)
    private var approvalFooter: some View {
        VStack(spacing: 0) {
            Rectangle()
                .fill(palette.border)
                .frame(height: 0.5)

            HStack(spacing: 12) {
                ScrollView(.horizontal, showsIndicators: false) {
                    Text(selectedNamesText.isEmpty ? " " : selectedNamesText)
                        .font(.system(size: 15, weight: .regular))
                        .foregroundStyle(selectedChatIds.isEmpty ? palette.muted : palette.textPrimary)
                        .lineLimit(1)
                        .fixedSize(horizontal: true, vertical: false)
                        .padding(.horizontal, 2)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityHidden(selectedChatIds.isEmpty)

                Button {
                    sendSelectedAndDismiss()
                } label: {
                    Image(systemName: "arrow.up")
                        .font(.system(size: 21, weight: .bold))
                        .foregroundStyle(selectedChatIds.isEmpty ? palette.textPrimary.opacity(0.42) : Color.white)
                        .frame(width: 48, height: 48)
                        .background(
                            Circle()
                                .fill(selectedChatIds.isEmpty ? palette.muted.opacity(0.24) : palette.action)
                        )
                        .contentShape(Circle())
                }
                .buttonStyle(.irisPlain)
                .disabled(selectedChatIds.isEmpty)
                .accessibilityLabel(sendButtonTitle)
                .accessibilityIdentifier("shareTargetSendButton")
            }
            .padding(.horizontal, 16)
            .padding(.top, 9)
            .padding(.bottom, 9)
        }
        .background(shareFooterBackground)
    }

    private var shareFooterBackground: Color {
        if colorScheme == .dark {
            return Color(.sRGB, red: 27.0 / 255.0, green: 27.0 / 255.0, blue: 27.0 / 255.0, opacity: 1)
        }
        return Color(.sRGB, red: 246.0 / 255.0, green: 246.0 / 255.0, blue: 246.0 / 255.0, opacity: 1)
    }
#endif

    private func sendSelectedAndDismiss() {
        manager.sendPendingShare(
            to: manager.state.chatList
                .map(\.chatId)
                .filter { selectedChatIds.contains($0) }
        )
        dismiss()
    }

    private func shareTargetRow(_ chat: ChatThreadSnapshot) -> some View {
        let selected = selectedChatIds.contains(chat.chatId)
        return Button {
            if selected {
                selectedChatIds.remove(chat.chatId)
            } else {
                selectedChatIds.insert(chat.chatId)
            }
        } label: {
            HStack(spacing: 12) {
                IrisAvatar(
                    label: chat.displayName,
                    size: 40,
                    emphasize: false,
                    pictureUrl: chat.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager
                )
                VStack(alignment: .leading, spacing: 2) {
                    Text(chat.displayName)
                        .font(.system(size: 17, weight: .regular))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    if let subtitle = chat.subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.system(size: 13, weight: .regular))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Spacer()
                ShareTargetSelectionBadge(isSelected: selected)
            }
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
        .listRowBackground(palette.panel)
        .listRowSeparator(.hidden)
        .accessibilityLabel("\(chat.displayName), \(selected ? "selected" : "not selected")")
        .accessibilityValue(selected ? "Selected" : "Not selected")
    }

    private func preselectSuggestedChat() {
        guard selectedChatIds.isEmpty else {
            return
        }
        let suggested = Set(share.suggestedTargetChatIds)
        let available = manager.state.chatList
            .map(\.chatId)
            .filter { suggested.contains($0) }
        selectedChatIds.formUnion(available)
    }
}

private struct ShareTargetSelectionBadge: View {
    @Environment(\.irisPalette) private var palette
    let isSelected: Bool

    var body: some View {
        Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
            .font(.system(size: 24, weight: .semibold))
            .symbolRenderingMode(.monochrome)
            .foregroundStyle(isSelected ? palette.action : palette.muted.opacity(0.55))
            .frame(width: 32, height: 40, alignment: .trailing)
            .accessibilityHidden(true)
    }
}
#endif

struct NavigationShell<Content: View>: View {
    let title: String
    let subtitle: String?
    let subtitleSystemImage: String?
    let isChatHeader: Bool
    let centerTitle: Bool
    let floatsHeader: Bool
    let canGoBack: Bool
    let onBack: () -> Void
    let backBadgeCount: UInt64
    let leading: AnyView
    let trailing: AnyView
    let titleAccessoryLeading: AnyView
    let onTitleTap: (() -> Void)?
    let offlineBanner: AnyView
    let content: () -> Content

    init(
        title: String,
        subtitle: String? = nil,
        subtitleSystemImage: String? = nil,
        isChatHeader: Bool = false,
        centerTitle: Bool = false,
        floatsHeader: Bool = false,
        canGoBack: Bool,
        onBack: @escaping () -> Void,
        backBadgeCount: UInt64 = 0,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView()),
        titleAccessoryLeading: AnyView = AnyView(EmptyView()),
        onTitleTap: (() -> Void)? = nil,
        offlineBanner: AnyView = AnyView(EmptyView()),
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.subtitle = subtitle
        self.subtitleSystemImage = subtitleSystemImage
        self.isChatHeader = isChatHeader
        self.centerTitle = centerTitle
        self.floatsHeader = floatsHeader
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.backBadgeCount = backBadgeCount
        self.leading = leading
        self.trailing = trailing
        self.titleAccessoryLeading = titleAccessoryLeading
        self.onTitleTap = onTitleTap
        self.offlineBanner = offlineBanner
        self.content = content
    }

    @Environment(\.irisPalette) private var palette

    @ViewBuilder
    var body: some View {
        if floatsHeader {
            floatingHeaderBody
        } else {
            insetHeaderBody
        }
    }

    private var insetHeaderBody: some View {
        screenContent
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .safeAreaInset(edge: .top, spacing: 0) {
                navigationHeader
                // Signal-style no-divider header: the title cluster
                // floats on a soft background fade that dissolves
                // into the scrolling chat/list content.
                .background(alignment: .top) {
                    IrisNavigationHeaderChrome(palette: palette)
                        .ignoresSafeArea(.all, edges: .top)
                }
            }
    }

    private var floatingHeaderBody: some View {
        GeometryReader { geometry in
            let topSafeArea = geometry.safeAreaInsets.top
            let contentTopInset = IrisNavigationHeaderMetrics.contentTopInset(
                topSafeArea: topSafeArea,
                isChatHeader: isChatHeader
            )
            let chromeHeight = IrisNavigationHeaderMetrics.chromeHeight(
                topSafeArea: topSafeArea,
                isChatHeader: isChatHeader
            )

            screenContent
                .environment(\.irisNavigationHeaderTopInset, contentTopInset)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .overlay(alignment: .top) {
                    navigationHeader
                        .padding(.top, topSafeArea)
                        .background(alignment: .top) {
                            IrisNavigationHeaderChrome(palette: palette, height: chromeHeight)
                                .ignoresSafeArea(.all, edges: .top)
                        }
                        .zIndex(20)
                }
        }
    }

    private var screenContent: some View {
        content()
    }

    private var navigationHeader: some View {
        VStack(spacing: 0) {
            IrisTopBar(
                title: title,
                subtitle: subtitle,
                subtitleSystemImage: subtitleSystemImage,
                isChatHeader: isChatHeader,
                centerTitle: centerTitle,
                canGoBack: canGoBack,
                onBack: onBack,
                backBadgeCount: backBadgeCount,
                leading: leading,
                trailing: trailing,
                titleAccessoryLeading: titleAccessoryLeading,
                onTitleTap: onTitleTap
            )
            offlineBanner
        }
    }
}

private struct NavigationRoute: Equatable {
    let screen: Screen
    let depth: Int

    var identity: String {
        "\(key)|\(depth)"
    }

    var key: String {
        switch screen {
        case .welcome:
            return "welcome"
        case .createAccount:
            return "createAccount"
        case .restoreAccount:
            return "restoreAccount"
        case .addDevice:
            return "addDevice"
        case .chatList:
            return "chatList"
        case .newChat:
            return "newChat"
        case .newGroup:
            return "newGroup"
        case .createInvite:
            return "createInvite"
        case .joinInvite:
            return "joinInvite"
        case .settings:
            return "settings"
        case .chat(let chatId):
            return "chat:\(chatId)"
        case .groupDetails(let groupId):
            return "groupDetails:\(groupId)"
        case .deviceRoster:
            return "deviceRoster"
        case .awaitingDeviceApproval:
            return "awaitingDeviceApproval"
        case .deviceRevoked:
            return "deviceRevoked"
        }
    }

}

#if os(iOS)
private struct UIKitRouteNavigationHost: UIViewControllerRepresentable {
    let routes: [NavigationRoute]
    let makeContent: (NavigationRoute) -> AnyView
    let onStackChanged: ([Screen]) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onStackChanged: onStackChanged)
    }

    func makeUIViewController(context: Context) -> UINavigationController {
        let navigationController = UINavigationController()
        configureNavigationController(navigationController)
        navigationController.setNavigationBarHidden(true, animated: false)
        navigationController.delegate = context.coordinator
        navigationController.interactivePopGestureRecognizer?.delegate = context.coordinator
        navigationController.interactivePopGestureRecognizer?.isEnabled = true
        context.coordinator.navigationController = navigationController
        return navigationController
    }

    func updateUIViewController(_ navigationController: UINavigationController, context: Context) {
        configureNavigationController(navigationController)
        navigationController.interactivePopGestureRecognizer?.delegate = context.coordinator
        navigationController.interactivePopGestureRecognizer?.isEnabled = true
        context.coordinator.update(
            navigationController: navigationController,
            routes: routes,
            makeContent: makeContent
        )
    }

    private func configureNavigationController(_ navigationController: UINavigationController) {
        navigationController.view.backgroundColor = .clear
        navigationController.navigationBar.prefersLargeTitles = false

        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.shadowColor = .clear
        navigationController.navigationBar.standardAppearance = appearance
        navigationController.navigationBar.scrollEdgeAppearance = appearance
        navigationController.navigationBar.compactAppearance = appearance
        if #available(iOS 15.0, *) {
            navigationController.navigationBar.compactScrollEdgeAppearance = appearance
        }
    }

    final class Coordinator: NSObject, UINavigationControllerDelegate, UIGestureRecognizerDelegate {
        private let onStackChanged: ([Screen]) -> Void
        private var currentRoutes: [NavigationRoute] = []
        private var applyingProgrammaticNavigation = false
        private var deferredUpdate: (routes: [NavigationRoute], makeContent: (NavigationRoute) -> AnyView)?
        weak var navigationController: UINavigationController?

        init(onStackChanged: @escaping ([Screen]) -> Void) {
            self.onStackChanged = onStackChanged
        }

        func update(
            navigationController: UINavigationController,
            routes: [NavigationRoute],
            makeContent: @escaping (NavigationRoute) -> AnyView
        ) {
            if isInteractivePopActive(in: navigationController)
                || isNavigationTransitionActive(in: navigationController) {
                deferredUpdate = (routes, makeContent)
                return
            }

            let existingControllers = routeControllers(in: navigationController)
            if existingControllers.map(\.route) == routes {
                refresh(controllers: existingControllers, makeContent: makeContent)
                currentRoutes = routes
                return
            }

            let nextControllers = routes.map { route in
                if let existing = existingControllers.first(where: { $0.route.identity == route.identity }) {
                    existing.route = route
                    existing.rootView = makeContent(route)
                    return existing
                }
                return RouteHostingController(route: route, rootView: makeContent(route))
            }

            let oldRoutes = existingControllers.map(\.route)
            let animated = shouldAnimate(from: oldRoutes, to: routes)
            applyingProgrammaticNavigation = animated
            currentRoutes = routes
            navigationController.setViewControllers(nextControllers, animated: animated)
            if !animated {
                applyingProgrammaticNavigation = false
            }
        }

        func navigationController(
            _ navigationController: UINavigationController,
            didShow viewController: UIViewController,
            animated: Bool
        ) {
            let visibleRoutes = routeControllers(in: navigationController).map(\.route)
            if applyingProgrammaticNavigation {
                applyingProgrammaticNavigation = false
                currentRoutes = visibleRoutes
                applyDeferredUpdateIfNeeded(navigationController: navigationController, visibleRoutes: visibleRoutes)
                return
            }
            guard visibleRoutes != currentRoutes else {
                applyDeferredUpdateIfNeeded(navigationController: navigationController, visibleRoutes: visibleRoutes)
                return
            }
            currentRoutes = visibleRoutes
            deferredUpdate = nil
            onStackChanged(visibleRoutes.dropFirst().map(\.screen))
        }

        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard gestureRecognizer === navigationController?.interactivePopGestureRecognizer else {
                return true
            }
            guard navigationController?.transitionCoordinator == nil else {
                return false
            }
            guard (navigationController?.viewControllers.count ?? currentRoutes.count) > 1 else {
                return false
            }
            let velocity = (gestureRecognizer as? UIPanGestureRecognizer)?.velocity(in: navigationController?.view)
            return (velocity?.x ?? 1) > 0
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            gestureRecognizer === navigationController?.interactivePopGestureRecognizer
                || otherGestureRecognizer === navigationController?.interactivePopGestureRecognizer
        }

        private func refresh(
            controllers: [RouteHostingController],
            makeContent: (NavigationRoute) -> AnyView
        ) {
            controllers.forEach { controller in
                controller.rootView = makeContent(controller.route)
            }
        }

        private func routeControllers(in navigationController: UINavigationController) -> [RouteHostingController] {
            navigationController.viewControllers.compactMap { $0 as? RouteHostingController }
        }

        private func isInteractivePopActive(in navigationController: UINavigationController) -> Bool {
            if navigationController.transitionCoordinator?.isInteractive == true {
                return true
            }
            switch navigationController.interactivePopGestureRecognizer?.state {
            case .began, .changed:
                return true
            default:
                return false
            }
        }

        private func isNavigationTransitionActive(in navigationController: UINavigationController) -> Bool {
            applyingProgrammaticNavigation || navigationController.transitionCoordinator != nil
        }

        private func applyDeferredUpdateIfNeeded(
            navigationController: UINavigationController,
            visibleRoutes: [NavigationRoute]
        ) {
            guard let deferredUpdate else { return }
            self.deferredUpdate = nil
            DispatchQueue.main.async { [weak self, weak navigationController] in
                guard let self, let navigationController else { return }
                guard self.routeControllers(in: navigationController).map(\.route) == visibleRoutes else {
                    return
                }
                self.update(
                    navigationController: navigationController,
                    routes: deferredUpdate.routes,
                    makeContent: deferredUpdate.makeContent
                )
            }
        }

        private func shouldAnimate(from oldRoutes: [NavigationRoute], to newRoutes: [NavigationRoute]) -> Bool {
            guard !oldRoutes.isEmpty, oldRoutes.first == newRoutes.first else {
                return false
            }
            if newRoutes.count == oldRoutes.count + 1 {
                return oldRoutes == Array(newRoutes.dropLast())
            }
            if newRoutes.count + 1 == oldRoutes.count {
                return newRoutes == Array(oldRoutes.dropLast())
            }
            return false
        }
    }
}

private final class RouteHostingController: UIHostingController<AnyView> {
    var route: NavigationRoute

    init(route: NavigationRoute, rootView: AnyView) {
        self.route = route
        super.init(rootView: rootView)
        view.backgroundColor = .clear
        navigationItem.backButtonDisplayMode = .minimal
    }

    @MainActor
    @preconcurrency required dynamic init?(coder aDecoder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}
#endif

#if os(iOS)
private struct OfflineStatusBanner: View {
    @Environment(\.irisPalette) private var palette

    let networkStatus: NetworkStatusSnapshot?
    @ObservedObject var nearbyService: IrisNearbyService
    let appSceneIsActive: Bool
    let foregroundedAt: Date
    let onTap: () -> Void
    @State private var now = Date()

    var body: some View {
        let text = bannerText(at: now)
        Button(action: onTap) {
            VStack(spacing: 0) {
                if let text {
                    // Glass capsule with a small accentAlt offline
                    // icon — the previous full-width orange bar
                    // screamed at the user every time a relay
                    // blipped. Carrying the warning in the icon
                    // alone keeps the banner readable without
                    // dominating the screen.
                    HStack(spacing: 6) {
                        Image(systemName: "wifi.slash")
                            .font(.system(size: 11, weight: .bold))
                            .foregroundStyle(palette.accentAlt)
                        Text(text)
                            .font(.system(.caption, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .irisGlassSurface(in: Capsule())
                    .overlay(
                        Capsule()
                            .strokeBorder(palette.border, lineWidth: 0.5)
                    )
                    .padding(.horizontal, 12)
                    .padding(.bottom, 4)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .accessibilityIdentifier("offlineStatusBanner")
                }
            }
            .clipped()
            .animation(.easeInOut(duration: 0.22), value: text)
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Open settings")
        .task(id: refreshToken) {
            await refreshBannerClockIfNeeded()
        }
    }

    private func bannerText(at date: Date) -> String? {
        guard appSceneIsActive,
              let status = networkStatus,
              !status.relayUrls.isEmpty,
              status.connectedRelayCount == 0,
              let offlineSince = status.allRelaysOfflineSinceSecs,
              date.timeIntervalSince1970 - TimeInterval(offlineSince) >= offlineBannerGraceInterval,
              date.timeIntervalSince(foregroundedAt) >= offlineBannerGraceInterval else {
            return nil
        }
        let bluetoothStatus = nearbyService.isBluetoothOn ? "on" : "off"
        let wifiStatus = mobileWifiEnabled(nearbyService) ? "on" : "off"
        return "Offline, Bluetooth \(bluetoothStatus), Wi-Fi \(wifiStatus)"
    }

    private var refreshToken: String {
        [
            appSceneIsActive ? "active" : "inactive",
            String(networkStatus?.connectedRelayCount ?? 0),
            String(networkStatus?.allRelaysOfflineSinceSecs ?? 0),
            String(foregroundedAt.timeIntervalSince1970),
            nearbyService.isBluetoothOn ? "bt-on" : "bt-off",
            mobileWifiEnabled(nearbyService) ? "wifi-on" : "wifi-off",
        ].joined(separator: "|")
    }

    private func nextRefreshDate(at date: Date) -> Date? {
        guard appSceneIsActive,
              let status = networkStatus,
              !status.relayUrls.isEmpty,
              status.connectedRelayCount == 0,
              let offlineSince = status.allRelaysOfflineSinceSecs else {
            return nil
        }
        let offlineDeadline = Date(
            timeIntervalSince1970: TimeInterval(offlineSince) + offlineBannerGraceInterval
        )
        let foregroundDeadline = foregroundedAt.addingTimeInterval(offlineBannerGraceInterval)
        let deadline = max(offlineDeadline, foregroundDeadline)
        return date < deadline ? deadline : nil
    }

    private func refreshBannerClockIfNeeded() async {
        let current = Date()
        await MainActor.run {
            now = current
        }
        guard let deadline = nextRefreshDate(at: current) else {
            return
        }
        let seconds = max(0, deadline.timeIntervalSince(current))
        try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        guard !Task.isCancelled else {
            return
        }
        await MainActor.run {
            now = Date()
        }
    }
}

#endif

#if os(iOS) || os(macOS)
private func mobileWifiEnabled(_ service: IrisNearbyService) -> Bool {
    service.isLanVisible && !mobileWifiBlockingStatuses.contains(service.lanStatus)
}

private let mobileWifiBlockingStatuses: Set<String> = [
    "No local network access",
    "Local network unavailable",
    "Local network failed"
]
#endif

private struct DesktopChatShell: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @Binding var directChatInfoChatId: String?
    let onOpenNearby: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            DesktopChatSidebar(manager: manager, onOpenNearby: onOpenNearby)
                .frame(width: 352)

            Rectangle()
                .fill(palette.border)
                .frame(width: 1)

            VStack(spacing: 0) {
                desktopContent
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(palette.background)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(palette.background)
    }

    @ViewBuilder
    private var desktopContent: some View {
        switch manager.activeScreen {
        case .chatList, .newChat:
            DesktopPaneTopBar(title: "New Chat")
            NewChatScreen(manager: manager)
        case .newGroup:
            DesktopPaneTopBar(title: "New Group", canGoBack: manager.canNavigateBack, onBack: manager.navigateBack)
            NewGroupScreen(manager: manager)
        case .createInvite:
            DesktopPaneTopBar(title: "Invite", canGoBack: manager.canNavigateBack, onBack: manager.navigateBack)
            CreateInviteScreen(manager: manager)
        case .joinInvite:
            DesktopPaneTopBar(title: "Join Chat", canGoBack: manager.canNavigateBack, onBack: manager.navigateBack)
            JoinInviteScreen(manager: manager)
        case .settings:
            DesktopPaneTopBar(title: "Settings", canGoBack: manager.canNavigateBack, onBack: manager.navigateBack)
            SettingsScreen(manager: manager, focusedSection: .constant(nil))
        case .chat(let chatId):
            let chat = manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
            DesktopPaneTopBar(
                title: chat?.displayName ?? "Chat",
                subtitle: chat?.isMuted == true ? "muted" : chat?.subtitle,
                subtitleSystemImage: chat?.isMuted == true ? "bell.slash.fill" : nil,
                onTitleTap: chat.map { current in
                    {
                        if let groupId = current.groupId {
                            manager.dispatch(.pushScreen(screen: .groupDetails(groupId: groupId)))
                        } else {
                            directChatInfoChatId = current.chatId
                        }
                    }
                },
                leading: chat.map { current in
                    AnyView(
                        IrisAvatar(
                            label: current.displayName,
                            size: 36,
                            pictureUrl: current.pictureUrl,
                            preferences: manager.state.preferences,
                            manager: manager
                        )
                    )
                } ?? AnyView(EmptyView()),
                trailing: AnyView(EmptyView())
            )
            ChatScreen(manager: manager, chatId: chatId)
                .id(chatId)
        case .groupDetails(let groupId):
            DesktopPaneTopBar(title: "Group", canGoBack: true, onBack: manager.navigateBack)
            GroupDetailsScreen(manager: manager, groupId: groupId)
        case .deviceRoster:
            DesktopPaneTopBar(title: "Manage Devices", canGoBack: manager.canNavigateBack, onBack: manager.navigateBack)
            DeviceRosterScreen(manager: manager)
        case .welcome:
            WelcomeScreen(manager: manager)
        case .createAccount:
            CreateAccountScreen(manager: manager)
        case .restoreAccount:
            RestoreAccountScreen(manager: manager)
        case .addDevice:
            AddDeviceScreen(manager: manager, awaitingApproval: false)
        case .awaitingDeviceApproval:
            AddDeviceScreen(manager: manager, awaitingApproval: true)
        case .deviceRevoked:
            DeviceRevokedScreen(manager: manager)
        }
    }
}

private struct DesktopPaneTopBar: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let subtitle: String?
    let subtitleSystemImage: String?
    let canGoBack: Bool
    let onBack: () -> Void
    let onTitleTap: (() -> Void)?
    let leading: AnyView
    let trailing: AnyView

    init(
        title: String,
        subtitle: String? = nil,
        subtitleSystemImage: String? = nil,
        canGoBack: Bool = false,
        onBack: @escaping () -> Void = {},
        onTitleTap: (() -> Void)? = nil,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView())
    ) {
        self.title = title
        self.subtitle = subtitle
        self.subtitleSystemImage = subtitleSystemImage
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.onTitleTap = onTitleTap
        self.leading = leading
        self.trailing = trailing
    }

    @ViewBuilder
    private var titleStack: some View {
        HStack(spacing: 10) {
            leading

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(.headline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .lineLimit(1)

                if let subtitle, !subtitle.isEmpty {
                    HStack(spacing: 4) {
                        if let subtitleSystemImage {
                            Image(systemName: subtitleSystemImage)
                                .font(.system(size: 10, weight: .semibold))
                        }

                        Text(subtitle)
                            .font(.system(.caption, design: .rounded))
                    }
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
                }
            }
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            if canGoBack {
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .frame(width: 34, height: 34)
                }
                .buttonStyle(.irisPlain)
                .accessibilityIdentifier("desktopPaneBackButton")
            }

            if let onTitleTap {
                Button(action: onTitleTap) {
                    HStack(spacing: 0) {
                        titleStack
                        Spacer(minLength: 12)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("chatHeaderTitleButton")
            } else {
                titleStack
                Spacer(minLength: 12)
            }

            trailing
        }
        .padding(.horizontal, 24)
        .frame(height: 58)
        .background(palette.toolbar)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(palette.border)
                .frame(height: 1)
        }
    }
}

private struct DesktopChatSidebar: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let onOpenNearby: () -> Void
    @State private var searchText = ""

    private var filteredChats: [ChatThreadSnapshot] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return manager.state.chatList
        }
        return manager.state.chatList.filter { chat in
            chat.displayName.lowercased().contains(query)
                || (chat.lastMessagePreview?.lowercased().contains(query) ?? false)
                || (chat.subtitle?.lowercased().contains(query) ?? false)
        }
    }

    private var selectedChatId: String? {
        if case .chat(let chatId) = manager.activeScreen {
            return chatId
        }
        return nil
    }

    private var newChatSelected: Bool {
        switch manager.activeScreen {
        case .chatList, .newChat:
            return true
        default:
            return false
        }
    }

    var body: some View {
        let relativeNow = Date()

        VStack(spacing: 0) {
            sidebarHeader

            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(palette.muted)
                TextField("Search", text: $searchText)
                    .textFieldStyle(.plain)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.textPrimary)
            }
            .padding(.horizontal, 12)
            .frame(height: 38)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(palette.panelAlt)
            )
            .padding(.horizontal, 14)
            .padding(.bottom, 8)

            ScrollView {
                LazyVStack(spacing: 2) {
                    DesktopSidebarActionRow(
                        title: "New chat",
                        subtitle: nil,
                        systemImage: "message.fill",
                        selected: newChatSelected
                    ) {
                        manager.dispatch(.pushScreen(screen: .newChat))
                    }
                    .accessibilityIdentifier("desktopNewChatRow")

                    #if os(macOS)
                    DesktopNearbyIrisRow(service: manager.nearbyIris, onOpen: onOpenNearby)
                        .accessibilityIdentifier("desktopNearbyRow")
                    #endif

                    ForEach(filteredChats, id: \.chatId) { chat in
                        DesktopSidebarChatRow(
                            manager: manager,
                            chat: chat,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                            selected: selectedChatId == chat.chatId
                        )
                        .accessibilityIdentifier("chatRow-\(String(chat.chatId.prefix(12)))")
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 16)
            }
        }
        .background(palette.panel)
    }

    private var sidebarHeader: some View {
        HStack(spacing: 12) {
            if let account = manager.state.account {
                ZStack {
                    Button(action: { manager.dispatch(.pushScreen(screen: .settings)) }) {
                        IrisAvatar(
                            label: account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName,
                            size: 42,
                            emphasize: true,
                            pictureUrl: account.pictureUrl,
                            preferences: manager.state.preferences,
                            manager: manager
                        )
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("chatListProfileButton")
                    if hasHttpPicture(account.pictureUrl) || hasHashtreePicture(account.pictureUrl) {
                        Color.clear
                            .frame(width: 1, height: 1)
                            .accessibilityIdentifier("chatListProfileAvatarImage")
                            .allowsHitTesting(false)
                    }
                }
            }

            Text("Chats")
                .font(.system(.title2, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)

            Spacer()

            Button(action: { manager.dispatch(.pushScreen(screen: .settings)) }) {
                Image(systemName: "ellipsis")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 40, height: 40)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("desktopSettingsButton")
        }
        .padding(.horizontal, 18)
        .padding(.top, 18)
        .padding(.bottom, 14)
    }
}

private struct DesktopSidebarActionRow: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let subtitle: String?
    let systemImage: String
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(selected ? palette.onAccent : palette.textPrimary)
                    .frame(width: 42, height: 42)
                    .background(
                        Circle()
                            .fill(selected ? palette.accent : palette.panelAlt)
                    )

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    if let subtitle {
                        Text(subtitle)
                            .font(.system(.caption, design: .rounded))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }

                Spacer(minLength: 8)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
            .background(rowBackground)
        }
        .buttonStyle(.irisPlain)
    }

    private var rowBackground: some View {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
            .fill(selected ? palette.panelAlt : Color.clear)
    }
}

#if os(macOS)
private struct DesktopNearbyIrisRow: View {
    @ObservedObject var service: IrisNearbyService
    let onOpen: () -> Void

    var body: some View {
        DesktopSidebarActionRow(
            title: "Nearby",
            subtitle: service.sidebarSubtitle,
            systemImage: "dot.radiowaves.left.and.right",
            selected: false
        ) {
            onOpen()
        }
    }
}
#endif

private struct DesktopSidebarChatRow: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chat: ChatThreadSnapshot
    let timeLabel: String?
    let selected: Bool

    private var preview: String {
        if chat.isTyping {
            return "Typing"
        }
        return chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"
    }

    var body: some View {
        Button {
            manager.dispatch(.openChat(chatId: chat.chatId))
        } label: {
            HStack(alignment: .top, spacing: 12) {
                IrisAvatar(
                    label: chat.displayName,
                    size: 44,
                    emphasize: chat.unreadCount > 0,
                    pictureUrl: chat.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager
                )

                VStack(alignment: .leading, spacing: 4) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        HStack(alignment: .firstTextBaseline, spacing: 5) {
                            Text(chat.displayName)
                                .font(.system(.headline, design: .rounded, weight: chat.unreadCount > 0 ? .bold : .semibold))
                                .foregroundStyle(palette.textPrimary)
                                .lineLimit(1)

                            if chat.isMuted {
                                Image(systemName: "bell.slash.fill")
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(palette.muted)
                                    .accessibilityLabel("muted")
                            }

                            if chat.isPinned {
                                Image(systemName: "pin.fill")
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(palette.muted)
                                    .accessibilityLabel("pinned")
                            }
                        }
                        .layoutPriority(1)

                        Spacer(minLength: 8)

                        if let timeLabel, !timeLabel.isEmpty {
                            Text(timeLabel)
                                .font(.system(.caption, design: .rounded, weight: .medium))
                                .foregroundStyle(palette.muted)
                                .lineLimit(1)
                        }
                    }

                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(preview)
                            .font(.system(.subheadline, design: .rounded))
                            .foregroundStyle(chat.unreadCount > 0 ? palette.textPrimary : palette.muted)
                            .lineLimit(2)

                        Spacer(minLength: 6)

                        Text(chat.unreadCount > 99 ? "99+" : "\(max(chat.unreadCount, 1))")
                            .font(.system(size: 11, weight: .bold, design: .rounded))
                            .foregroundStyle(chat.unreadCount > 0 ? palette.onAccent : Color.clear)
                            .padding(.horizontal, 7)
                            .frame(minHeight: 20)
                            .background(Capsule().fill(chat.unreadCount > 0 ? palette.accent : Color.clear))
                            .accessibilityHidden(chat.unreadCount == 0)
                    }
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .contentShape(Rectangle())
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(selected ? palette.panelAlt : Color.clear)
            )
        }
        .buttonStyle(.irisPlain)
        .contextMenu {
            chatListItemContextMenu(manager: manager, chat: chat)
        }
    }
}

@ViewBuilder
private func chatListItemContextMenu(manager: AppManager, chat: ChatThreadSnapshot) -> some View {
    Button {
        manager.dispatch(.setChatUnread(chatId: chat.chatId, unread: chat.unreadCount == 0))
    } label: {
        IrisContextMenuLabel(
            title: chat.unreadCount > 0 ? "Mark read" : "Mark as unread",
            systemImage: chat.unreadCount > 0 ? "envelope.open.fill" : "envelope.badge.fill"
        )
    }

    Button {
        manager.dispatch(.setChatPinned(chatId: chat.chatId, pinned: !chat.isPinned))
    } label: {
        IrisContextMenuLabel(
            title: chat.isPinned ? "Unpin chat" : "Pin chat",
            systemImage: chat.isPinned ? "pin.slash.fill" : "pin.fill"
        )
    }

    Button {
        manager.dispatch(.setChatMuted(chatId: chat.chatId, muted: !chat.isMuted))
    } label: {
        IrisContextMenuLabel(
            title: chat.isMuted ? "Unmute chat" : "Mute chat",
            systemImage: chat.isMuted ? "bell.fill" : "bell.slash.fill"
        )
    }

    Button(role: .destructive) {
        manager.dispatch(.deleteChat(chatId: chat.chatId))
    } label: {
        Label("Delete", systemImage: "trash.fill")
    }
}

private struct IrisContextMenuLabel: View {
    let title: String
    let systemImage: String

    var body: some View {
        Label {
            Text(title)
        } icon: {
            Image(systemName: systemImage)
                .renderingMode(.template)
                .foregroundStyle(Color.primary)
        }
    }
}

#if os(macOS)
private struct DesktopUpdateStripe: View {
    @Environment(\.irisPalette) private var palette
    // Observe only the update controller so unrelated AppManager publishes
    // (relay events, typing pings, scene phase) don't re-evaluate this view.
    @ObservedObject var updates: DesktopUpdateController

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "arrow.down.circle.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(palette.muted)

            Text(updates.version.isEmpty ? "Update available" : "\(updates.version) available")
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .lineLimit(1)

            Spacer(minLength: 12)

            Toggle("Install automatically", isOn: $updates.autoInstall)
                .toggleStyle(.checkbox)
                .irisControlTint()
                .font(.system(.caption, design: .rounded, weight: .medium))
                .foregroundStyle(palette.muted)
                .accessibilityIdentifier("desktopUpdateAutoInstallToggle")

            Button {
                updates.install()
            } label: {
                Text(updates.installing ? "Installing…" : "Install")
            }
            .buttonStyle(IrisSecondaryButtonStyle(compact: true))
            .disabled(!updates.canInstall)
            .accessibilityIdentifier("desktopInstallUpdateButton")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(palette.panelAlt)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(palette.border)
                .frame(height: 1)
        }
        .accessibilityIdentifier("desktopUpdateStripe")
    }
}
#endif

private struct IdentifiedString: Identifiable, Hashable {
    let value: String
    var id: String { value }
}

private struct IrisProfilePictureViewerItem: Identifiable, Equatable {
    let label: String
    let pictureUrl: String
    let accessibilityIdentifier: String

    var id: String { "\(accessibilityIdentifier)|\(pictureUrl)" }

    init?(
        label: String,
        pictureUrl: String?,
        accessibilityIdentifier: String
    ) {
        let trimmed = pictureUrl?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard irisCanOpenProfilePicture(trimmed) else { return nil }
        self.label = label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Profile" : label
        self.pictureUrl = trimmed
        self.accessibilityIdentifier = accessibilityIdentifier
    }
}

private struct DirectChatInfoScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chatId: String
    let onClose: () -> Void
    @State private var advancedExpanded = false
    @State private var profileDebug: PeerProfileDebugSnapshot?
    @State private var loadedProfileDebugFor: String?
    @State private var profilePictureViewerItem: IrisProfilePictureViewerItem?
    @State private var commonGroups: [ChatThreadSnapshot] = []
    @State private var commonGroupsLoadedFor: String?
    @State private var showingBlockConfirmation = false
    @State private var showingUnblockConfirmation = false
    @State private var showingReportConfirmation = false

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let chat {
                    VStack(spacing: 10) {
                        directChatAvatar(chat)
                        Text(chat.displayName)
                            .font(.system(.title2, design: .rounded, weight: .bold))
                            .foregroundStyle(palette.textPrimary)
                            .multilineTextAlignment(.center)
                        if let subtitle = chat.subtitle, !subtitle.isEmpty {
                            Text(subtitle)
                                .font(.system(.subheadline, design: .rounded))
                                .foregroundStyle(palette.muted)
                                .multilineTextAlignment(.center)
                        }
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.top, 22)

                    if !commonGroups.isEmpty {
                        IrisSectionCard {
                            Text("Groups in common")
                                .font(.system(.headline, design: .rounded, weight: .semibold))
                                .foregroundStyle(palette.textPrimary)

                            VStack(spacing: 0) {
                                ForEach(Array(commonGroups.enumerated()), id: \.element.chatId) { index, group in
                                    commonGroupRow(group)
                                    if index < commonGroups.count - 1 {
                                        Divider().overlay(palette.border)
                                            .padding(.leading, 50)
                                    }
                                }
                            }
                        }
                    }

                    IrisSectionCard {
                        Button {
                            manager.dispatch(.setChatMuted(chatId: chatId, muted: !chat.isMuted))
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: chat.isMuted ? "bell.fill" : "bell.slash.fill")
                                    .frame(width: 24)
                                Text(chat.isMuted ? "Unmute chat" : "Mute chat")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(palette.textPrimary)
                            .padding(.vertical, 2)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatMuteButton")

                        Divider().overlay(palette.border)

                        IrisCopyButton(label: "Copy user ID", value: peerInputToNpub(input: chatId), compact: false)
                            .accessibilityIdentifier("directChatCopyUserIdButton")
                    }

                    IrisSectionCard {
                        CardHeader(
                            title: "Disappearing messages",
                            subtitle: nil
                        )
                        VStack(spacing: 0) {
                            ForEach(disappearingMessageOptions, id: \.0) { label, ttlSeconds in
                                Button {
                                    manager.dispatch(.setChatMessageTtl(chatId: chatId, ttlSeconds: ttlSeconds))
                                } label: {
                                    HStack {
                                        Text(label)
                                            .foregroundStyle(palette.textPrimary)
                                        Spacer()
                                        if chat.messageTtlSeconds == ttlSeconds {
                                            Image(systemName: "checkmark")
                                                .font(.system(size: 14, weight: .semibold))
                                                .foregroundStyle(palette.textPrimary)
                                        }
                                    }
                                    .padding(.vertical, 10)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.irisPlain)
                            }
                        }
                    }

                    DirectChatAdvancedCard(
                        debug: profileDebug,
                        isExpanded: $advancedExpanded
                    )
                    .accessibilityIdentifier("directChatAdvancedCard")
                    .onAppear(perform: loadProfileDebugIfNeeded)
                    .irisOnChange(of: advancedExpanded) { _ in
                        loadProfileDebugIfNeeded()
                    }

                    IrisSectionCard {
                        Button(role: manager.isUserBlocked(chatId) ? nil : .destructive) {
                            if manager.isUserBlocked(chatId) {
                                showingUnblockConfirmation = true
                            } else {
                                showingBlockConfirmation = true
                            }
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: manager.isUserBlocked(chatId) ? "checkmark.shield.fill" : "nosign")
                                    .frame(width: 24)
                                Text(manager.isUserBlocked(chatId) ? "Unblock user" : "Block user")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(manager.isUserBlocked(chatId) ? palette.textPrimary : .red)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatBlockButton")

                        Divider().overlay(palette.border)

#if os(iOS)
                        // The mailto: hand-off to irismessenger@pm.me is
                        // wired specifically for the iOS App Store
                        // user-generated-content review process — Apple
                        // requires every UGC-bearing iOS app to expose
                        // a way to flag abusive content. Other platforms
                        // route abuse handling through their own surfaces
                        // (Zapstore listing, GitHub issues, the irischat.org
                        // contact), so the in-app Report button is
                        // intentionally iOS-only.
                        Button(role: .destructive) {
                            showingReportConfirmation = true
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "flag.fill")
                                    .frame(width: 24)
                                Text("Report user")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(.red)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatReportButton")

                        Divider().overlay(palette.border)
#endif

                        Button(role: .destructive) {
                            manager.dispatch(.deleteChat(chatId: chatId))
                            onClose()
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "trash")
                                    .frame(width: 24)
                                Text("Delete chat")
                                    .font(.system(.body, design: .rounded, weight: .semibold))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(.red)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("directChatDeleteButton")
                    }
                } else {
                    ProgressView()
                        .padding(.top, 40)
                }
            }
            .padding(.horizontal, 18)
            .padding(.bottom, 24)
            .frame(maxWidth: .infinity, alignment: .leading)
            .textSelection(.enabled)
        }
        .background(palette.background)
        .irisProfilePictureViewer(
            item: $profilePictureViewerItem,
            preferences: manager.state.preferences,
            manager: manager
        )
        .task(id: chatId) {
            loadCommonGroups()
        }
        .confirmationDialog(
            "Block user?",
            isPresented: $showingBlockConfirmation,
            titleVisibility: .visible
        ) {
            Button("Block user", role: .destructive) {
                manager.setUserBlocked(chatId, blocked: true)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("You will not send messages to this user.")
        }
        .confirmationDialog(
            "Unblock user?",
            isPresented: $showingUnblockConfirmation,
            titleVisibility: .visible
        ) {
            Button("Unblock user") {
                manager.setUserBlocked(chatId, blocked: false)
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog(
            "Report user?",
            isPresented: $showingReportConfirmation,
            titleVisibility: .visible
        ) {
            Button("Report and block", role: .destructive) {
                if let chat {
                    reportUser(chat, block: true)
                }
            }
            Button("Report only") {
                if let chat {
                    reportUser(chat, block: false)
                }
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func directChatAvatar(_ chat: CurrentChatSnapshot) -> some View {
        if let item = IrisProfilePictureViewerItem(
            label: chat.displayName,
            pictureUrl: chat.pictureUrl,
            accessibilityIdentifier: "directChatProfilePictureViewer"
        ) {
            Button {
                profilePictureViewerItem = item
            } label: {
                directChatAvatarImage(chat)
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Open profile picture")
            .accessibilityIdentifier("directChatProfilePictureButton")
        } else {
            directChatAvatarImage(chat)
        }
    }

    private func directChatAvatarImage(_ chat: CurrentChatSnapshot) -> some View {
        IrisAvatar(
            label: chat.displayName,
            size: 96,
            emphasize: true,
            pictureUrl: chat.pictureUrl,
            preferences: manager.state.preferences,
            manager: manager,
            loadedImageIdentifier: "directChatProfileAvatarImage"
        )
    }

    @ViewBuilder
    private func commonGroupRow(_ group: ChatThreadSnapshot) -> some View {
        Button {
            if let groupId = groupId(from: group.chatId) {
                manager.dispatch(.pushScreen(screen: .groupDetails(groupId: groupId)))
            }
        } label: {
            HStack(spacing: 12) {
                IrisAvatar(
                    label: group.displayName,
                    size: 38,
                    emphasize: false,
                    pictureUrl: group.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager
                )
                VStack(alignment: .leading, spacing: 3) {
                    Text(group.displayName)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    Text("\(group.memberCount) people")
                        .font(.system(.footnote, design: .rounded))
                        .foregroundStyle(palette.muted)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 9)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
    }

    private func loadProfileDebugIfNeeded() {
        guard advancedExpanded else { return }
        if loadedProfileDebugFor != chatId {
            profileDebug = nil
            loadedProfileDebugFor = chatId
        }
        guard profileDebug == nil else { return }
        profileDebug = manager.peerProfileDebug(ownerInput: chatId)
    }

    private func loadCommonGroups() {
        guard commonGroupsLoadedFor != chatId else { return }
        commonGroupsLoadedFor = chatId
        commonGroups = manager.mutualGroups(ownerInput: chatId)
    }

    private func reportUser(_ chat: CurrentChatSnapshot, block: Bool) {
        if block {
            manager.setUserBlocked(chatId, blocked: true)
        }

        let userId = peerInputToNpub(input: chatId)
        let body = """
        Reported user: \(chat.displayName)
        User ID: \(userId)
        App: Iris Chat \(manager.buildSummaryText())

        What happened:
        """
        guard let url = irisMailtoURL(
            to: irisSupportEmail,
            subject: "Iris Chat user report",
            body: body
        ) else {
            manager.copyToClipboard("User ID: \(userId)")
            return
        }
        PlatformExternalURL.open(url)
    }

    private func groupId(from chatId: String) -> String? {
        let prefix = "group:"
        guard chatId.lowercased().hasPrefix(prefix) else {
            return nil
        }
        let raw = String(chatId.dropFirst(prefix.count))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return raw.isEmpty ? nil : raw
    }
}

private struct DirectChatAdvancedCard: View {
    @Environment(\.irisPalette) private var palette
    let debug: PeerProfileDebugSnapshot?
    @Binding var isExpanded: Bool

    var body: some View {
        IrisSectionCard {
            DisclosureGroup(isExpanded: $isExpanded) {
                if let debug {
                    VStack(alignment: .leading, spacing: 10) {
                        DirectChatDebugRow(label: "Sessions", value: "\(debug.sessionCount)")
                        DirectChatDebugRow(label: "Active sessions", value: "\(debug.activeSessionCount)")
                        DirectChatDebugRow(label: "Receiving sessions", value: "\(debug.receivingSessionCount)")
                        DirectChatDebugRow(label: "Known devices", value: "\(debug.knownDeviceCount)")
                        DirectChatDebugRow(label: "Device roster", value: "\(debug.rosterDeviceCount)")
                        DirectChatDebugRow(label: "Tracked senders", value: "\(debug.trackedSenderCount)")
                        DirectChatDebugRow(label: "Recent handshakes", value: "\(debug.recentHandshakeDeviceCount)")
                        DirectChatDebugRow(label: "Last handshake", value: lastHandshakeText(debug.lastHandshakeAtSecs))
                        DirectChatDebugRow(label: "Message tracking", value: debug.trackedForMessages ? "On" : "Off")
                        MonoValue(label: "User ID", value: debug.ownerNpub)
                        MonoValue(label: "Public key", value: debug.ownerPubkeyHex)
                    }
                    .padding(.top, 10)
                } else {
                    ProgressView()
                        .padding(.top, 10)
                }
            } label: {
                HStack(spacing: 9) {
                    Image(systemName: "wrench.and.screwdriver.fill")
                        .foregroundStyle(palette.textPrimary)
                    Text("Debug")
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
        }
    }
}

private struct DirectChatDebugRow: View {
    @Environment(\.irisPalette) private var palette
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(label)
                .font(.system(.body, design: .rounded))
                .foregroundStyle(palette.muted)
            Spacer(minLength: 12)
            Text(value)
                .font(.system(.body, design: .rounded, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(palette.textPrimary)
                .multilineTextAlignment(.trailing)
        }
    }
}

private func lastHandshakeText(_ seconds: UInt64?) -> String {
    guard let seconds else { return "Never" }
    return Date(timeIntervalSince1970: TimeInterval(seconds))
        .formatted(date: .abbreviated, time: .shortened)
}

private func relayStatusColor(_ status: NetworkStatusSnapshot?, palette: IrisPalette) -> Color {
    guard let status, !status.relayUrls.isEmpty else {
        return palette.muted.opacity(0.55)
    }
    if status.connectedRelayCount > 0 {
        return Color(red: 34.0 / 255.0, green: 197.0 / 255.0, blue: 94.0 / 255.0)
    }
    if status.syncing || status.pendingOutboundCount > 0 || status.pendingGroupControlCount > 0 {
        return Color(red: 234.0 / 255.0, green: 179.0 / 255.0, blue: 8.0 / 255.0)
    }
    return Color(red: 239.0 / 255.0, green: 68.0 / 255.0, blue: 68.0 / 255.0)
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

extension Array where Element == ChatThreadSnapshot {
    func filteredByQuery(_ query: String) -> [ChatThreadSnapshot] {
        let raw = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return self }
        let lower = raw.lowercased()
        let normalized = normalizePeerInput(input: raw).lowercased()
        return filter { chat in
            chat.displayName.lowercased().contains(lower)
                || chat.chatId.lowercased().contains(normalized)
                || (chat.subtitle?.lowercased().contains(lower) ?? false)
        }
    }
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
            VStack(spacing: 20) {
                VStack(spacing: 18) {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("welcomeChooserCard")

                    Image("IrisLogo")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 132, height: 132)
                        .accessibilityHidden(true)

                    HStack(spacing: 0) {
                        Text("iris")
                            .foregroundStyle(palette.accent)
                        Text(" chat")
                            .foregroundStyle(palette.textPrimary)
                    }
                    .font(.system(.largeTitle, design: .rounded, weight: .bold))

                    VStack(spacing: 10) {
                        Button {
                            manager.dispatch(.pushScreen(screen: .createAccount))
                        } label: {
                            Label("Create profile", systemImage: "plus")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisPrimaryButtonStyle())
                        .accessibilityIdentifier("welcomeCreateAction")

                        Button {
                            manager.dispatch(.pushScreen(screen: .restoreAccount))
                        } label: {
                            Label("Restore profile", systemImage: "key.fill")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("welcomeRestoreAction")

                        Button {
                            manager.dispatch(.pushScreen(screen: .addDevice))
                        } label: {
                            Label("Link this device", systemImage: "iphone")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(IrisSecondaryButtonStyle())
                        .accessibilityIdentifier("welcomeAddDeviceAction")
                    }
                    .frame(maxWidth: 320)
                }
                .frame(maxWidth: .infinity)

                if manager.trustedTestBuildEnabled() {
                    Text("Test build")
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.accentAlt)
                        .accessibilityIdentifier("welcomeSecondaryCard")
                }
            }
            .frame(maxWidth: 480)
            .frame(maxWidth: .infinity)
            .padding(.top, IrisLayout.usesDesktopChrome ? 96 : 56)
        }
    }
}

struct CreateAccountScreen: View {
    @ObservedObject var manager: AppManager
    @State private var displayName = ""
    @FocusState private var isNameFocused: Bool

    private var trimmedDisplayName: String {
        displayName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canCreateAccount: Bool {
        !trimmedDisplayName.isEmpty && !manager.state.busy.creatingAccount
    }

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("createAccountScreen")

                CardHeader(
                    title: "Create profile"
                )

                TextField("Name", text: $displayName)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .focused($isNameFocused)
                    .submitLabel(.done)
                    .onSubmit(submitCreateAccount)
                    .accessibilityIdentifier("signupNameField")

                Button(manager.state.busy.creatingAccount ? "Creating…" : "Create profile") {
                    submitCreateAccount()
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(!canCreateAccount)
                .accessibilityIdentifier("generateKeyButton")
            }
        }
        .onAppear {
            DispatchQueue.main.async {
                isNameFocused = true
            }
        }
    }

    private func submitCreateAccount() {
        guard canCreateAccount else { return }
        manager.createAccount(name: trimmedDisplayName)
    }
}

struct RestoreAccountScreen: View {
    @ObservedObject var manager: AppManager
    @StateObject private var restoreSecret = SecretKeyDraft()
    @State private var lastSubmittedSecret: String?

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("restoreAccountScreen")

                CardHeader(
                    title: "Restore profile",
                    subtitle: "Paste your secret key."
                )

                SecretKeyField(text: Binding(
                    get: { restoreSecret.text },
                    set: { updateSecret($0) }
                ))
                    .irisInputField()

                Button(manager.state.busy.restoringSession ? "Restoring…" : "Restore profile") {
                    submitRestore(restoreSecret.text, force: true)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(manager.state.busy.restoringSession)
                .accessibilityIdentifier("importKeyButton")
            }
        }
    }

    private func updateSecret(_ value: String) {
        let previous = restoreSecret.text.trimmingCharacters(in: .whitespacesAndNewlines)
        restoreSecret.text = value
        let current = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard shouldAutoSubmitSecret(previous: previous, current: current) else {
            return
        }
        submitRestore(current)
    }

    private func submitRestore(_ value: String, force: Bool = false) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            manager.restoreSession(ownerNsec: trimmed)
            return
        }
        guard !manager.state.busy.restoringSession else { return }
        guard force || lastSubmittedSecret != trimmed else {
            return
        }
        lastSubmittedSecret = trimmed
        manager.restoreSession(ownerNsec: trimmed)
    }
}

struct AddDeviceScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let awaitingApproval: Bool

    @State private var showingLogoutConfirmation = false

    var body: some View {
        IrisScrollScreen {
            linkDeviceCard
                .frame(maxWidth: 480)
                .frame(maxWidth: .infinity)
        }
        .onAppear {
            if !awaitingApproval,
               manager.state.linkDevice == nil,
               !manager.state.busy.linkingDevice {
                manager.startLinkedDevice(ownerInput: "")
            }
        }
        .alert("Delete all local data?", isPresented: $showingLogoutConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.logout()
            }
            .accessibilityIdentifier("awaitingApprovalConfirmLogoutButton")
        } message: {
            Text("This removes your secret keys, messages, and cached files from this device.")
        }
    }

    private var linkDeviceCard: some View {
        IrisSectionCard {
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("addDeviceScreen")

            CardHeader(
                title: awaitingApproval ? "Finish linking" : "Link this device",
                subtitle: awaitingApproval
                    ? "Waiting for approval from your signed-in device."
                    : "Scan this code with your signed-in device."
            )

            if awaitingApproval {
                Button("Sign out") {
                    showingLogoutConfirmation = true
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("awaitingApprovalLogoutButton")
            } else if let linkDevice = manager.state.linkDevice {
                ZStack {
                    QrCodeImage(text: linkDevice.url)
                        .frame(width: 240, height: 240)
                    Color.clear
                        .accessibilityIdentifier("linkDeviceQrCode")
                }
                .frame(maxWidth: .infinity)

                VStack(spacing: 10) {
                    Button("Copy link code") {
                        manager.copyToClipboard(linkDevice.url)
                    }
                    .buttonStyle(IrisPrimaryButtonStyle())
                    .accessibilityIdentifier("linkDeviceCopyButton")

                    Button(manager.state.busy.linkingDevice ? "Creating…" : "New code") {
                        manager.startLinkedDevice(ownerInput: "")
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .disabled(manager.state.busy.linkingDevice)
                    .accessibilityIdentifier("linkDeviceRefreshButton")
                }
            } else {
                ProgressView()
                    .accessibilityIdentifier("linkDeviceCreating")
            }
        }
    }
}

struct ChatListScreen: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.irisNavigationHeaderTopInset) private var navigationHeaderTopInset
    @ObservedObject var manager: AppManager
    let onOpenNearby: () -> Void
    @State private var searchText: String = ""
    // Cached search response so we only call into the Rust core when
    // the query string itself changes. Previously the body grabbed a
    // fresh `manager.search(...)` snapshot on every SwiftUI body
    // re-eval, which fires on every state push (incoming relay
    // event, typing indicator, message delivery flip, …). On a busy
    // chat that turned into hundreds of FFI calls / SQLite-mutex
    // acquires per second and was visible on iPhone as a warming
    // device. The .task below now refreshes the cache once per
    // searchText edit.
    @State private var cachedSearchResults: SearchResultSnapshot?
    @State private var expandedSearchSections: Set<ChatListSearchSection> = []
    @State private var searchMessageLimit: UInt32 = 50
    @State private var lastExpansionQuery: String = ""

    private static let initialMessageSearchLimit: UInt32 = 50
    private static let messageSearchLimitStep: UInt32 = 50

    init(manager: AppManager, onOpenNearby: @escaping () -> Void = {}) {
        self.manager = manager
        self.onOpenNearby = onOpenNearby
    }

    private var trimmedQuery: String {
        searchText.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var searchActive: Bool { !trimmedQuery.isEmpty }

    private var searchRequestToken: String {
        "\(trimmedQuery)|\(searchMessageLimit)"
    }

    var body: some View {
        let relativeNow = Date()

#if os(iOS)
        ChatListTableView(
            searchText: $searchText,
            manager: manager,
            chats: manager.state.chatList,
            preferences: manager.state.preferences,
            relativeNow: relativeNow,
            palette: palette,
            topContentInset: navigationHeaderTopInset,
            isSearchActive: searchActive,
            cachedSearchResults: cachedSearchResults,
            expandedSearchSections: expandedSearchSections,
            messageLimit: searchMessageLimit,
            onOpenNearby: onOpenNearby,
            onViewMoreSearchResults: viewMoreSearchResults
        )
        .background(palette.background)
        .irisOnChange(of: searchText) { _ in
            resetSearchExpansionIfNeeded()
            autoProceedIfShortcut()
        }
        .task(id: searchRequestToken) {
            // Refresh the search cache once per query change. Body
            // re-runs on every state push otherwise — the cache turns
            // an O(state-push) FTS5 query into an O(keystroke) one.
            cachedSearchResults = trimmedQuery.isEmpty
                ? nil
                : manager.search(trimmedQuery, limit: searchMessageLimit)
        }
#else
        ScrollView {
            LazyVStack(spacing: 0) {
                ChatListSearchField(text: $searchText)

                if searchActive {
                    if let results = cachedSearchResults {
                        SearchResultsList(
                            manager: manager,
                            results: results,
                            relativeNow: relativeNow,
                            expandedSections: expandedSearchSections,
                            messageLimit: searchMessageLimit,
                            onViewMore: viewMoreSearchResults
                        )
                    }
                } else {
#if os(iOS) || os(macOS)
                    if manager.state.preferences.nearbyEnabled {
                        NearbyChatListRow(manager: manager, service: manager.nearbyIris, onOpen: onOpenNearby)
                    }
#endif

                    if manager.state.chatList.isEmpty {
                        Text("No chats yet")
                            .font(.system(.body, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 20)
                    } else {
                        let preferences = manager.state.preferences
                        ForEach(manager.state.chatList, id: \.chatId) { chat in
                            ChatListRowContainer(
                                manager: manager,
                                chat: chat,
                                timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                                preferences: preferences
                            )
                            .accessibilityIdentifier("chatRow-\(String(chat.chatId.prefix(12)))")
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .top)
        }
        .background(palette.background)
        .irisOnChange(of: searchText) { _ in
            resetSearchExpansionIfNeeded()
            autoProceedIfShortcut()
        }
        .task(id: searchRequestToken) {
            // Refresh the search cache once per query change. Body
            // re-runs on every state push otherwise — the cache turns
            // an O(state-push) FTS5 query into an O(keystroke) one.
            cachedSearchResults = trimmedQuery.isEmpty
                ? nil
                : manager.search(trimmedQuery, limit: searchMessageLimit)
        }
#endif
    }

    private func resetSearchExpansionIfNeeded() {
        let query = trimmedQuery
        guard query != lastExpansionQuery else {
            return
        }
        lastExpansionQuery = query
        expandedSearchSections.removeAll()
        searchMessageLimit = Self.initialMessageSearchLimit
    }

    private func viewMoreSearchResults(_ section: ChatListSearchSection) {
        if section == .messages, expandedSearchSections.contains(section) {
            let nextLimit = searchMessageLimit.addingReportingOverflow(Self.messageSearchLimitStep)
            searchMessageLimit = nextLimit.overflow ? UInt32.max : nextLimit.partialValue
        } else {
            expandedSearchSections.insert(section)
        }
    }

    /// Mirrors NewChatScreen's auto-proceed: when the user pastes a
    /// full npub or invite URL into the search bar, dispatch the
    /// matching action without making them tap the shortcut row.
    /// Partial input never classifies, so this is safe to call on
    /// every keystroke.
    private func autoProceedIfShortcut() {
        let trimmed = trimmedQuery
        guard !trimmed.isEmpty,
              let shortcut = classifyChatInput(input: trimmed) else { return }
        switch shortcut {
        case let .directPeer(peerInput, _, _, _):
            searchText = ""
            manager.dispatch(.createChat(peerInput: peerInput))
        case let .invite(inviteInput, _):
            searchText = ""
            manager.dispatch(.acceptInvite(inviteInput: inviteInput))
        }
    }
}

private enum ChatListSearchSection: String, Hashable {
    case contacts
    case groups
    case messages
}

/// Always-visible search field at the top of the chat list. Drives the
/// grouped Signal-style search results below it. We render the field
/// inline (instead of using `.searchable`) so it composes cleanly with
/// the custom `NavigationShell` we use across iOS/macOS/Linux instead
/// of a stock `NavigationStack`.
private struct ChatListSearchField: View {
    @Environment(\.irisPalette) private var palette
    @Binding var text: String
    @FocusState private var isFocused: Bool

    var body: some View {
#if os(iOS)
        IrisChatListSearchBar(text: $text)
            .frame(height: 52)
            .padding(.horizontal, 8)
            .padding(.top, 4)
            .padding(.bottom, 2)
#else
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(palette.muted)
            TextField("Search chats, groups, messages", text: $text)
                .textFieldStyle(.plain)
                .autocorrectionDisabled(true)
#if os(iOS)
                .textInputAutocapitalization(.never)
#endif
                .focused($isFocused)
                .accessibilityIdentifier("chatListSearchField")
            if isFocused {
                Button {
                    isFocused = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close search")
                .accessibilityIdentifier("chatListSearchCloseButton")
            } else if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(palette.panelAlt)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal, 12)
        .padding(.top, 10)
        .padding(.bottom, 4)
#endif
    }
}

#if os(iOS)
private struct IrisChatListSearchBar: UIViewRepresentable {
    @Environment(\.colorScheme) private var colorScheme
    @Binding var text: String

    private var isDark: Bool {
        colorScheme == .dark
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeUIView(context: Context) -> UISearchBar {
        let searchBar = UISearchBar(frame: .zero)
        searchBar.delegate = context.coordinator
        searchBar.placeholder = "Search"
        searchBar.autocapitalizationType = .none
        searchBar.autocorrectionType = .no
        searchBar.returnKeyType = .search
        searchBar.enablesReturnKeyAutomatically = false
        searchBar.searchBarStyle = .minimal
        searchBar.backgroundColor = .clear
        searchBar.backgroundImage = UIImage()
        searchBar.searchTextField.accessibilityIdentifier = "chatListSearchField"
        searchBar.searchTextField.clearButtonMode = .never
        Self.applyAppearance(to: searchBar, isDark: isDark)
        context.coordinator.attach(to: searchBar)
        return searchBar
    }

    func updateUIView(_ searchBar: UISearchBar, context: Context) {
        if searchBar.text != text {
            searchBar.text = text
        }
        Self.applyAppearance(to: searchBar, isDark: isDark)
        context.coordinator.updateCloseButton(for: searchBar, isDark: isDark)
    }

    private static func applyAppearance(to searchBar: UISearchBar, isDark: Bool) {
        let style: UIUserInterfaceStyle = isDark ? .dark : .light
        let field = searchBar.searchTextField

        searchBar.overrideUserInterfaceStyle = style
        searchBar.tintColor = .label
        searchBar.backgroundColor = .clear
        searchBar.barTintColor = .clear
        searchBar.searchBarStyle = .minimal

        field.overrideUserInterfaceStyle = style
        field.textColor = .label
        field.tintColor = .label
        field.leftView?.tintColor = .secondaryLabel
    }

    final class Coordinator: NSObject, UISearchBarDelegate {
        @Binding private var text: String
        private var isFocused = false
        private lazy var closeButton: UIButton = {
            let button = UIButton(type: .system)
            button.setImage(UIImage(systemName: "xmark.circle.fill"), for: .normal)
            button.tintColor = .secondaryLabel
            button.accessibilityLabel = "Close search"
            button.accessibilityIdentifier = "chatListSearchCloseButton"
            button.addTarget(self, action: #selector(closeSearch), for: .touchUpInside)
            button.frame = CGRect(x: 0, y: 0, width: 28, height: 28)
            return button
        }()

        init(text: Binding<String>) {
            self._text = text
        }

        func attach(to searchBar: UISearchBar) {
            updateCloseButton(for: searchBar, isDark: searchBar.traitCollection.userInterfaceStyle == .dark)
        }

        func updateCloseButton(for searchBar: UISearchBar, isDark: Bool) {
            closeButton.overrideUserInterfaceStyle = isDark ? .dark : .light
            closeButton.tintColor = .secondaryLabel
            searchBar.searchTextField.rightView = isFocused ? closeButton : nil
            searchBar.searchTextField.rightViewMode = isFocused ? .always : .never
        }

        func searchBarTextDidBeginEditing(_ searchBar: UISearchBar) {
            isFocused = true
            updateCloseButton(for: searchBar, isDark: searchBar.overrideUserInterfaceStyle == .dark)
        }

        func searchBarTextDidEndEditing(_ searchBar: UISearchBar) {
            isFocused = false
            updateCloseButton(for: searchBar, isDark: searchBar.overrideUserInterfaceStyle == .dark)
        }

        func searchBar(_ searchBar: UISearchBar, textDidChange searchText: String) {
            text = searchText
        }

        func searchBarSearchButtonClicked(_ searchBar: UISearchBar) {
            searchBar.resignFirstResponder()
        }

        @objc private func closeSearch(_ sender: UIButton) {
            sender.window?.endEditing(true)
        }
    }
}
#endif

private struct SearchResultsList: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let results: SearchResultSnapshot
    let relativeNow: Date
    let expandedSections: Set<ChatListSearchSection>
    let messageLimit: UInt32
    let onViewMore: (ChatListSearchSection) -> Void

    private let initialChatRows = 7
    private let initialMessageRows = 20

    var body: some View {
        let preferences = manager.state.preferences
        let isEmpty = results.contacts.isEmpty
            && results.groups.isEmpty
            && results.messages.isEmpty
            && results.shortcut == nil

        if isEmpty {
            Text("No matches")
                .font(.system(.body, design: .rounded))
                .foregroundStyle(palette.muted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 28)
        } else {
            LazyVStack(alignment: .leading, spacing: 0) {
                if let shortcut = results.shortcut {
                    ChatInputShortcutRow(manager: manager, shortcut: shortcut)
                }
                if !results.contacts.isEmpty {
                    SearchSectionHeader(title: "Contacts")
                    let contacts = visibleRows(
                        results.contacts,
                        section: .contacts,
                        initialCount: initialChatRows
                    )
                    ForEach(contacts, id: \.chatId) { chat in
                        ChatListRowContainer(
                            manager: manager,
                            chat: chat,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                            preferences: preferences
                        )
                    }
                    if shouldShowMore(results.contacts, visibleRows: contacts, section: .contacts) {
                        SearchViewMoreRow {
                            onViewMore(.contacts)
                        }
                    }
                }
                if !results.groups.isEmpty {
                    SearchSectionHeader(title: "Groups")
                    let groups = visibleRows(
                        results.groups,
                        section: .groups,
                        initialCount: initialChatRows
                    )
                    ForEach(groups, id: \.chatId) { chat in
                        ChatListRowContainer(
                            manager: manager,
                            chat: chat,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                            preferences: preferences
                        )
                    }
                    if shouldShowMore(results.groups, visibleRows: groups, section: .groups) {
                        SearchViewMoreRow {
                            onViewMore(.groups)
                        }
                    }
                }
                if !results.messages.isEmpty {
                    SearchSectionHeader(title: "Messages")
                    let messages = visibleRows(
                        results.messages,
                        section: .messages,
                        initialCount: initialMessageRows
                    )
                    ForEach(messages, id: \.messageId) { hit in
                        MessageSearchHitRow(
                            manager: manager,
                            hit: hit,
                            relativeNow: relativeNow,
                            preferences: preferences
                        )
                    }
                    if shouldShowMoreMessages(visibleRows: messages) {
                        SearchViewMoreRow {
                            onViewMore(.messages)
                        }
                    }
                }
            }
        }
    }

    private func visibleRows<T>(
        _ rows: [T],
        section: ChatListSearchSection,
        initialCount: Int
    ) -> [T] {
        expandedSections.contains(section) ? rows : Array(rows.prefix(initialCount))
    }

    private func shouldShowMore<T>(
        _ rows: [T],
        visibleRows: [T],
        section: ChatListSearchSection
    ) -> Bool {
        !expandedSections.contains(section) && rows.count > visibleRows.count
    }

    private func shouldShowMoreMessages(visibleRows: [MessageSearchHit]) -> Bool {
        let mayHaveMoreFetchedRows = !expandedSections.contains(.messages)
            && results.messages.count > visibleRows.count
        let mayHaveMoreRemoteRows = expandedSections.contains(.messages)
            && results.messages.count >= Int(messageLimit)
            && messageLimit < UInt32.max
        return mayHaveMoreFetchedRows || mayHaveMoreRemoteRows
    }
}

private struct ChatInputShortcutRow: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let shortcut: ChatInputShortcut

    var body: some View {
        let descriptor = describe(shortcut)
        Button {
            manager.dispatch(descriptor.action)
        } label: {
            HStack(spacing: 12) {
                Image(systemName: descriptor.systemImage)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 36, height: 36)
                    .background(palette.panelAlt)
                    .clipShape(Circle())
                VStack(alignment: .leading, spacing: 2) {
                    Text(descriptor.title)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    Text(descriptor.subtitle)
                        .font(.system(.caption, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("chatListSearchShortcut")
    }

    private func describe(_ shortcut: ChatInputShortcut) -> Descriptor {
        switch shortcut {
        case let .directPeer(_, display, _, _):
            return Descriptor(
                systemImage: "person.crop.circle.badge.plus",
                title: "Start chat",
                subtitle: display,
                action: peerAction(shortcut)
            )
        case let .invite(_, display):
            return Descriptor(
                systemImage: "envelope.open",
                title: "Accept invite",
                subtitle: display,
                action: peerAction(shortcut)
            )
        }
    }

    private func peerAction(_ shortcut: ChatInputShortcut) -> AppAction {
        switch shortcut {
        case let .directPeer(peerInput, _, _, _):
            return .createChat(peerInput: peerInput)
        case let .invite(inviteInput, _):
            return .acceptInvite(inviteInput: inviteInput)
        }
    }

    private struct Descriptor {
        let systemImage: String
        let title: String
        let subtitle: String
        let action: AppAction
    }
}

private struct SearchSectionHeader: View {
    @Environment(\.irisPalette) private var palette
    let title: String

    var body: some View {
        Text(title)
            .font(.system(.caption, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.muted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 16)
            .padding(.bottom, 4)
    }
}

private struct SearchViewMoreRow: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: "chevron.down.circle.fill")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(width: 36, height: 36)
                Text("View more")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("chatListSearchViewMore")
    }
}

private struct MessageSearchHitRow: View {
    let manager: AppManager
    let hit: MessageSearchHit
    let relativeNow: Date
    let preferences: PreferencesSnapshot

    var body: some View {
        IrisChatRow(
            title: hit.chatDisplayName,
            isMuted: false,
            isPinned: false,
            preview: hit.body,
            subtitle: nil,
            timeLabel: irisRelativeTime(hit.createdAtSecs, relativeTo: relativeNow),
            unreadCount: 0,
            pictureUrl: hit.chatPictureUrl,
            preferences: preferences,
            manager: manager,
            onTap: {
                manager.openChatAtMessage(chatId: hit.chatId, messageId: hit.messageId)
            }
        )
        .accessibilityIdentifier("messageHit-\(String(hit.messageId.prefix(12)))")
    }
}

struct InChatSearchTarget: Identifiable, Hashable {
    let chatId: String
    let displayName: String

    var id: String { chatId }
}

private struct InChatSearchButton: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let target: InChatSearchTarget
    @State private var presentedTarget: InChatSearchTarget?

    var body: some View {
        Button {
            presentedTarget = target
        } label: {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .frame(width: 40, height: 40)
                .frame(width: 48, height: 48)
                .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Search in this chat")
        .accessibilityIdentifier("chatHeaderSearchButton")
        .padding(.trailing, 4)
        .sheet(item: $presentedTarget) { target in
            InChatSearchSheet(manager: manager, target: target) {
                presentedTarget = nil
            }
            .irisModalSurface()
#if os(iOS)
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
#elseif os(macOS)
            .frame(minWidth: 420, minHeight: 520)
#endif
            .irisDismissOnMacOutsideClick { presentedTarget = nil }
        }
    }
}

/// Scoped message search bound to a single conversation. Reached from
/// the chat / group-details header magnifying-glass icon. Tapping a
/// hit dismisses the sheet and opens the chat at that conversation.
private struct InChatSearchSheet: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let target: InChatSearchTarget
    let onClose: () -> Void
    @State private var query: String = ""
    // Same query-keyed cache as ChatListScreen so a state push
    // (e.g. an incoming message) doesn't re-run the FTS5 query.
    @State private var cachedResults: SearchResultSnapshot?
    @State private var messageSearchLimit: UInt32 = Self.initialMessageSearchLimit
    @FocusState private var isFocused: Bool

    private static let initialMessageSearchLimit: UInt32 = 50
    private static let messageSearchLimitStep: UInt32 = 50

    private var trimmedQuery: String {
        query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var searchRequestToken: String {
        "\(target.chatId)|\(trimmedQuery)|\(messageSearchLimit)"
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(palette.muted)
                TextField("Search in \(target.displayName)", text: $query)
                    .textFieldStyle(.plain)
                    .autocorrectionDisabled(true)
#if os(iOS)
                    .textInputAutocapitalization(.never)
#endif
                    .focused($isFocused)
                    .accessibilityIdentifier("inChatSearchField")
                if !query.isEmpty {
                    Button {
                        query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(palette.muted)
                    }
                    .buttonStyle(.plain)
                }
                IrisModalCloseButton(action: onClose)
                    .accessibilityIdentifier("inChatSearchCloseButton")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)

            Divider()

            let trimmed = trimmedQuery
            ScrollView {
                if trimmed.isEmpty {
                    Text("Type to search messages in this chat.")
                        .font(.system(.body, design: .rounded))
                        .foregroundStyle(palette.muted)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 48)
                } else if let results = cachedResults,
                          results.query.trimmingCharacters(in: .whitespacesAndNewlines) == trimmed,
                          results.scopeChatId == target.chatId {
                    if results.messages.isEmpty {
                        Text("No matches")
                            .font(.system(.body, design: .rounded))
                            .foregroundStyle(palette.muted)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 48)
                    } else {
                        let preferences = manager.state.preferences
                        let now = Date()
                        LazyVStack(spacing: 0) {
                            ForEach(results.messages, id: \.messageId) { hit in
                                IrisChatRow(
                                    title: hit.chatDisplayName,
                                    isMuted: false,
                                    isPinned: false,
                                    preview: hit.body,
                                    subtitle: nil,
                                    timeLabel: irisRelativeTime(hit.createdAtSecs, relativeTo: now),
                                    unreadCount: 0,
                                    pictureUrl: hit.chatPictureUrl,
                                    preferences: preferences,
                                    manager: manager,
                                    onTap: {
                                        manager.openChatAtMessage(
                                            chatId: hit.chatId,
                                            messageId: hit.messageId
                                        )
                                        onClose()
                                    }
                                )
                                .accessibilityIdentifier("inChatMessageHit-\(String(hit.messageId.prefix(12)))")
                            }
                            if results.messages.count >= Int(messageSearchLimit) {
                                SearchViewMoreRow {
                                    viewMoreMessages()
                                }
                            }
                        }
                    }
                }
            }
        }
        .background(palette.background)
        .onAppear { isFocused = true }
        .irisOnChange(of: trimmedQuery) { _ in
            messageSearchLimit = Self.initialMessageSearchLimit
        }
        .task(id: searchRequestToken) {
            let trimmed = trimmedQuery
            cachedResults = trimmed.isEmpty
                ? nil
                : manager.search(trimmed, scopeChatId: target.chatId, limit: messageSearchLimit)
        }
    }

    private func viewMoreMessages() {
        let nextLimit = messageSearchLimit.addingReportingOverflow(Self.messageSearchLimitStep)
        messageSearchLimit = nextLimit.overflow ? UInt32.max : nextLimit.partialValue
    }
}

private struct NewChatCircleButton: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ZStack(alignment: .trailing) {
                Color.clear
                Image(systemName: "square.and.pencil")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .frame(width: 40, height: 40)
                    .irisGlassSurface(in: Circle())
            }
            .frame(width: 60, height: 48, alignment: .trailing)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .frame(width: 60, height: 48, alignment: .trailing)
        .contentShape(Rectangle())
        .accessibilityLabel("New chat")
        .accessibilityIdentifier("chatListNewChatButton")
    }
}

private struct ChatListRowContainer: View {
    // Plain reference — the parent ChatListScreen already observes `manager`
    // and rebuilds this container with fresh `chat` / `timeLabel` / `preferences`
    // values when state changes. Subscribing here would re-evaluate every row
    // on every manager publish (typing pings, relay events, …), which on a
    // chat list of any size adds up to noticeable CPU + battery drain.
    let manager: AppManager
    let chat: ChatThreadSnapshot
    let timeLabel: String?
    let preferences: PreferencesSnapshot

    @ViewBuilder
    var body: some View {
        let row = chatRow

#if os(macOS)
        row.contextMenu {
            chatListItemContextMenu(manager: manager, chat: chat)
        }
#else
        row
#endif
    }

    private var chatRow: some View {
        let trimmedDraft = chat.draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let preview: String = {
            if chat.isTyping { return "Typing" }
            if !trimmedDraft.isEmpty { return "Draft: \(trimmedDraft)" }
            return chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"
        }()
        return IrisChatRow(
            title: chat.displayName,
            isMuted: chat.isMuted,
            isPinned: chat.isPinned,
            preview: preview,
            draftPreview: chat.isTyping || trimmedDraft.isEmpty ? nil : trimmedDraft,
            subtitle: nil,
            timeLabel: timeLabel,
            unreadCount: chat.unreadCount,
            pictureUrl: chat.pictureUrl,
            preferences: preferences,
            manager: manager,
            onTap: {
                manager.dispatch(.openChat(chatId: chat.chatId))
            }
        )
    }
}

#if os(iOS)
private struct ChatListTableView: UIViewRepresentable {
    @Binding var searchText: String
    let manager: AppManager
    let chats: [ChatThreadSnapshot]
    let preferences: PreferencesSnapshot
    let relativeNow: Date
    let palette: IrisPalette
    let topContentInset: CGFloat
    let isSearchActive: Bool
    let cachedSearchResults: SearchResultSnapshot?
    let expandedSearchSections: Set<ChatListSearchSection>
    let messageLimit: UInt32
    let onOpenNearby: () -> Void
    let onViewMoreSearchResults: (ChatListSearchSection) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> UITableView {
        let tableView = ChatListScrollTableView(frame: .zero, style: .grouped)
        tableView.dataSource = context.coordinator
        tableView.delegate = context.coordinator
        tableView.backgroundColor = .clear
        tableView.separatorStyle = .none
        tableView.rowHeight = UITableView.automaticDimension
        tableView.estimatedRowHeight = 80
        tableView.contentInsetAdjustmentBehavior = .never
        tableView.keyboardDismissMode = .interactive
        tableView.sectionHeaderTopPadding = 0
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: Coordinator.cellReuseIdentifier)
        tableView.accessibilityIdentifier = "chatListTable"
        return tableView
    }

    func updateUIView(_ tableView: UITableView, context: Context) {
        let sections = makeSections()
        let fingerprint = makeFingerprint()
        updateContentInset(in: tableView)
        updateSearchHeader(in: tableView, context: context)
        context.coordinator.manager = manager
        context.coordinator.preferences = preferences
        context.coordinator.relativeNow = relativeNow
        context.coordinator.palette = palette
        context.coordinator.expandedSearchSections = expandedSearchSections
        context.coordinator.messageLimit = messageLimit
        context.coordinator.onOpenNearby = onOpenNearby
        context.coordinator.onViewMoreSearchResults = onViewMoreSearchResults
        context.coordinator.sections = sections
        guard context.coordinator.fingerprint != fingerprint else { return }
        context.coordinator.fingerprint = fingerprint
        tableView.reloadData()
    }

    private func updateContentInset(in tableView: UITableView) {
        let previousTopInset = tableView.contentInset.top
        guard abs(previousTopInset - topContentInset) > 0.5 else { return }

        let wasPinnedToTop = tableView.contentOffset.y <= -previousTopInset + 1
            || (previousTopInset == 0 && abs(tableView.contentOffset.y) <= 1)

        var contentInset = tableView.contentInset
        contentInset.top = topContentInset
        tableView.contentInset = contentInset

        var indicatorInsets = tableView.verticalScrollIndicatorInsets
        indicatorInsets.top = topContentInset
        tableView.verticalScrollIndicatorInsets = indicatorInsets

        if wasPinnedToTop {
            tableView.contentOffset.y = -topContentInset
        }
    }

    private func updateSearchHeader(in tableView: UITableView, context: Context) {
        let rootView = AnyView(
            ChatListSearchField(text: $searchText)
                .environment(\.irisPalette, palette)
        )
        let controller: UIHostingController<AnyView>
        if let existing = context.coordinator.searchHeaderController {
            existing.rootView = rootView
            controller = existing
        } else {
            let created = UIHostingController(rootView: rootView)
            created.view.backgroundColor = .clear
            context.coordinator.searchHeaderController = created
            controller = created
        }

        let targetSize = CGSize(
            width: max(tableView.bounds.width, 1),
            height: UIView.layoutFittingCompressedSize.height
        )
        let fittingSize = controller.view.systemLayoutSizeFitting(
            targetSize,
            withHorizontalFittingPriority: .required,
            verticalFittingPriority: .fittingSizeLevel
        )
        let headerHeight = ceil(fittingSize.height)
        var frame = controller.view.frame
        frame.size = CGSize(width: tableView.bounds.width, height: headerHeight)
        controller.view.frame = frame

        if tableView.tableHeaderView !== controller.view ||
            abs((tableView.tableHeaderView?.frame.height ?? 0) - headerHeight) > 0.5 {
            tableView.tableHeaderView = controller.view
        }
    }

    private func makeSections() -> [Section] {
        if isSearchActive {
            return [
                Section(
                    title: nil,
                    items: cachedSearchResults.map { [.searchResults($0)] } ?? []
                )
            ]
        }

        if chats.isEmpty {
            return [
                Section(title: nil, items: [.nearby, .empty])
            ]
        }

        let pinnedChats = chats.filter(\.isPinned)
        guard !pinnedChats.isEmpty else {
            return [
                Section(title: nil, items: [.nearby] + chats.map(Item.chat))
            ]
        }

        let unpinnedChats = chats.filter { !$0.isPinned }
        return [
            Section(title: "Pinned", items: pinnedChats.map(Item.chat)),
            Section(title: "Chats", items: [.nearby] + unpinnedChats.map(Item.chat)),
        ]
    }

    private func makeFingerprint() -> [String] {
        let expanded = expandedSearchSections.map(\.rawValue).sorted().joined(separator: ",")
        var values = [
            "search:\(isSearchActive):\(searchText):\(messageLimit):\(expanded)",
            "nearby:\(manager.nearbyIris.sidebarSubtitle):\(manager.nearbyIris.peers.count)",
        ]
        if let cachedSearchResults {
            values.append(
                "results:\(cachedSearchResults.query):" +
                    "\(cachedSearchResults.contacts.count):" +
                    "\(cachedSearchResults.groups.count):" +
                    "\(cachedSearchResults.messages.count):" +
                    "\(cachedSearchResults.shortcut == nil)"
            )
        }
        values.append("hasPinned:\(chats.contains { $0.isPinned })")
        if chats.isEmpty {
            values.append("empty")
        } else {
            for chat in chats {
                var chatValues: [String] = []
                chatValues.append(chat.chatId)
                chatValues.append(chat.displayName)
                chatValues.append(chat.lastMessagePreview ?? "")
                chatValues.append(chat.subtitle ?? "")
                chatValues.append(chat.draft)
                chatValues.append(chat.lastMessageAtSecs.map(String.init) ?? "")
                chatValues.append(String(chat.unreadCount))
                chatValues.append(String(chat.isMuted))
                chatValues.append(String(chat.isPinned))
                chatValues.append(String(chat.isTyping))
                chatValues.append(chat.pictureUrl ?? "")
                values.append(chatValues.joined(separator: "\u{1F}"))
            }
        }
        return values
    }

    enum Item {
        case nearby
        case empty
        case searchResults(SearchResultSnapshot)
        case chat(ChatThreadSnapshot)
    }

    struct Section {
        let title: String?
        let items: [Item]
    }

    final class Coordinator: NSObject, UITableViewDataSource, UITableViewDelegate {
        static let cellReuseIdentifier = "ChatListTableCell"

        weak var manager: AppManager?
        var searchHeaderController: UIHostingController<AnyView>?
        var sections: [Section] = []
        var preferences: PreferencesSnapshot?
        var relativeNow = Date()
        var palette = IrisPalette.light
        var expandedSearchSections: Set<ChatListSearchSection> = []
        var messageLimit: UInt32 = 0
        var onOpenNearby: (() -> Void)?
        var onViewMoreSearchResults: ((ChatListSearchSection) -> Void)?
        var fingerprint: [String] = []

        func numberOfSections(in tableView: UITableView) -> Int {
            sections.count
        }

        func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
            sections[section].items.count
        }

        func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
            let cell = tableView.dequeueReusableCell(withIdentifier: Self.cellReuseIdentifier, for: indexPath)
            cell.backgroundColor = .clear
            cell.selectedBackgroundView = UIView()
            cell.accessoryType = .none
            cell.isAccessibilityElement = true
            cell.accessibilityTraits = []

            switch item(at: indexPath) {
            case .nearby:
                configureNearby(cell)
            case .empty:
                configureEmpty(cell)
            case let .searchResults(results):
                configureSearchResults(cell, results: results)
            case let .chat(chat):
                configureChat(cell, chat: chat)
            }

            return cell
        }

        func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
            tableView.deselectRow(at: indexPath, animated: true)
            switch item(at: indexPath) {
            case .nearby:
                onOpenNearby?()
            case .empty, .searchResults:
                break
            case let .chat(chat):
                manager?.dispatch(.openChat(chatId: chat.chatId))
            }
        }

        func tableView(
            _ tableView: UITableView,
            leadingSwipeActionsConfigurationForRowAt indexPath: IndexPath
        ) -> UISwipeActionsConfiguration? {
            guard case let .chat(chat) = item(at: indexPath) else { return nil }
            let configuration = UISwipeActionsConfiguration(actions: [
                contextualAction(
                    accessibilityTitle: chat.unreadCount > 0 ? "Read" : "Unread",
                    systemImage: chat.unreadCount > 0 ? "envelope.open.fill" : "envelope.badge.fill",
                    color: .signalUltramarine
                ) { [weak self] in
                    self?.manager?.dispatch(.setChatUnread(chatId: chat.chatId, unread: chat.unreadCount == 0))
                },
                contextualAction(
                    accessibilityTitle: chat.isPinned ? "Unpin" : "Pin",
                    systemImage: chat.isPinned ? "pin.slash.fill" : "pin.fill",
                    color: .signalPinOrange
                ) { [weak self] in
                    self?.manager?.dispatch(.setChatPinned(chatId: chat.chatId, pinned: !chat.isPinned))
                },
            ])
            configuration.performsFirstActionWithFullSwipe = false
            return configuration
        }

        func tableView(
            _ tableView: UITableView,
            trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath
        ) -> UISwipeActionsConfiguration? {
            guard case let .chat(chat) = item(at: indexPath) else { return nil }
            let configuration = UISwipeActionsConfiguration(actions: [
                deleteAction(chat: chat, presentingFrom: tableView),
                contextualAction(
                    accessibilityTitle: chat.isMuted ? "Unmute" : "Mute",
                    systemImage: chat.isMuted ? "bell.fill" : "bell.slash.fill",
                    color: .signalIndigo
                ) { [weak self] in
                    self?.manager?.dispatch(.setChatMuted(chatId: chat.chatId, muted: !chat.isMuted))
                },
            ])
            configuration.performsFirstActionWithFullSwipe = false
            return configuration
        }

        func tableView(_ tableView: UITableView, canEditRowAt indexPath: IndexPath) -> Bool {
            if case .chat = item(at: indexPath) {
                return true
            }
            return false
        }

        func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
            sections[section].title == nil ? .leastNormalMagnitude : UITableView.automaticDimension
        }

        func tableView(_ tableView: UITableView, heightForFooterInSection section: Int) -> CGFloat {
            .leastNormalMagnitude
        }

        func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
            guard let title = sections[section].title else { return UIView() }

            let container = UIView()
            container.backgroundColor = .clear
            let topMargin: CGFloat = section == 0 ? 14 : 6
            container.layoutMargins = UIEdgeInsets(top: topMargin, left: 16, bottom: 8, right: 16)

            let label = UILabel()
            label.translatesAutoresizingMaskIntoConstraints = false
            label.font = .preferredFont(forTextStyle: .headline)
            label.textColor = UIColor(palette.textPrimary)
            label.text = title
            label.adjustsFontForContentSizeCategory = true
            label.accessibilityTraits.insert(.header)

            container.addSubview(label)
            NSLayoutConstraint.activate([
                label.leadingAnchor.constraint(equalTo: container.layoutMarginsGuide.leadingAnchor),
                label.trailingAnchor.constraint(equalTo: container.layoutMarginsGuide.trailingAnchor),
                label.topAnchor.constraint(equalTo: container.layoutMarginsGuide.topAnchor),
                label.bottomAnchor.constraint(equalTo: container.layoutMarginsGuide.bottomAnchor),
            ])
            return container
        }

        func tableView(_ tableView: UITableView, viewForFooterInSection section: Int) -> UIView? {
            UIView()
        }

        private func item(at indexPath: IndexPath) -> Item {
            sections[indexPath.section].items[indexPath.row]
        }

        private func configureNearby(_ cell: UITableViewCell) {
            guard let manager else { return }
            cell.accessibilityIdentifier = "nearbyChatRow"
            cell.accessibilityLabel = "Nearby, \(manager.nearbyIris.sidebarSubtitle)"
            cell.accessibilityTraits = [.button]
            cell.selectionStyle = .default
            cell.contentConfiguration = UIHostingConfiguration {
                ChatListTableRowContent(
                    title: "Nearby",
                    preview: manager.nearbyIris.sidebarSubtitle,
                    subtitle: nil,
                    timeLabel: nil,
                    unreadCount: 0,
                    preferences: manager.state.preferences,
                    manager: manager,
                    leading: AnyView(NearbyWirelessAvatar()),
                    previewLeading: nearbyPreviewLeading(service: manager.nearbyIris, manager: manager)
                )
                .accessibilityHidden(true)
                .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func configureEmpty(_ cell: UITableViewCell) {
            cell.accessibilityIdentifier = "chatListEmpty"
            cell.accessibilityLabel = "No chats yet"
            cell.accessibilityTraits = [.staticText]
            cell.selectionStyle = .none
            cell.contentConfiguration = UIHostingConfiguration {
                Text("No chats yet")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
                    .accessibilityHidden(true)
                    .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func configureSearchResults(_ cell: UITableViewCell, results: SearchResultSnapshot) {
            guard let manager else { return }
            cell.accessibilityIdentifier = "chatListSearchResults"
            cell.accessibilityLabel = "Search results"
            cell.accessibilityTraits = [.staticText]
            cell.selectionStyle = .none
            cell.contentConfiguration = UIHostingConfiguration {
                SearchResultsList(
                    manager: manager,
                    results: results,
                    relativeNow: relativeNow,
                    expandedSections: expandedSearchSections,
                    messageLimit: messageLimit,
                    onViewMore: { [weak self] section in
                        self?.onViewMoreSearchResults?(section)
                    }
                )
                .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func configureChat(_ cell: UITableViewCell, chat: ChatThreadSnapshot) {
            guard let manager else { return }
            let preview = chatListPreview(for: chat)
            let timeLabel = irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow)
            cell.accessibilityIdentifier = "chatRow-\(String(chat.chatId.prefix(12)))"
            cell.accessibilityLabel = [chat.displayName, preview, timeLabel].compactMap { $0 }.joined(separator: ", ")
            cell.accessibilityTraits = [.button]
            cell.selectionStyle = .default
            cell.contentConfiguration = UIHostingConfiguration {
                ChatListTableRowContent(
                    title: chat.displayName,
                    isMuted: chat.isMuted,
                    isPinned: chat.isPinned,
                    preview: preview,
                    draftPreview: chat.isTyping ? nil : trimmedDraftPreview(for: chat),
                    subtitle: nil,
                    timeLabel: timeLabel,
                    unreadCount: chat.unreadCount,
                    pictureUrl: chat.pictureUrl,
                    preferences: preferences,
                    manager: manager
                )
                .accessibilityHidden(true)
                .environment(\.irisPalette, palette)
            }
            .margins(.all, 0)
        }

        private func nearbyPreviewLeading(service: IrisNearbyService, manager: AppManager) -> AnyView? {
            guard !service.peers.isEmpty else { return nil }
            return AnyView(
                NearbyAvatarStack(
                    peers: Array(service.peers.prefix(3)),
                    preferences: manager.state.preferences,
                    manager: manager
                )
            )
        }

        private func contextualAction(
            accessibilityTitle: String,
            systemImage: String,
            style: UIContextualAction.Style = .normal,
            color: UIColor,
            handler: @escaping () -> Void
        ) -> UIContextualAction {
            let action = UIContextualAction(style: style, title: "") { _, _, completion in
                handler()
                completion(true)
            }
            action.backgroundColor = color
            action.image = UIImage(systemName: systemImage)
            action.accessibilityLabel = accessibilityTitle
            return action
        }

        private func deleteAction(chat: ChatThreadSnapshot, presentingFrom tableView: UITableView) -> UIContextualAction {
            let action = UIContextualAction(style: .destructive, title: "") { [weak self, weak tableView] _, _, completion in
                guard let self, let tableView else {
                    completion(false)
                    return
                }
                let alert = UIAlertController(title: "Delete chat?", message: nil, preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in
                    completion(false)
                })
                alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { _ in
                    self.manager?.dispatch(.deleteChat(chatId: chat.chatId))
                    completion(true)
                })
                guard let presenter = tableView.window?.rootViewController else {
                    completion(false)
                    return
                }
                presenter.present(alert, animated: true)
            }
            action.backgroundColor = .signalRed
            action.image = UIImage(systemName: "trash.fill")
            action.accessibilityLabel = "Delete"
            return action
        }

        private func chatListPreview(for chat: ChatThreadSnapshot) -> String {
            let trimmedDraft = trimmedDraftPreview(for: chat) ?? ""
            if chat.isTyping { return "Typing" }
            if !trimmedDraft.isEmpty { return "Draft: \(trimmedDraft)" }
            return chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"
        }

        private func trimmedDraftPreview(for chat: ChatThreadSnapshot) -> String? {
            let trimmedDraft = chat.draft.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmedDraft.isEmpty ? nil : trimmedDraft
        }
    }
}

private final class ChatListScrollTableView: UITableView {
    private let fillerFooterView = UIView()

    override init(frame: CGRect, style: UITableView.Style) {
        super.init(frame: frame, style: style)
        tableFooterView = fillerFooterView
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        updateFooterHeight()
    }

    override func adjustedContentInsetDidChange() {
        super.adjustedContentInsetDidChange()
        updateFooterHeight()
    }

    private func updateFooterHeight() {
        let headerHeight = tableHeaderView?.frame.height ?? 0
        let visibleHeight = max(0, frame.inset(by: adjustedContentInset).height)
        var availableHeight = max(0, visibleHeight - headerHeight)

        for section in 0..<numberOfSections where availableHeight > 0 {
            availableHeight = max(0, availableHeight - rect(forSection: section).height)
        }

        let displayScale = window?.windowScene?.screen.scale ?? UIScreen.main.scale
        let footerHeight = availableHeight + 1 / displayScale
        guard abs(fillerFooterView.frame.height - footerHeight) > 0.5 else { return }

        var footerFrame = fillerFooterView.frame
        footerFrame.size.height = footerHeight
        fillerFooterView.frame = footerFrame
        tableFooterView = fillerFooterView
    }
}

private struct ChatListTableRowContent: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let isMuted: Bool
    let isPinned: Bool
    let preview: String
    let draftPreview: String?
    let subtitle: String?
    let timeLabel: String?
    let unreadCount: UInt64
    let pictureUrl: String?
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    let leading: AnyView?
    let previewLeading: AnyView?

    init(
        title: String,
        isMuted: Bool = false,
        isPinned: Bool = false,
        preview: String,
        draftPreview: String? = nil,
        subtitle: String?,
        timeLabel: String?,
        unreadCount: UInt64,
        pictureUrl: String? = nil,
        preferences: PreferencesSnapshot? = nil,
        manager: AppManager? = nil,
        leading: AnyView? = nil,
        previewLeading: AnyView? = nil
    ) {
        self.title = title
        self.isMuted = isMuted
        self.isPinned = isPinned
        self.preview = preview
        self.draftPreview = draftPreview
        self.subtitle = subtitle
        self.timeLabel = timeLabel
        self.unreadCount = unreadCount
        self.pictureUrl = pictureUrl
        self.preferences = preferences
        self.manager = manager
        self.leading = leading
        self.previewLeading = previewLeading
    }

    var body: some View {
        HStack(alignment: .center, spacing: IrisChatListRowMetrics.avatarTextSpacing) {
            if let leading {
                leading
            } else {
                IrisAvatar(
                    label: title,
                    size: IrisChatListRowMetrics.avatarSize,
                    emphasize: unreadCount > 0,
                    pictureUrl: pictureUrl,
                    preferences: preferences,
                    manager: manager
                )
            }

            VStack(alignment: .leading, spacing: IrisChatListRowMetrics.textStackSpacing) {
                HStack(alignment: .firstTextBaseline, spacing: IrisChatListRowMetrics.textRowSpacing) {
                    HStack(alignment: .firstTextBaseline, spacing: IrisChatListRowMetrics.titleAccessorySpacing) {
                        Text(title)
                            .font(.headline)
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)

                        if isMuted {
                            Image(systemName: "bell.slash.fill")
                                .font(.system(size: IrisChatListRowMetrics.muteIconSize, weight: .semibold))
                                .foregroundStyle(palette.muted)
                                .accessibilityLabel("muted")
                        }

                    }
                    .layoutPriority(1)

                    Spacer(minLength: 8)

                    if let timeLabel, !timeLabel.isEmpty {
                        Text(timeLabel)
                            .font(.subheadline)
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }

                HStack(alignment: .center, spacing: IrisChatListRowMetrics.textRowSpacing) {
                    if let previewLeading {
                        previewLeading
                    }
                    previewText
                        .font(.subheadline)
                        .foregroundStyle(palette.muted)
                        .lineLimit(previewLeading == nil ? 2 : 1, reservesSpace: previewLeading == nil)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .layoutPriority(1)

                    unreadBadge
                }

                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }
            }
        }
        .padding(.horizontal, IrisChatListRowMetrics.horizontalPadding)
        .padding(.vertical, IrisChatListRowMetrics.verticalPadding)
    }

    private var previewText: Text {
        if let draftPreview {
            return Text("Draft: ").italic() + Text(draftPreview)
        }
        return Text(preview)
    }

    @ViewBuilder
    private var unreadBadge: some View {
        IrisUnreadBadge(count: unreadCount)
    }
}

private extension UIColor {
    static var signalUltramarine: UIColor {
        UIColor { traits in
            if traits.accessibilityContrast == .high {
                return traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0x5D / 255, green: 0x92 / 255, blue: 0xFF / 255, alpha: 1)
                    : UIColor(red: 0x0A / 255, green: 0x43 / 255, blue: 0xB9 / 255, alpha: 1)
            }
            return traits.userInterfaceStyle == .dark
                ? UIColor(red: 0x2D / 255, green: 0x70 / 255, blue: 0xFA / 255, alpha: 1)
                : UIColor(red: 0x22 / 255, green: 0x67 / 255, blue: 0xF5 / 255, alpha: 1)
        }
    }

    static var signalIndigo: UIColor {
        UIColor { traits in
            if traits.accessibilityContrast == .high {
                return traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0x7D / 255, green: 0x7A / 255, blue: 0xFF / 255, alpha: 1)
                    : UIColor(red: 0x36 / 255, green: 0x34 / 255, blue: 0xA3 / 255, alpha: 1)
            }
            return traits.userInterfaceStyle == .dark
                ? UIColor(red: 0x5E / 255, green: 0x5C / 255, blue: 0xE6 / 255, alpha: 1)
                : UIColor(red: 0x58 / 255, green: 0x56 / 255, blue: 0xD6 / 255, alpha: 1)
        }
    }

    static var signalRed: UIColor {
        UIColor { traits in
            if traits.accessibilityContrast == .high {
                return traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0xFF / 255, green: 0x69 / 255, blue: 0x61 / 255, alpha: 1)
                    : UIColor(red: 0xD7 / 255, green: 0x00 / 255, blue: 0x15 / 255, alpha: 1)
            }
            return traits.userInterfaceStyle == .dark
                ? UIColor(red: 0xFF / 255, green: 0x45 / 255, blue: 0x3A / 255, alpha: 1)
                : UIColor(red: 0xFF / 255, green: 0x3B / 255, blue: 0x30 / 255, alpha: 1)
        }
    }

    static var signalPinOrange: UIColor {
        UIColor(red: 0xFF / 255, green: 0x99 / 255, blue: 0x0A / 255, alpha: 1)
    }
}
#endif

#if os(iOS) || os(macOS)
private struct NearbyChatListRow: View {
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService
    let onOpen: () -> Void

    var body: some View {
        IrisChatRow(
            title: "Nearby",
            preview: service.sidebarSubtitle,
            subtitle: nil,
            timeLabel: nil,
            unreadCount: 0,
            pictureUrl: nil,
            preferences: manager.state.preferences,
            manager: manager,
            leading: AnyView(NearbyWirelessAvatar()),
            previewLeading: previewLeading,
            onTap: {
                onOpen()
            }
        )
        .accessibilityIdentifier("nearbyChatRow")
    }

    private var previewLeading: AnyView? {
        guard !service.peers.isEmpty else { return nil }
        return AnyView(
            NearbyAvatarStack(
                peers: Array(service.peers.prefix(3)),
                preferences: manager.state.preferences,
                manager: manager
            )
        )
    }
}

private struct NearbyWirelessAvatar: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        ZStack {
            Circle().fill(palette.panelAlt)
            Circle().stroke(palette.border, lineWidth: 1)
            Image(systemName: "dot.radiowaves.left.and.right")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
        }
        .frame(width: IrisChatListRowMetrics.avatarSize, height: IrisChatListRowMetrics.avatarSize)
    }
}

private struct NearbyAvatarStack: View {
    @Environment(\.irisPalette) private var palette
    let peers: [IrisNearbyPeer]
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    // Stays smaller than the subheadline line height so the row's preview
    // HStack doesn't grow taller when the stack appears.
    var avatarSize: CGFloat = 16

    private var stride: CGFloat { avatarSize - 6 }

    var body: some View {
        ZStack(alignment: .leading) {
            ForEach(Array(peers.enumerated()), id: \.element.id) { index, peer in
                IrisAvatar(
                    label: peer.name.isEmpty ? "?" : peer.name,
                    size: avatarSize,
                    pictureUrl: peer.pictureURL,
                    preferences: preferences,
                    manager: manager
                )
                .overlay(Circle().stroke(palette.background, lineWidth: 1.5))
                .offset(x: CGFloat(index) * stride)
            }
        }
        .frame(width: stackWidth, height: avatarSize, alignment: .leading)
    }

    private var stackWidth: CGFloat {
        guard !peers.isEmpty else { return avatarSize }
        return CGFloat(peers.count - 1) * stride + avatarSize
    }
}

private struct NearbyIrisScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService
    let onClose: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            header

            Rectangle()
                .fill(palette.border)
                .frame(height: 1)

            transportControls

            Spacer(minLength: 0)
        }
        .background(palette.background)
        .irisModalSurface()
    }

    private var header: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Nearby")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
            }

            Spacer()

            IrisModalCloseButton(action: onClose)
                .accessibilityIdentifier("nearbyCloseButton")
        }
        .padding(.horizontal, 18)
        .frame(height: 58)
        .background(palette.toolbar)
    }

    private var transportControls: some View {
        VStack(spacing: 0) {
            transportRow(
                title: "Bluetooth",
                subtitle: service.bluetoothTransportWarning,
                peers: service.bluetoothPeers,
                isOn: bluetoothBinding,
                accessibilityID: "nearbyBluetoothSwitch"
            )

            Rectangle()
                .fill(palette.border)
                .frame(height: 1)
                .padding(.leading, 18)

            transportRow(
                title: "Wi-Fi",
                subtitle: service.lanTransportWarning,
                peers: service.lanPeers,
                isOn: lanBinding,
                accessibilityID: "nearbyLanSwitch"
            )
        }
        .background(palette.panel)
    }

    private func transportRow(
        title: String,
        subtitle: String?,
        peers: [IrisNearbyPeer],
        isOn: Binding<Bool>,
        accessibilityID: String
    ) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.system(.body, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                    if let subtitle {
                        Text(subtitle)
                            .font(.system(.caption, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.muted)
                            .lineLimit(1)
                    }
                }
                Spacer()
                Toggle("", isOn: isOn)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .irisControlTint()
                    .accessibilityIdentifier(accessibilityID)
            }
            .frame(height: 52)

            if isOn.wrappedValue {
                if peers.isEmpty, subtitle == nil {
                    Text("No users nearby")
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.bottom, 12)
                } else if !peers.isEmpty {
                    peerStrip(peers)
                }
                if let mailbag = service.mailbagSummary {
                    Text("Mailbag · \(mailbag)")
                        .font(.system(.caption2, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.muted)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.bottom, 10)
                }
            }
        }
        .padding(.horizontal, 18)
    }

    private var bluetoothBinding: Binding<Bool> {
        Binding(
            get: { service.isVisible },
            set: { manager.setNearbyBluetoothEnabled($0) }
        )
    }

    private var lanBinding: Binding<Bool> {
        Binding(
            get: { service.isLanVisible },
            set: { manager.setNearbyLanEnabled($0) }
        )
    }

    @ViewBuilder
    private func peerStrip(_ peers: [IrisNearbyPeer]) -> some View {
        if !peers.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 12) {
                    ForEach(peers) { peer in
                        Button {
                            openPeer(peer)
                        } label: {
                            VStack(spacing: 6) {
                                IrisAvatar(
                                    label: peer.name,
                                    size: 42,
                                    pictureUrl: peer.pictureURL,
                                    preferences: manager.state.preferences,
                                    manager: manager
                                )
                                Text(peer.name)
                                    .font(.system(.caption, design: .rounded, weight: .semibold))
                                    .foregroundStyle(palette.textPrimary)
                                    .lineLimit(1)
                                    .frame(maxWidth: 78)
                            }
                        }
                        .buttonStyle(.irisPlain)
                        .disabled(peer.ownerPubkeyHex == nil)
                        .accessibilityIdentifier("nearbyPeer-\(String(peer.id.prefix(12)))")
                    }
                }
                .padding(.horizontal, 0)
                .padding(.vertical, 10)
            }
        }
    }

    private func openPeer(_ peer: IrisNearbyPeer) {
        guard let ownerPubkeyHex = peer.ownerPubkeyHex else { return }
        manager.dispatch(.createChat(peerInput: ownerPubkeyHex))
        onClose()
    }
}
#endif

struct NewChatScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @State private var peerInput = ""
    @State private var submittedInput: String?
    @State private var showingScanner = false
    @State private var showingInviteQr = false

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
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                handleNewChatInput(code)
                showingScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
        #if os(macOS)
        .overlay { inviteQrOverlay }
        #else
        .sheet(isPresented: $showingInviteQr) {
            inviteQrSheet
                .irisModalSurface()
        }
        #endif
        .irisOnChange(of: peerInput) { _ in
            autoProceedIfReady()
        }
        .task {
            if manager.state.publicInvite == nil && !manager.state.busy.creatingInvite {
                manager.dispatch(.createPublicInvite)
            }
        }
    }

    #if os(macOS)
    @ViewBuilder
    private var inviteQrOverlay: some View {
        if showingInviteQr {
            ZStack {
                Color.black.opacity(0.45)
                    .ignoresSafeArea()
                    .contentShape(Rectangle())
                    .onTapGesture { showingInviteQr = false }
                inviteQrSheet
                    .background(palette.background)
                    .clipShape(RoundedRectangle(cornerRadius: 16))
                    .overlay(
                        RoundedRectangle(cornerRadius: 16)
                            .strokeBorder(palette.border, lineWidth: 1)
                    )
                    .shadow(radius: 22)
                    .frame(maxWidth: 420)
                    .padding(40)
                    .contentShape(Rectangle())
                    .onTapGesture {}
                    .irisOnEscapeKey { showingInviteQr = false }
            }
        }
    }
    #endif

    private var newChatCard: some View {
        IrisSectionCard {
            Text("New Chat")
                .font(.system(.title2, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)
                .frame(maxWidth: .infinity, alignment: .center)

            if let invite = manager.state.publicInvite {
                Text("Share an invite to start a chat")
                    .font(.system(.footnote, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity, alignment: .center)

                HStack(spacing: 10) {
                    Button {
                        manager.copyToClipboard(invite.url)
                    } label: {
                        NewChatInviteActionLabel(systemImage: "doc.on.doc", title: "Copy")
                    }
                    .frame(maxWidth: .infinity)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("newChatInviteCopyButton")

                    ShareLink(item: invite.url) {
                        NewChatInviteActionLabel(systemImage: "square.and.arrow.up", title: "Share")
                    }
                    .frame(maxWidth: .infinity)
                    .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                    .accessibilityIdentifier("newChatInviteShareButton")

                    Button(action: { showingInviteQr = true }) {
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

            TextField("Paste invite or user id", text: $peerInput)
                .irisIdentifierInputModifiers()
                .textFieldStyle(.plain)
                .irisInputField()
                .accessibilityIdentifier("newChatPeerInput")

            if irisSupportsQrScanning {
                Button(action: { showingScanner = true }) {
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

    @ViewBuilder
    private var inviteQrSheet: some View {
        if let invite = manager.state.publicInvite {
            VStack(spacing: 18) {
                ZStack {
                    Text("Invite code")
                        .font(.system(.title3, design: .rounded, weight: .bold))
                        .foregroundStyle(palette.textPrimary)
                        .frame(maxWidth: .infinity)
                    HStack {
                        Spacer()
                        IrisModalCloseButton {
                            showingInviteQr = false
                        }
                        .accessibilityIdentifier("newChatInviteQrCloseButton")
                    }
                }

                QrCodeImage(text: invite.url)
                    .frame(maxWidth: 320)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .accessibilityIdentifier("newChatInviteQrCode")

                Text("Scan this code to start a chat")
                    .font(.system(.footnote, design: .rounded))
                    .foregroundStyle(palette.muted)

                Button("Copy") { manager.copyToClipboard(invite.url) }
                    .buttonStyle(IrisSecondaryButtonStyle())
            }
            .padding(24)
        } else {
            ProgressView()
                .padding(40)
        }
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

    private func handleNewChatInput(_ raw: String) {
        let normalized = normalizePeerInput(input: raw)
        if !normalized.isEmpty, isValidPeerInput(input: normalized) {
            peerInput = normalized
            submittedInput = normalized
            manager.dispatch(.createChat(peerInput: normalized))
            return
        }

        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            peerInput = trimmed
            submittedInput = trimmed
            manager.dispatch(.acceptInvite(inviteInput: trimmed))
        }
    }
}

private struct NewChatInviteActionLabel: View {
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

private func shouldAutoSubmitSecret(previous: String, current: String) -> Bool {
    guard !current.isEmpty else {
        return false
    }
    let pasted = current.count > previous.count + 4
    let lower = current.lowercased()
    if lower.hasPrefix("nsec1") {
        return pasted || current.count >= 63
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

struct NewGroupScreen: View {
    private enum Step: Equatable {
        case members
        case details
    }

    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    @State private var step: Step = .members
    @State private var name = ""
    @State private var memberInput = ""
    @State private var selectedOwners = Set<String>()
    @State private var showingScanner = false
    @State private var showingGroupPicturePicker = false
    @State private var groupPhoto: StagedAttachment?
    @FocusState private var isNameFocused: Bool

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

    private var filteredKnownChats: [ChatThreadSnapshot] {
        existingDirectChats.filteredByQuery(memberInput)
    }

    private var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
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
            Color.clear
                .frame(height: 0)
                .accessibilityIdentifier("newGroupPrimaryCard")

            if step == .members {
                memberSelectionStep
            } else {
                groupDetailsStep
            }
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                addMember(code)
                showingScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
        .fileImporter(
            isPresented: $showingGroupPicturePicker,
            allowedContentTypes: [.image],
            allowsMultipleSelection: false
        ) { result in
            guard case let .success(urls) = result, let url = urls.first else {
                return
            }
            groupPhoto = manager.stageGroupPicture(fileURL: url)
        }
        .irisOnChange(of: step) { nextStep in
            if nextStep == .details {
                DispatchQueue.main.async {
                    isNameFocused = true
                }
            }
        }
    }

    private var memberSelectionStep: some View {
        Group {
            IrisSectionCard(accent: true) {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("newGroupMemberStep")

                CardHeader(title: "Select members")

                TextField("Search or paste user ID", text: $memberInput)
                    .irisIdentifierInputModifiers()
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .accessibilityIdentifier("newGroupMemberInput")

                VStack(spacing: 10) {
                    pasteMemberButton
                    scanMemberButton
                    addMemberButton
                }

                selectedMembersChips
            }

            if !filteredKnownChats.isEmpty {
                knownUsersCard
            }

            Button(selectedOwners.isEmpty ? "Next" : "Next (\(selectedOwners.count))") {
                step = .details
            }
            .buttonStyle(IrisPrimaryButtonStyle())
            .accessibilityIdentifier("newGroupNextButton")
        }
    }

    private var groupDetailsStep: some View {
        Group {
            IrisSectionCard(accent: true) {
                Color.clear
                    .frame(height: 0)
                    .accessibilityIdentifier("newGroupDetailsStep")

                CardHeader(title: "Group details")

                HStack(spacing: 12) {
                    IrisAvatar(label: name.isEmpty ? "Group" : name, size: 56, emphasize: true)

                    VStack(alignment: .leading, spacing: 8) {
                        Button(groupPhoto == nil ? "Photo" : "Change photo") {
                            showingGroupPicturePicker = true
                        }
                        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                        .accessibilityIdentifier("newGroupPhotoButton")

                        if let groupPhoto {
                            HStack(spacing: 8) {
                                Text(groupPhoto.filename)
                                    .font(.system(.footnote, design: .rounded))
                                    .foregroundStyle(palette.muted)
                                    .lineLimit(1)

                                Button("Remove") {
                                    self.groupPhoto = nil
                                }
                                .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                                .accessibilityIdentifier("newGroupRemovePhotoButton")
                            }
                        }
                    }
                }

                TextField("Group name", text: $name)
                    .textFieldStyle(.plain)
                    .irisInputField()
                    .focused($isNameFocused)
                    .accessibilityIdentifier("newGroupNameInput")

                selectedMembersChips
            }

            HStack(spacing: 10) {
                Button("Back") {
                    step = .members
                }
                .buttonStyle(IrisSecondaryButtonStyle())

                Button(manager.state.busy.creatingGroup ? "Creating…" : "Create group") {
                    manager.createGroup(
                        name: name,
                        memberInputs: selectedOwners.sorted(),
                        picture: groupPhoto
                    )
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .disabled(!canCreate)
                .accessibilityIdentifier("newGroupCreateButton")
            }
        }
    }

    private var knownUsersCard: some View {
        IrisSectionCard {
            CardHeader(title: memberInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Known users" : "Search results")

            ForEach(Array(filteredKnownChats.enumerated()), id: \.element.chatId) { index, chat in
                Button {
                    if selectedOwners.contains(chat.chatId) {
                        selectedOwners.remove(chat.chatId)
                    } else {
                        selectedOwners.insert(chat.chatId)
                    }
                    memberInput = ""
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
                            .foregroundStyle(selectedOwners.contains(chat.chatId) ? palette.textPrimary : palette.muted)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.irisPlain)

                if index < filteredKnownChats.count - 1 {
                    Divider().overlay(palette.border)
                }
            }
        }
    }

    private var selectedMembersChips: some View {
        Group {
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
                Button("Scan code") { showingScanner = true }
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

struct GroupDetailsScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let groupId: String

    @State private var groupName = ""
    @State private var memberInput = ""
    @State private var showingScanner = false
    @State private var showingGroupPicturePicker = false
    @State private var groupPictureViewerItem: IrisProfilePictureViewerItem?

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

                    HStack(spacing: 14) {
                        groupAvatar(details)
                        if details.canManage {
                            Button(manager.state.busy.uploadingAttachment ? "Uploading…" : "Change photo") {
                                showingGroupPicturePicker = true
                            }
                            .buttonStyle(IrisSecondaryButtonStyle(compact: true))
                            .disabled(manager.state.busy.uploadingAttachment)
                            .accessibilityIdentifier("groupDetailsChangePhotoButton")
                        }
                    }

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
                        title: "Disappearing messages",
                        subtitle: "Messages auto-delete after the chosen interval."
                    )
                    let chatId = "group:\(groupId)"
                    let currentTtl = manager.state.currentChat?.chatId == chatId
                        ? manager.state.currentChat?.messageTtlSeconds
                        : nil
                    VStack(spacing: 0) {
                        ForEach(disappearingMessageOptions, id: \.0) { label, ttlSeconds in
                            Button {
                                manager.dispatch(.setChatMessageTtl(chatId: chatId, ttlSeconds: ttlSeconds))
                            } label: {
                                HStack {
                                    Text(label)
                                        .foregroundStyle(palette.textPrimary)
                                    Spacer()
                                    if currentTtl == ttlSeconds {
                                        Image(systemName: "checkmark")
                                            .font(.system(size: 14, weight: .semibold))
                                            .foregroundStyle(palette.textPrimary)
                                    }
                                }
                                .padding(.vertical, 10)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.irisPlain)
                        }
                    }
                }

                IrisSectionCard {
                    Button {
                        manager.dispatch(.setChatMuted(chatId: "group:\(groupId)", muted: !details.isMuted))
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: details.isMuted ? "bell.fill" : "bell.slash.fill")
                            Text(details.isMuted ? "Unmute chat" : "Mute chat")
                            Spacer()
                        }
                        .foregroundStyle(palette.textPrimary)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("groupDetailsMuteButton")
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Members",
                        subtitle: "\(details.members.count) people in this conversation."
                    )

                    ForEach(Array(details.members.enumerated()), id: \.element.ownerPubkeyHex) { index, member in
                        let primary = primaryDisplayName(displayName: member.displayName, fallback: member.npub)
                        VStack(alignment: .leading, spacing: 10) {
                            let memberHeader = HStack(alignment: .top, spacing: 12) {
                                IrisAvatar(label: primary, size: 38, emphasize: member.isLocalOwner)

                                VStack(alignment: .leading, spacing: 6) {
                                    Text(primary)
                                        .font(.system(.headline, design: .rounded, weight: .semibold))
                                        .foregroundStyle(palette.textPrimary)
                                    HStack(spacing: 6) {
                                        if member.isLocalOwner {
                                            IrisInfoPill("You")
                                        }
                                        if member.isCreator {
                                            IrisInfoPill("Creator")
                                        } else if member.isAdmin {
                                            IrisInfoPill("Admin")
                                        }
                                    }
                                }

                                Spacer()
                            }
                            if member.isLocalOwner {
                                memberHeader
                            } else {
                                Button {
                                    manager.dispatch(.createChat(peerInput: member.ownerPubkeyHex))
                                } label: {
                                    memberHeader
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(.irisPlain)
                                .accessibilityIdentifier("groupDetailsOpenMember-\(String(member.ownerPubkeyHex.prefix(12)))")
                            }

                            if details.canManage && !member.isLocalOwner {
                                ViewThatFits(in: .horizontal) {
                                    HStack(spacing: 8) {
                                        memberAdminButton(member)
                                        removeMemberButton(member)
                                    }
                                    VStack(spacing: 8) {
                                        memberAdminButton(member)
                                        removeMemberButton(member)
                                    }
                                }
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
                            subtitle: "Search known users or paste / scan a user ID."
                        )

                        TextField("Search or paste user ID", text: $memberInput)
                            .irisIdentifierInputModifiers()
                            .textFieldStyle(.plain)
                            .irisInputField()
                            .accessibilityIdentifier("groupDetailsAddMemberInput")

                        VStack(spacing: 10) {
                            if irisSupportsQrScanning {
                                Button("Scan code") { showingScanner = true }
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

                    let candidateChats = knownUsersForAdding(details: details)
                    if !candidateChats.isEmpty {
                        IrisSectionCard {
                            CardHeader(
                                title: memberInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Known users" : "Search results"
                            )

                            ForEach(Array(candidateChats.enumerated()), id: \.element.chatId) { index, chat in
                                Button {
                                    manager.dispatch(.addGroupMembers(groupId: groupId, memberInputs: [chat.chatId]))
                                    memberInput = ""
                                } label: {
                                    HStack(spacing: 12) {
                                        IrisAvatar(label: chat.displayName, size: 38)
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
                                        Image(systemName: "plus.circle")
                                            .foregroundStyle(palette.textPrimary)
                                    }
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.irisPlain)
                                .accessibilityIdentifier("groupDetailsKnownUser-\(String(chat.chatId.prefix(12)))")
                                .disabled(manager.state.busy.updatingGroup)

                                if index < candidateChats.count - 1 {
                                    Divider().overlay(palette.border)
                                }
                            }
                        }
                    }
                }

                IrisSectionCard {
                    CardHeader(
                        title: "Delete chat",
                        subtitle: "Removes this group from your chat list and forgets local messages."
                    )
                    Button(role: .destructive) {
                        manager.dispatch(.deleteChat(chatId: "group:\(groupId)"))
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "trash")
                            Text("Delete chat")
                        }
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("groupDetailsDeleteChatButton")
                }
            }
        }
        .irisProfilePictureViewer(
            item: $groupPictureViewerItem,
            preferences: manager.state.preferences,
            manager: manager
        )
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                memberInput = normalizePeerInput(input: code)
                showingScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
        .fileImporter(
            isPresented: $showingGroupPicturePicker,
            allowedContentTypes: [.image],
            allowsMultipleSelection: false
        ) { result in
            if case let .success(urls) = result, let url = urls.first {
                manager.updateGroupPicture(groupId: groupId, fileURL: url)
            }
        }
    }

    @ViewBuilder
    private func groupAvatar(_ details: GroupDetailsSnapshot) -> some View {
        if let item = IrisProfilePictureViewerItem(
            label: details.name,
            pictureUrl: details.pictureUrl,
            accessibilityIdentifier: "groupDetailsProfilePictureViewer"
        ) {
            Button {
                groupPictureViewerItem = item
            } label: {
                groupAvatarImage(details)
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Open group photo")
            .accessibilityIdentifier("groupDetailsProfilePictureButton")
        } else {
            groupAvatarImage(details)
        }
    }

    private func groupAvatarImage(_ details: GroupDetailsSnapshot) -> some View {
        IrisAvatar(
            label: details.name,
            size: 56,
            emphasize: true,
            pictureUrl: details.pictureUrl,
            preferences: manager.state.preferences,
            manager: manager,
            loadedImageIdentifier: "groupDetailsAvatarImage"
        )
    }

    private func knownUsersForAdding(details: GroupDetailsSnapshot) -> [ChatThreadSnapshot] {
        let localOwnerHex = manager.state.account?.publicKeyHex
        let memberHexes = Set(details.members.map { $0.ownerPubkeyHex })
        return manager.state.chatList
            .filter { chat in
                chat.kind == .direct
                    && chat.chatId != localOwnerHex
                    && !memberHexes.contains(chat.chatId)
            }
            .filteredByQuery(memberInput)
    }

    private func memberAdminButton(_ member: GroupMemberSnapshot) -> some View {
        Button(member.isAdmin ? "Dismiss admin" : "Make admin") {
            manager.setGroupAdmin(
                groupId: groupId,
                ownerPubkeyHex: member.ownerPubkeyHex,
                isAdmin: !member.isAdmin
            )
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .disabled(manager.state.busy.updatingGroup || member.isCreator)
        .accessibilityIdentifier("groupDetailsAdminMember-\(String(member.ownerPubkeyHex.prefix(12)))")
    }

    private func removeMemberButton(_ member: GroupMemberSnapshot) -> some View {
        Button("Remove", role: .destructive) {
            manager.dispatch(.removeGroupMember(groupId: groupId, ownerPubkeyHex: member.ownerPubkeyHex))
        }
        .buttonStyle(IrisSecondaryButtonStyle(compact: true))
        .disabled(manager.state.busy.updatingGroup || member.isCreator)
        .accessibilityIdentifier("groupDetailsRemoveMember-\(String(member.ownerPubkeyHex.prefix(12)))")
    }
}

struct DeviceRosterScreen: View {
    @ObservedObject var manager: AppManager
    @State private var deviceInput = ""
    @State private var showingScanner = false

    var body: some View {
        IrisScrollScreen {
            DeviceRosterContent(
                manager: manager,
                deviceInput: $deviceInput,
                showingScanner: $showingScanner
            )
        }
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                deviceInput = code
                showingScanner = false
            }
            .irisModalSurface()
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
    }
}

private struct DeviceRosterContent: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @Binding var deviceInput: String
    @Binding var showingScanner: Bool

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
            return "Scan the code from the device you want to link, or paste it."
        }
        if isCurrentDeviceRegistered {
            return "This device can view the list but cannot change it."
        }
        return "Sign in with your secret key before changing devices."
    }

    var body: some View {
        if let roster = manager.state.deviceRoster {
            IrisSectionCard(accent: true) {
                CardHeader(
                    title: "Linked devices",
                    subtitle: "These devices can use your profile."
                )

                Button("Copy user ID") {
                    manager.copyToClipboard(roster.ownerNpub)
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("deviceRosterOwnerNpub")

                Button("Copy this device code") {
                    manager.copyToClipboard(roster.currentDeviceNpub)
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("deviceRosterCurrentDeviceNpub")
            }

            IrisSectionCard {
                CardHeader(
                    title: "Link another device",
                    subtitle: deviceAccessSubtitle
                )

                TextField("Link code", text: $deviceInput)
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
                        Button("Scan code") { showingScanner = true }
                            .buttonStyle(IrisSecondaryButtonStyle())
                            .accessibilityIdentifier("deviceRosterScanButton")
                    }
                    Button(manager.state.busy.updatingRoster ? "Linking…" : "Link device") {
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
                    title: "Devices",
                    subtitle: "\(roster.devices.count) linked"
                )

                if roster.devices.isEmpty {
                    Text("No linked devices")
                        .font(.system(.headline, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .accessibilityIdentifier("deviceRosterEmptyState")
                    Text("Linked devices will appear here.")
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
                Text("Devices unavailable.")
                    .font(.system(.headline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
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
        device.isCurrentDevice ? "This device" : "Linked device"
    }

    private var displaySubtitle: String {
        let client = device.isCurrentDevice ? PlatformDeviceLabels.currentClientLabel : "Iris Chat"
        return client
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
                IrisInfoPill(device.isAuthorized ? "Linked" : "Pending", tint: device.isAuthorized ? .green : .orange)
                if device.isStale {
                    IrisInfoPill("Needs attention", tint: .red)
                }
                if let ago = irisRelativeTime(device.addedAtSecs) {
                    IrisInfoPill("Added \(ago) ago", tint: .gray)
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
        Button(manager.state.busy.updatingRoster ? "Linking…" : "Link") {
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
        .alert("Remove device?", isPresented: $showingRemoveConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Remove", role: .destructive) {
                manager.removeAuthorizedDevice(devicePubkeyHex: device.devicePubkeyHex)
            }
            .accessibilityIdentifier("deviceRosterConfirmRemove-\(String(device.devicePubkeyHex.prefix(12)))")
        } message: {
            Text("This device will no longer use your profile.")
        }
    }
}

struct DeviceRevokedScreen: View {
    @ObservedObject var manager: AppManager
    @State private var showingLogoutConfirmation = false

    var body: some View {
        IrisScrollScreen {
            IrisSectionCard(accent: true) {
                Text("Device removed")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)

                Text("This device no longer has access. Sign in again to keep using Iris Chat here.")
                    .font(.system(.body, design: .rounded))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)

                Button("Sign in again") {
                    showingLogoutConfirmation = true
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .accessibilityIdentifier("deviceRevokedLogoutButton")
            }
            .accessibilityIdentifier("deviceRevokedScreen")
        }
        .alert("Delete all local data?", isPresented: $showingLogoutConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.logout()
            }
            .accessibilityIdentifier("deviceRevokedConfirmLogoutButton")
        } message: {
            Text("This removes your secret keys, messages, and cached files from this device.")
        }
    }
}

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
                deviceRosterInput = code
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
        .alert(item: $pendingSecretExport) { exportKind in
            let isDeviceExport = exportKind == .device
            return Alert(
                title: Text(isDeviceExport ? "Export This Device's Key" : "Export Secret Key"),
                message: Text(isDeviceExport
                    ? "This key only unlocks this device. Copy it now?"
                    : "Your secret key gives full access to your profile. Never share it with anyone. Store it securely."),
                primaryButton: .cancel(Text("Cancel")),
                secondaryButton: .default(Text(isDeviceExport ? "Copy Key" : "Copy")) {
                    let secret = isDeviceExport ? manager.exportDeviceNsec() : manager.exportOwnerNsec()
                    guard let secret, !secret.isEmpty else {
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
                ForEach(SettingsPage.menuPages.prefix(7)) { page in
                    SettingsMenuRow(page: page, selected: selectedPage == page) {
                        selectedPage = page
                    }
                }
            }

            SettingsMenuSection {
                ForEach(Array(SettingsPage.menuPages.dropFirst(7))) { page in
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
                    "New chats from anyone",
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
                    Label("Export this device's key", systemImage: "key.fill")
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("myProfileExportDeviceKeyButton")
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

private struct SupportBundleShareItem: Identifiable {
    let id = UUID()
    let url: URL
}

private struct SupportBundleShareSheet: View {
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

private enum ProfileQrTab: String, CaseIterable, Identifiable {
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

private struct ProfileQrModal: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    let closeSettings: (() -> Void)?
    @State private var selectedTab: ProfileQrTab = .code

    var body: some View {
        ZStack {
            BackgroundFill()

            VStack(spacing: 0) {
                header

                if selectedTab == .code {
                    ProfileQrCodePane(manager: manager, account: account)
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

private struct ProfileQrCodePane: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    @State private var copiedUserID = false
    @State private var copyResetTask: Task<Void, Never>?

    private var displayName: String {
        account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName
    }

    private var profileURL: URL {
        irisChatProfileURL(npub: account.npub)
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

                    ShareLink(item: profileURL) {
                        ProfileQrActionLabel(systemImage: "square.and.arrow.up", title: "Share")
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("profileQrShareButton")
                }

                Text("Scan to start a chat with me.")
                    .font(.system(.footnote, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .multilineTextAlignment(.center)
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
        VStack(spacing: 16) {
            QrCodeImage(text: profileURL.absoluteString, size: 214)
                .padding(14)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(Color.white)
                )
                .accessibilityIdentifier("myProfileQrCode")

            VStack(spacing: 4) {
                Text(displayName)
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .foregroundStyle(Color(red: 0.04, green: 0.11, blue: 0.22))
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 32)
        .padding(.vertical, 34)
        .frame(maxWidth: .infinity)
        .background(
            RoundedRectangle(cornerRadius: 26, style: .continuous)
                .fill(Color(red: 0.83, green: 0.91, blue: 1.0))
        )
    }

    private func copyUserID() {
        manager.copyToClipboard(profileURL.absoluteString)
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

private struct ProfileQrScanPane: View {
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

private struct ProfileQrActionButton: View {
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

private struct ProfileQrActionLabel: View {
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

private struct SettingsProfileMenuRow: View {
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

private struct SettingsMenuSection<Content: View>: View {
    @Environment(\.irisPalette) private var palette
    let content: () -> Content

    init(@ViewBuilder content: @escaping () -> Content) {
        self.content = content
    }

    var body: some View {
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

private struct SettingsMenuRow: View {
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

private struct SettingsExternalLinkRow: View {
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

#if os(iOS) || os(macOS)
private struct NearbySettingsRows: View {
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
private struct DesktopUpdateSettingsSection: View {
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

private struct NotificationsSettingsSection: View {
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

private struct ImageProxySettingsSection: View {
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

private struct NostrRelaySettingsSection: View {
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

private struct ProfileEditorCard: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let account: AccountSnapshot
    @Binding var profileName: String
    let openProfilePicture: (IrisProfilePictureViewerItem) -> Void
    let showQrCode: () -> Void
    @State private var showingProfilePicturePicker = false
    @State private var showingProfilePictureSourceMenu = false
    #if canImport(PhotosUI)
    @State private var showingProfilePicturePhotoPicker = false
    @State private var pickedProfilePicturePhotos: [PhotosPickerItem] = []
    #endif

    var body: some View {
        IrisSectionCard(accent: true) {
            VStack(spacing: 10) {
                profileAvatar
                Text(account.displayName.isEmpty ? "Profile" : account.displayName)
                    .font(.system(.title2, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
            }
            .frame(maxWidth: .infinity)
            .onAppear {
                profileName = account.displayName
            }
            .irisOnChange(of: account.displayName) { value in
                profileName = value
            }

            TextField("Display name", text: $profileName)
                .textFieldStyle(.roundedBorder)
                .disabled(!account.hasOwnerSigningAuthority)
                .accessibilityIdentifier("myProfileDisplayNameInput")

            Button(manager.state.busy.uploadingAttachment ? "Uploading…" : "Upload profile photo") {
                presentProfilePictureSource()
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .disabled(!account.hasOwnerSigningAuthority || manager.state.busy.uploadingAttachment)
            .accessibilityIdentifier("myProfileUploadPictureButton")

            Button("Save profile") {
                manager.updateProfileMetadata(name: profileName, pictureURL: account.pictureUrl)
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .disabled(!account.hasOwnerSigningAuthority || normalizedProfileName.isEmpty || !profileMetadataChanged)
            .accessibilityIdentifier("myProfileSaveProfileButton")

            Button {
                showQrCode()
            } label: {
                Label("Show QR code", systemImage: "qrcode")
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .accessibilityIdentifier("myProfileQrButton")

            VStack(spacing: 10) {
                IrisCopyButton(label: "Copy user ID", value: account.npub, compact: false)
                IrisCopyButton(label: "Copy this device code", value: account.deviceNpub, compact: false)
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
            #if canImport(PhotosUI)
            Button("Photo Library") { showingProfilePicturePhotoPicker = true }
            #endif
            Button("Files") { showingProfilePicturePicker = true }
            Button("Cancel", role: .cancel) {}
        }
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
            guard let url = await loadPickedProfilePicture(item) else { return }
            await MainActor.run {
                manager.uploadProfilePicture(fileURL: url)
            }
        }
    }

    private func loadPickedProfilePicture(_ item: PhotosPickerItem) async -> URL? {
        guard let data = try? await item.loadTransferable(type: Data.self) else {
            return nil
        }
        let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-profile-picks", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("\(UUID().uuidString).\(ext)")
        do {
            try data.write(to: url, options: [.atomic])
            return url
        } catch {
            return nil
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
                    size: 96,
                    emphasize: true,
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
                size: 96,
                emphasize: true,
                pictureUrl: account.pictureUrl,
                preferences: manager.state.preferences,
                manager: manager,
                loadedImageIdentifier: "myProfileAvatarImage"
            )
        }
    }

    private var profileMetadataChanged: Bool {
        normalizedProfileName != account.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedProfileName: String {
        profileName.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

private extension View {
    @ViewBuilder
    func irisProfilePictureViewer(
        item: Binding<IrisProfilePictureViewerItem?>,
        preferences: PreferencesSnapshot,
        manager: AppManager
    ) -> some View {
#if os(iOS)
        fullScreenCover(item: item) { viewerItem in
            IrisProfilePictureViewer(
                item: viewerItem,
                preferences: preferences,
                manager: manager
            ) {
                item.wrappedValue = nil
            }
        }
#else
        overlay {
            if let viewerItem = item.wrappedValue {
                IrisProfilePictureViewer(
                    item: viewerItem,
                    preferences: preferences,
                    manager: manager
                ) {
                    item.wrappedValue = nil
                }
            }
        }
#endif
    }
}

private struct IrisProfilePictureViewer: View {
    let item: IrisProfilePictureViewerItem
    let preferences: PreferencesSnapshot
    let manager: AppManager
    let onClose: () -> Void

    var body: some View {
        GeometryReader { geometry in
            let diameter = max(120, min(geometry.size.width, geometry.size.height) - 48)
            ZStack(alignment: .topTrailing) {
                Color.black
                    .ignoresSafeArea()
                    .onTapGesture(perform: onClose)

                IrisAvatar(
                    label: item.label,
                    size: diameter,
                    emphasize: false,
                    pictureUrl: item.pictureUrl,
                    preferences: preferences,
                    manager: manager,
                    loadedImageIdentifier: "\(item.accessibilityIdentifier)Image"
                )
                .overlay(
                    Circle()
                        .strokeBorder(Color.white.opacity(0.12), lineWidth: 1)
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)

                IrisModalCloseButton(
                    accessibilityLabel: "Close profile picture",
                    tone: .light,
                    iconSize: 30,
                    hitSize: 66,
                    action: onClose
                )
            }
        }
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .accessibilityIdentifier(item.accessibilityIdentifier)
        .zIndex(20)
    }
}

private struct BackgroundFill: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        // Solid palette.background — a previous gradient mixed in 28%
        // panelAlt at the bottom, which lifted the lower half of every
        // screen with no explicit .background of its own (e.g., the
        // chat screen) into a noticeably greyer tone than the near-
        // black palette value the rest of the app is tuned for.
        palette.background
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

// Tiny wrapper so RootView doesn't have to subscribe to ToastCenter via the
// fat AppManager — toasts pop on their own publisher and don't drag any
// other view into a re-render.
private struct ToastOverlay: View {
    @ObservedObject var center: ToastCenter

    var body: some View {
        if let toast = center.message {
            ToastView(text: toast)
                .padding(.top, 14)
        }
    }
}

#if canImport(AppKit)
private final class SecretKeyDraft: ObservableObject {
    @Published var text = ""
}

private final class BindingSecureTextField: NSSecureTextField {
    var onTextChange: ((String) -> Void)?

    override func textDidChange(_ notification: Notification) {
        super.textDidChange(notification)
        onTextChange?(stringValue)
    }

    override func textDidEndEditing(_ notification: Notification) {
        super.textDidEndEditing(notification)
        onTextChange?(stringValue)
    }
}

private struct MacSecretKeyField: NSViewRepresentable {
    @Binding var text: String

    func makeNSView(context: Context) -> NSSecureTextField {
        let field = BindingSecureTextField()
        field.delegate = context.coordinator
        field.target = context.coordinator
        field.action = #selector(Coordinator.textFieldAction(_:))
        field.isContinuous = true
        field.onTextChange = { value in
            context.coordinator.update(value)
        }
        field.identifier = NSUserInterfaceItemIdentifier("importKeyField")
        field.placeholderString = "Secret key"
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.textColor = .labelColor
        return field
    }

    func updateNSView(_ nsView: NSSecureTextField, context: Context) {
        if let field = nsView as? BindingSecureTextField {
            field.onTextChange = { value in
                context.coordinator.update(value)
            }
        }
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
        nsView.placeholderString = "Secret key"
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        private let text: Binding<String>

        init(text: Binding<String>) {
            self.text = text
        }

        func update(_ value: String) {
            text.wrappedValue = value
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else {
                return
            }
            update(field.stringValue)
        }

        func controlTextDidEndEditing(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else {
                return
            }
            update(field.stringValue)
        }

        @objc func textFieldAction(_ sender: NSTextField) {
            update(sender.stringValue)
        }
    }
}
#endif

#if !canImport(AppKit)
private final class SecretKeyDraft: ObservableObject {
    @Published var text = ""
}
#endif

private struct SecretKeyField: View {
    @Binding var text: String

    var body: some View {
        #if canImport(AppKit)
        MacSecretKeyField(text: $text)
            .frame(height: 22)
        #else
        SecureField("Secret key", text: $text)
            .irisIdentifierInputModifiers()
            .textContentType(.password)
            .textFieldStyle(.plain)
            .accessibilityIdentifier("importKeyField")
        #endif
    }
}

private struct LoadingOverlay: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        ZStack {
            palette.background.opacity(0.4).ignoresSafeArea()
            Image("IrisLogo")
                .resizable()
                .scaledToFit()
                .frame(width: 112, height: 112)
                .accessibilityLabel("Iris")
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
            .buttonStyle(.irisPlain)
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
