import Foundation
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

private let irisSourceURL = URL(string: "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs")!
private let irisSourceLabel = "Iris Chat source code"
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
}

private enum SettingsPage: String, CaseIterable, Identifiable {
    case profile
    case messaging
    case notifications
    case media
    case nearby
    case messageServers
    case security
    case updates
    case about
    case support
    case accountData

    var id: String { rawValue }

    var title: String {
        switch self {
        case .profile: return "Profile"
        case .messaging: return "Messaging"
        case .notifications: return "Notifications"
        case .media: return "Media"
        case .nearby: return "Nearby"
        case .messageServers: return "Message servers"
        case .security: return "Security"
        case .updates: return "Updates"
        case .about: return "About"
        case .support: return "Support"
        case .accountData: return "Account data"
        }
    }

    var systemImage: String {
        switch self {
        case .profile: return "person.crop.circle.fill"
        case .messaging: return "bubble.left.and.bubble.right.fill"
        case .notifications: return "bell.fill"
        case .media: return "photo.fill"
        case .nearby: return "dot.radiowaves.left.and.right"
        case .messageServers: return "server.rack"
        case .security: return "lock.fill"
        case .updates: return "arrow.down.circle.fill"
        case .about: return "info.circle.fill"
        case .support: return "wrench.and.screwdriver.fill"
        case .accountData: return "trash.fill"
        }
    }

    var accessibilityID: String {
        switch self {
        case .profile: return "settingsProfileRow"
        case .messaging: return "settingsMessagingRow"
        case .notifications: return "settingsNotificationsRow"
        case .media: return "settingsMediaRow"
        case .nearby: return "settingsNearbyRow"
        case .messageServers: return "settingsMessageServersRow"
        case .security: return "settingsSecurityRow"
        case .updates: return "settingsUpdatesRow"
        case .about: return "settingsAboutRow"
        case .support: return "settingsSupportRow"
        case .accountData: return "settingsAccountDataRow"
        }
    }

    static var menuPages: [SettingsPage] {
        var pages: [SettingsPage] = [
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
        pages.append(contentsOf: [.about, .support, .accountData])
        return pages
    }
}

struct RootView: View {
    @ObservedObject var manager: AppManager
    @State private var directChatInfoChatId: String?
    @State private var inChatSearch: InChatSearchTarget?
    @State private var settingsFocus: SettingsFocusSection?
#if os(iOS) || os(macOS)
    @State private var showingNearbyIris = false
#endif

    var body: some View {
        IrisTheme {
            ZStack(alignment: .top) {
                BackgroundFill()

                if usesDesktopChatShell {
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
                } else if case .welcome = manager.activeScreen {
                    WelcomeScreen(manager: manager)
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
                    NavigationShell(
                        title: screenTitle(manager.activeScreen),
                        subtitle: chatHeaderSubtitle,
                        subtitleSystemImage: chatHeaderSubtitleSystemImage,
                        canGoBack: manager.canNavigateBack,
                        onBack: manager.navigateBack,
                        backBadgeCount: backUnreadCount,
                        leading: topBarLeadingItem,
                        trailing: topBarTrailingItem,
                        titleAccessoryLeading: chatHeaderTitleAvatar,
                        onTitleTap: chatHeaderOnTap,
                        offlineBanner: offlineBanner
                    ) {
                        content
                    }
                }

                ToastOverlay(center: manager.toasts)

                if manager.bootstrapInFlight {
                    LoadingOverlay()
                        .allowsHitTesting(false)
                }
            }
        .sheet(item: $inChatSearch) { target in
            InChatSearchSheet(manager: manager, target: target) {
                inChatSearch = nil
            }
#if os(iOS)
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
#elseif os(macOS)
            .frame(minWidth: 420, minHeight: 520)
#endif
            .irisDismissOnMacOutsideClick { inChatSearch = nil }
        }
        }
#if os(iOS) || os(macOS)
        .sheet(isPresented: $showingNearbyIris) {
            NearbyIrisScreen(
                manager: manager,
                service: manager.nearbyIris,
                onClose: { showingNearbyIris = false }
            )
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
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
#elseif os(macOS)
            ShareTargetSheet(manager: manager, share: share)
                .frame(minWidth: 380, minHeight: 420)
#endif
        }
        .onAppear {
            manager.nearbyIris.startBluetoothStateMonitoring()
        }
#endif
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

    private var topBarLeadingItem: AnyView {
        guard case .chatList = manager.activeScreen, let account = manager.state.account else {
            return AnyView(EmptyView())
        }

        return AnyView(
            Button(action: { manager.dispatch(.pushScreen(screen: .settings)) }) {
                IrisAvatar(
                    label: account.displayName.isEmpty ? fallbackProfileNameForIdentity(account.npub) : account.displayName,
                    emphasize: true,
                    pictureUrl: account.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager,
                    loadedImageIdentifier: "chatListProfileAvatarImage"
                )
            }
            .buttonStyle(.irisPlain)
            // Chat-list rows pad to 16pt; the top bar pads to 8pt to
            // align with the chat-screen composer. The extra 8pt here
            // moves the chat-list header avatar to the same 16pt
            // gutter as the row avatars below it.
            .padding(.leading, 8)
            .accessibilityIdentifier("chatListProfileButton")
        )
    }

    private var topBarTrailingItem: AnyView {
        if case .chatList = manager.activeScreen {
            return AnyView(
                NewChatCircleButton {
                    manager.dispatch(.pushScreen(screen: .newChat))
                }
                .padding(.trailing, 8)
            )
        }

        // Surface "Search in this chat" on the chat / group-details
        // pages. Tapping pops up an inline scoped-search sheet so the
        // user doesn't have to navigate back to the chat list and
        // retype the chat name.
        if let target = chatHeaderSearchTarget() {
            return AnyView(
                Button {
                    inChatSearch = target
                } label: {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 16, weight: .semibold))
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.irisPlain)
                .accessibilityLabel("Search in this chat")
                .accessibilityIdentifier("chatHeaderSearchButton")
                .padding(.trailing, 8)
            )
        }

        // The chat header avatar/title is the entry point to chat info now —
        // no overflow menu needed.
        return AnyView(EmptyView())
    }

    private func chatHeaderSearchTarget() -> InChatSearchTarget? {
        switch manager.activeScreen {
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

    private var backUnreadCount: UInt64 {
        guard case .chat(let activeChatId) = manager.activeScreen else {
            return 0
        }
        return manager.state.chatList
            .filter { $0.chatId != activeChatId }
            .reduce(UInt64(0)) { $0 + $1.unreadCount }
    }

    private var chatHeaderTitleAvatar: AnyView {
        guard case .chat = manager.activeScreen, let chat = manager.state.currentChat else {
            return AnyView(EmptyView())
        }
        return AnyView(
            IrisAvatar(
                label: chat.displayName,
                size: 34,
                pictureUrl: chat.pictureUrl,
                preferences: manager.state.preferences,
                manager: manager
            )
        )
    }

    private var chatHeaderOnTap: (() -> Void)? {
        guard case .chat = manager.activeScreen, let chat = manager.state.currentChat else {
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

    private var chatHeaderSubtitle: String? {
        guard case .chat = manager.activeScreen, let chat = manager.state.currentChat else {
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

    private var chatHeaderSubtitleSystemImage: String? {
        guard case .chat = manager.activeScreen, let chat = manager.state.currentChat else {
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
                    settingsFocus = .messageServers
                    manager.dispatch(.pushScreen(screen: .settings))
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

    var body: some View {
        NavigationStack {
            Group {
                if manager.state.chatList.isEmpty {
                    VStack(spacing: 18) {
                        Text("Start a chat first")
                            .font(.headline)
                        Button("New chat") {
                            manager.clearPendingShare()
                            manager.dispatch(.pushScreen(screen: .newChat))
                            dismiss()
                        }
                        .buttonStyle(.borderedProminent)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    List {
                        if filteredChats.isEmpty {
                            Text("No matches")
                                .foregroundStyle(.secondary)
                        }
                        ForEach(filteredChats, id: \.chatId) { chat in
                            shareTargetRow(chat)
                        }
                    }
                    .searchable(text: $searchText, prompt: "Search")
                    .listStyle(.plain)
                    .onAppear(perform: preselectSuggestedChat)
                    .irisOnChange(of: share.id) { _ in
                        selectedChatIds.removeAll()
                        preselectSuggestedChat()
                    }
                }
            }
            .navigationTitle("Choose recipients")
#if os(iOS)
            .safeAreaInset(edge: .bottom) {
                if !manager.state.chatList.isEmpty {
                    sendBar
                }
            }
#endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        manager.clearPendingShare()
                        dismiss()
                    }
                }
#if os(macOS)
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
        }
    }

    private var sendButtonTitle: String {
        selectedChatIds.count > 1 ? "Send (\(selectedChatIds.count))" : "Send"
    }

#if os(iOS)
    private var sendBar: some View {
        Button(sendButtonTitle) {
            sendSelectedAndDismiss()
        }
        .buttonStyle(IrisPrimaryButtonStyle())
        .disabled(selectedChatIds.isEmpty)
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .background(.regularMaterial)
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
                Image(systemName: selected ? "checkmark.square.fill" : "square")
                    .font(.system(size: 23, weight: .semibold))
                    .foregroundStyle(selected ? palette.textPrimary : palette.muted)
                    .frame(width: 28, height: 28)
                IrisAvatar(
                    label: chat.displayName,
                    size: 38,
                    emphasize: selected,
                    pictureUrl: chat.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager
                )
                VStack(alignment: .leading, spacing: 3) {
                    Text(chat.displayName)
                        .foregroundStyle(.primary)
                    if let subtitle = chat.subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("\(chat.displayName), \(selected ? "selected" : "not selected")")
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
#endif

struct NavigationShell<Content: View>: View {
    let title: String
    let subtitle: String?
    let subtitleSystemImage: String?
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

    var body: some View {
        content()
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            // Float the IrisTopBar (and the offline banner when active)
            // over the screen content via .safeAreaInset, so the chat
            // timeline scrolls *under* the header instead of being
            // bumped down by a solid bar above it.
            .safeAreaInset(edge: .top, spacing: 0) {
                VStack(spacing: 0) {
                    IrisTopBar(
                        title: title,
                        subtitle: subtitle,
                        subtitleSystemImage: subtitleSystemImage,
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
                // Signal-style "fade into chat" header: a vertical
                // gradient fading from the toolbar tone at the top
                // into the chat background at the bottom — no
                // hairline divider, the title cluster floats on a
                // soft gradient that softens into the timeline.
                // Drawn here (not inside IrisTopBar) because
                // .safeAreaInset content can't extend its background
                // up into the system status bar with the normal
                // .ignoresSafeArea modifier.
                .background(alignment: .top) {
                    NavigationHeaderChrome(palette: palette)
                        .ignoresSafeArea(.all, edges: .top)
                }
            }
#if os(iOS)
            .modifier(IrisSwipeBackModifier(enabled: canGoBack, onBack: onBack))
#endif
    }
}

private struct NavigationHeaderChrome: View {
    let palette: IrisPalette

    var body: some View {
        // Pure background color fading to transparent — no toolbar
        // tone lift. In dark theme that's black-at-top → transparent,
        // in light theme white-at-top → transparent. Identical fade
        // shape, just inverted by the palette.
        LinearGradient(
            colors: [
                palette.background,
                palette.background.opacity(0.78),
                palette.background.opacity(0)
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }
}

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

private struct IrisSwipeBackModifier: ViewModifier {
    let enabled: Bool
    let onBack: () -> Void

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .simultaneousGesture(
                DragGesture(minimumDistance: 18, coordinateSpace: .global)
                    .onEnded { value in
                        guard enabled, value.startLocation.x <= 28 else {
                            return
                        }
                        let horizontal = value.translation.width
                        let vertical = abs(value.translation.height)
                        guard horizontal > 72, horizontal > vertical * 1.35 else {
                            return
                        }
                        onBack()
                    }
            )
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

private struct DirectChatInfoScreen: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let chatId: String
    let onClose: () -> Void
    @State private var advancedExpanded = false
    @State private var profileDebug: PeerProfileDebugSnapshot?
    @State private var loadedProfileDebugFor: String?

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let chat {
                    HStack(spacing: 14) {
                        IrisAvatar(
                            label: chat.displayName,
                            size: 72,
                            emphasize: true,
                            pictureUrl: chat.pictureUrl,
                            preferences: manager.state.preferences,
                            manager: manager
                        )
                        VStack(alignment: .leading, spacing: 4) {
                            Text(chat.displayName)
                                .font(.system(.title3, design: .rounded, weight: .bold))
                                .foregroundStyle(palette.textPrimary)
                            if let subtitle = chat.subtitle, !subtitle.isEmpty {
                                Text(subtitle)
                                    .font(.system(.footnote, design: .rounded))
                                    .foregroundStyle(palette.muted)
                            }
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.top, 8)

                    IrisCopyButton(label: "Copy user ID", value: peerInputToNpub(input: chatId))
                        .accessibilityIdentifier("directChatCopyUserIdButton")

                    Button {
                        manager.dispatch(.setChatMuted(chatId: chatId, muted: !chat.isMuted))
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: chat.isMuted ? "bell.fill" : "bell.slash.fill")
                            Text(chat.isMuted ? "Unmute chat" : "Mute chat")
                        }
                        .foregroundStyle(palette.textPrimary)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("directChatMuteButton")

                    IrisSectionCard {
                        CardHeader(
                            title: "Disappearing messages",
                            subtitle: "Messages auto-delete after the chosen interval."
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

                    Button {
                        manager.dispatch(.deleteChat(chatId: chatId))
                        onClose()
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "trash")
                            Text("Delete chat")
                        }
                        .foregroundStyle(.red)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.irisPlain)
                    .accessibilityIdentifier("directChatDeleteButton")
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
        .alert("Delete app data?", isPresented: $showingLogoutConfirmation) {
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
                    NearbyChatListRow(manager: manager, service: manager.nearbyIris, onOpen: onOpenNearby)
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

private enum ChatListSearchSection: Hashable {
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

    var body: some View {
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
                .accessibilityIdentifier("chatListSearchField")
            if !text.isEmpty {
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
    }
}

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
                Button("Done", action: onClose)
                    .font(.system(.body, weight: .semibold))
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
            Image(systemName: "square.and.pencil")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .frame(width: 36, height: 36)
                .irisGlassSurface(in: Circle())
        }
        .buttonStyle(.irisPlain)
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

#if os(iOS)
        SwipeableChatListRow(
            chat: chat,
            row: row,
            onToggleUnread: {
                manager.dispatch(.setChatUnread(chatId: chat.chatId, unread: chat.unreadCount == 0))
            },
            onTogglePin: {
                manager.dispatch(.setChatPinned(chatId: chat.chatId, pinned: !chat.isPinned))
            },
            onToggleMute: {
                manager.dispatch(.setChatMuted(chatId: chat.chatId, muted: !chat.isMuted))
            },
            onDelete: {
                manager.dispatch(.deleteChat(chatId: chat.chatId))
            }
        )
#elseif os(macOS)
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
private struct SwipeableChatListRow<Row: View>: View {
    @Environment(\.irisPalette) private var palette

    let chat: ChatThreadSnapshot
    let row: Row
    let onToggleUnread: () -> Void
    let onTogglePin: () -> Void
    let onToggleMute: () -> Void
    let onDelete: () -> Void

    @State private var restingOffset: CGFloat = 0
    @State private var displayedOffset: CGFloat = 0
    @State private var activeDragAxis: DragAxis?
    @State private var showingDeleteConfirmation = false
    @GestureState private var dragIsActive = false

    private let actionWidth: CGFloat = 152
    private let revealThreshold: CGFloat = 42

    private enum DragAxis {
        case horizontal
        case vertical
    }

    private var currentOffset: CGFloat {
        displayedOffset
    }

    private var snapAnimation: Animation {
        .spring(response: 0.24, dampingFraction: 0.86)
    }

    var body: some View {
        ZStack {
            leadingActions
                .frame(maxWidth: .infinity, alignment: .leading)
                .allowsHitTesting(currentOffset > 1)
                .accessibilityHidden(currentOffset <= 1)

            trailingActions
                .frame(maxWidth: .infinity, alignment: .trailing)
                .allowsHitTesting(currentOffset < -1)
                .accessibilityHidden(currentOffset >= -1)

            row
                .background(palette.background)
                .offset(x: currentOffset)
                .highPriorityGesture(rowDragGesture)
                .accessibilityAction(named: chat.isMuted ? "Unmute" : "Mute") {
                    onToggleMute()
                }
                .accessibilityAction(named: chat.unreadCount > 0 ? "Mark read" : "Mark as unread") {
                    onToggleUnread()
                }
                .accessibilityAction(named: chat.isPinned ? "Unpin" : "Pin") {
                    onTogglePin()
                }
                .accessibilityAction(named: "Delete") {
                    showingDeleteConfirmation = true
                }
        }
        .clipped()
        .onChange(of: dragIsActive) { isActive in
            guard !isActive else { return }
            activeDragAxis = nil
            if displayedOffset != restingOffset {
                displayOffset(restingOffset, animated: true)
            }
        }
        .alert("Delete chat?", isPresented: $showingDeleteConfirmation) {
            Button("Delete", role: .destructive) {
                onDelete()
                setRestingOffset(0, animated: false)
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    private var leadingActions: some View {
        HStack(spacing: 0) {
            swipeButton(
                title: chat.unreadCount > 0 ? "Read" : "Unread",
                systemImage: chat.unreadCount > 0 ? "envelope.open.fill" : "envelope.badge.fill",
                tint: palette.accent
            ) {
                onToggleUnread()
                setRestingOffset(0, animated: true)
            }
            swipeButton(
                title: chat.isPinned ? "Unpin" : "Pin",
                systemImage: chat.isPinned ? "pin.slash.fill" : "pin.fill",
                tint: palette.accentAlt
            ) {
                onTogglePin()
                setRestingOffset(0, animated: true)
            }
        }
        .frame(width: actionWidth)
    }

    private var trailingActions: some View {
        HStack(spacing: 0) {
            swipeButton(
                title: chat.isMuted ? "Unmute" : "Mute",
                systemImage: chat.isMuted ? "bell.fill" : "bell.slash.fill",
                tint: palette.accentAlt
            ) {
                onToggleMute()
                setRestingOffset(0, animated: true)
            }
            swipeButton(
                title: "Delete",
                systemImage: "trash.fill",
                tint: .red
            ) {
                showingDeleteConfirmation = true
            }
        }
        .frame(width: actionWidth)
    }

    private func swipeButton(
        title: String,
        systemImage: String,
        tint: Color,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: systemImage)
                    .font(.system(size: 16, weight: .semibold))
                Text(title)
                    .font(.system(.caption2, design: .rounded, weight: .bold))
                    .lineLimit(1)
            }
            .foregroundStyle(Color.white)
            .frame(width: actionWidth / 2)
            .frame(maxHeight: .infinity)
            .background(tint)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
    }

    private var rowDragGesture: some Gesture {
        DragGesture(minimumDistance: 12, coordinateSpace: .local)
            .updating($dragIsActive) { _, state, _ in
                state = true
            }
            .onChanged { value in
                let horizontal = abs(value.translation.width)
                let vertical = abs(value.translation.height)

                if activeDragAxis == nil {
                    activeDragAxis = horizontal > vertical ? .horizontal : .vertical
                }

                guard activeDragAxis == .horizontal else { return }
                displayOffset(restingOffset + value.translation.width, animated: false)
            }
            .onEnded { value in
                guard activeDragAxis == .horizontal else {
                    activeDragAxis = nil
                    return
                }

                setRestingOffset(targetOffset(for: value), animated: true)
                activeDragAxis = nil
            }
    }

    private func targetOffset(for value: DragGesture.Value) -> CGFloat {
        let projectedOffset = clampedOffset(restingOffset + value.translation.width)
        let predictedOffset = clampedOffset(restingOffset + value.predictedEndTranslation.width)

        if restingOffset > actionWidth / 2 {
            let shouldClose = projectedOffset < actionWidth - revealThreshold || predictedOffset < actionWidth / 2
            return shouldClose ? 0 : actionWidth
        }

        if restingOffset < -actionWidth / 2 {
            let shouldClose = projectedOffset > -actionWidth + revealThreshold || predictedOffset > -actionWidth / 2
            return shouldClose ? 0 : -actionWidth
        }

        let shouldRevealLeading = projectedOffset > revealThreshold || predictedOffset > actionWidth / 2
        if shouldRevealLeading {
            return actionWidth
        }

        let shouldReveal = projectedOffset < -revealThreshold || predictedOffset < -actionWidth / 2
        return shouldReveal ? -actionWidth : 0
    }

    private func setRestingOffset(_ offset: CGFloat, animated: Bool) {
        let offset = clampedOffset(offset)
        let changes = {
            restingOffset = offset
            displayedOffset = offset
        }

        if animated {
            withAnimation(snapAnimation, changes)
        } else {
            withoutAnimation(changes)
        }
    }

    private func displayOffset(_ offset: CGFloat, animated: Bool) {
        let changes = {
            displayedOffset = clampedOffset(offset)
        }

        if animated {
            withAnimation(snapAnimation, changes)
        } else {
            withoutAnimation(changes)
        }
    }

    private func withoutAnimation(_ changes: () -> Void) {
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction, changes)
    }

    private func clampedOffset(_ offset: CGFloat) -> CGFloat {
        min(actionWidth, max(-actionWidth, offset))
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
        .frame(width: 48, height: 48)
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
            .irisDismissOnMacOutsideClick { showingScanner = false }
        }
        #if os(macOS)
        .overlay { inviteQrOverlay }
        #else
        .sheet(isPresented: $showingInviteQr) {
            inviteQrSheet
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
                        IrisAvatar(
                            label: details.name,
                            size: 56,
                            emphasize: true,
                            pictureUrl: details.pictureUrl,
                            preferences: manager.state.preferences,
                            manager: manager
                        )
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
                            HStack(alignment: .top, spacing: 12) {
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
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                memberInput = normalizePeerInput(input: code)
                showingScanner = false
            }
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
            return "Scan the code from the device you want to link, or paste it."
        }
        if isCurrentDeviceRegistered {
            return "This device can view the list but cannot change it."
        }
        return "Sign in with your secret key before changing devices."
    }

    var body: some View {
        IrisScrollScreen {
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
        .sheet(isPresented: $showingScanner) {
            QrScannerSheet { code in
                deviceInput = code
                showingScanner = false
            }
            .irisDismissOnMacOutsideClick { showingScanner = false }
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
        .alert("Delete app data?", isPresented: $showingLogoutConfirmation) {
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
    @State private var pendingSecretExport: SecretExportKind?
    @State private var showingLogoutConfirmation = false
    @State private var showingDeleteAllConfirmation = false
    @State private var profileName = ""
    @State private var profilePictureViewerURL: URL?
    @State private var newRelayURL = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""
    @State private var selectedPage: SettingsPage?

    var body: some View {
        settingsBody
            // Settings contains copyable values like version, user ID,
            // device key, server URLs, and build metadata. Buttons and
            // Links still receive taps; inert Text can be selected.
            .textSelection(.enabled)
    }

    @ViewBuilder
    private var settingsBody: some View {
        ZStack {
            BackgroundFill()

            if IrisLayout.usesDesktopChrome {
                desktopSettingsLayout
            } else {
                mobileSettingsLayout
            }
        }
        .overlay {
            if let profilePictureViewerURL {
                IrisProfilePictureViewer(url: profilePictureViewerURL) {
                    self.profilePictureViewerURL = nil
                }
            }
        }
        .onAppear(perform: applyFocusedSection)
        .onChange(of: focusedSection) { _ in
            applyFocusedSection()
        }
        .accessibilityIdentifier("settingsScreen")
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
        .alert("Delete app data?", isPresented: $showingLogoutConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.logout()
            }
            .accessibilityIdentifier("myProfileConfirmLogoutButton")
        } message: {
            Text("This removes your secret keys, messages, and cached files from this device.")
        }
        .alert("Delete app data?", isPresented: $showingDeleteAllConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                manager.resetAppState()
            }
            .accessibilityIdentifier("myProfileConfirmDeleteAllDataButton")
        } message: {
            Text("This removes your secret keys, messages, and cached files from this device.")
        }
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
        if let selectedPage {
            settingsPageScroll(selectedPage, showsBackButton: true)
        } else {
            IrisScrollScreen {
                settingsMenu
            }
        }
    }

    private var settingsMenu: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let account = manager.state.account {
                SettingsProfileMenuRow(
                    account: account,
                    preferences: manager.state.preferences,
                    manager: manager
                ) {
                    selectedPage = .profile
                }
            }

            SettingsMenuSection {
                ForEach(SettingsPage.menuPages.prefix(6)) { page in
                    SettingsMenuRow(page: page, selected: selectedPage == page) {
                        selectedPage = page
                    }
                }
            }

            SettingsMenuSection {
                ForEach(Array(SettingsPage.menuPages.dropFirst(6))) { page in
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
                Button {
                    selectedPage = nil
                } label: {
                    Label("Settings", systemImage: "chevron.left")
                        .font(.system(.body, design: .rounded, weight: .semibold))
                }
                .buttonStyle(.irisPlain)
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
                    openProfilePicture: { profilePictureViewerURL = $0 },
                    manageDevices: {
                        manager.dispatch(.pushScreen(screen: .deviceRoster))
                    }
                )
            }

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

                ShareLink(item: manager.supportBundleJson()) {
                    HStack(spacing: 8) {
                        Image(systemName: "square.and.arrow.up")
                        Text("Share debug dump")
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(IrisPrimaryButtonStyle())
                .accessibilityIdentifier("myProfileShareSupportBundleButton")

                Button("Copy debug dump") {
                    manager.copyToClipboard(manager.supportBundleJson())
                }
                .buttonStyle(IrisSecondaryButtonStyle())
                .accessibilityIdentifier("myProfileCopySupportBundleButton")
            }

        case .accountData:
            IrisSectionCard {
                CardHeader(
                    title: "Account data",
                    subtitle: "Local profile, secret keys, messages, and cached files are removed from this device."
                )

                Button("Logout", role: .destructive) {
                    showingLogoutConfirmation = true
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
        }
        self.focusedSection = nil
    }

}

private struct SettingsProfileMenuRow: View {
    @Environment(\.irisPalette) private var palette
    let account: AccountSnapshot
    let preferences: PreferencesSnapshot
    @ObservedObject var manager: AppManager
    let action: () -> Void

    var body: some View {
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
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(palette.muted)
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
        .buttonStyle(.irisPlain)
        .accessibilityIdentifier("settingsProfileRow")
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
                    .background(
                        Circle()
                            .fill(selected ? palette.accent.opacity(0.16) : palette.toolbar)
                    )
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

#if os(iOS) || os(macOS)
private struct NearbySettingsRows: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
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
    let openProfilePicture: (URL) -> Void
    let manageDevices: () -> Void
    @State private var showingProfilePicturePicker = false
    @State private var showingProfilePictureSourceMenu = false
    #if canImport(PhotosUI)
    @State private var showingProfilePicturePhotoPicker = false
    @State private var pickedProfilePicturePhotos: [PhotosPickerItem] = []
    #endif

    var body: some View {
        IrisSectionCard(accent: true) {
            HStack(spacing: 14) {
                profileAvatar
                VStack(alignment: .leading, spacing: 4) {
                    Text(account.displayName.isEmpty ? "Profile" : account.displayName)
                        .font(.system(.title3, design: .rounded, weight: .bold))
                        .foregroundStyle(palette.textPrimary)
                }
            }
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
                manageDevices()
            } label: {
                Label("Manage devices", systemImage: "laptopcomputer.and.iphone")
            }
            .buttonStyle(IrisSecondaryButtonStyle())
            .accessibilityIdentifier("myProfileManageDevicesButton")

            QrCodeImage(text: account.npub, size: 220)
                .frame(maxWidth: .infinity, alignment: .center)
                .accessibilityIdentifier("myProfileQrCode")

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
        let trimmedURL = account.pictureUrl?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let isViewable = trimmedURL.hasPrefix("http://") || trimmedURL.hasPrefix("https://")
        let displayURL = proxiedImageURL(trimmedURL, preferences: manager.state.preferences, width: 1024, height: 1024)
        if isViewable, let url = URL(string: displayURL ?? trimmedURL) {
            Button {
                openProfilePicture(url)
            } label: {
                IrisAvatar(
                    label: label,
                    size: 52,
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
                size: 52,
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
                case .failure:
                    Image(systemName: "photo.badge.exclamationmark")
                        .font(.system(size: 56, weight: .regular))
                        .foregroundStyle(.white.opacity(0.7))
                case .empty:
                    ProgressView()
                        .tint(.white)
                @unknown default:
                    ProgressView()
                        .tint(.white)
                }
            }

            IrisModalCloseButton(
                accessibilityLabel: "Close profile picture",
                tone: .light,
                iconSize: 30,
                hitSize: 66,
                action: onClose
            )
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
