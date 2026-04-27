import Foundation
import SwiftUI
import UniformTypeIdentifiers
#if canImport(PhotosUI)
import PhotosUI
#endif

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

struct RootView: View {
    @ObservedObject var manager: AppManager
    @State private var directChatInfoChatId: String?

    var body: some View {
        IrisTheme {
            ZStack(alignment: .top) {
                BackgroundFill()

                if usesDesktopChatShell {
                    DesktopChatShell(manager: manager)
                } else {
                    NavigationShell(
                        title: screenTitle(manager.activeScreen),
                        canGoBack: manager.canNavigateBack,
                        onBack: manager.navigateBack,
                        backBadgeCount: backUnreadCount,
                        leading: topBarLeadingItem,
                        trailing: topBarTrailingItem,
                        titleAccessoryLeading: chatHeaderTitleAvatar,
                        onTitleTap: chatHeaderOnTap
                    ) {
                        content
                    }
                }

                if let toast = manager.toastMessage {
                    ToastView(text: toast)
                        .padding(.top, 14)
                }

                if manager.bootstrapInFlight {
                    LoadingOverlay()
                }
            }
            .sheet(
                item: Binding(
                    get: { directChatInfoChatId.map(IdentifiedString.init) },
                    set: { directChatInfoChatId = $0?.value }
                )
            ) { wrapper in
                DirectChatInfoSheet(manager: manager, chatId: wrapper.value)
                    .presentationDetents([.medium])
                    .presentationDragIndicator(.visible)
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
                    pictureUrl: account.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager,
                    loadedImageIdentifier: "chatListProfileAvatarImage"
                )
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("chatListProfileButton")
        )
    }

    private var topBarTrailingItem: AnyView {
        if case .chatList = manager.activeScreen {
            return AnyView(
                Button(action: { manager.dispatch(.pushScreen(screen: .newChat)) }) {
                    Label("New", systemImage: "square.and.pencil")
                }
                .buttonStyle(IrisPrimaryButtonStyle(compact: true))
                .accessibilityIdentifier("chatListNewChatButton")
            )
        }

        // The chat header avatar/title is the entry point to chat info now —
        // no overflow menu needed.
        return AnyView(EmptyView())
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
                size: 30,
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
    let backBadgeCount: UInt64
    let leading: AnyView
    let trailing: AnyView
    let titleAccessoryLeading: AnyView
    let onTitleTap: (() -> Void)?
    let content: () -> Content

    init(
        title: String,
        canGoBack: Bool,
        onBack: @escaping () -> Void,
        backBadgeCount: UInt64 = 0,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView()),
        titleAccessoryLeading: AnyView = AnyView(EmptyView()),
        onTitleTap: (() -> Void)? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.backBadgeCount = backBadgeCount
        self.leading = leading
        self.trailing = trailing
        self.titleAccessoryLeading = titleAccessoryLeading
        self.onTitleTap = onTitleTap
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
                trailing: trailing,
                titleAccessoryLeading: titleAccessoryLeading,
                onTitleTap: onTitleTap
            )

            content()
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
    }
}

private struct DesktopChatShell: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager

    var body: some View {
        HStack(spacing: 0) {
            DesktopChatSidebar(manager: manager)
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
            SettingsScreen(manager: manager)
        case .chat(let chatId):
            let chat = manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
            DesktopPaneTopBar(
                title: chat?.displayName ?? "Chat",
                subtitle: chat?.subtitle,
                onTitleTap: chat.map { current in
                    {
                        if let groupId = current.groupId {
                            manager.dispatch(.pushScreen(screen: .groupDetails(groupId: groupId)))
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
    let canGoBack: Bool
    let onBack: () -> Void
    let onTitleTap: (() -> Void)?
    let leading: AnyView
    let trailing: AnyView

    init(
        title: String,
        subtitle: String? = nil,
        canGoBack: Bool = false,
        onBack: @escaping () -> Void = {},
        onTitleTap: (() -> Void)? = nil,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView())
    ) {
        self.title = title
        self.subtitle = subtitle
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
                    Text(subtitle)
                        .font(.system(.caption, design: .rounded))
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
                .buttonStyle(.plain)
                .accessibilityIdentifier("desktopPaneBackButton")
            }

            if let onTitleTap {
                Button(action: onTitleTap) { titleStack }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("chatHeaderTitleButton")
            } else {
                titleStack
            }

            Spacer(minLength: 12)

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

                    TimelineView(.periodic(from: .now, by: 15)) { timeline in
                        ForEach(filteredChats, id: \.chatId) { chat in
                            DesktopSidebarChatRow(
                                manager: manager,
                                chat: chat,
                                timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: timeline.date),
                                selected: selectedChatId == chat.chatId
                            )
                            .accessibilityIdentifier("chatRow-\(String(chat.chatId.prefix(12)))")
                        }
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
                    .buttonStyle(.plain)
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
            .buttonStyle(.plain)
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
        .buttonStyle(.plain)
    }

    private var rowBackground: some View {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
            .fill(selected ? palette.panelAlt : Color.clear)
    }
}

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
                        Text(chat.displayName)
                            .font(.system(.headline, design: .rounded, weight: chat.unreadCount > 0 ? .bold : .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)

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

                        if chat.unreadCount > 0 {
                            Text(chat.unreadCount > 99 ? "99+" : "\(chat.unreadCount)")
                                .font(.system(size: 11, weight: .bold, design: .rounded))
                                .foregroundStyle(palette.onAccent)
                                .padding(.horizontal, 7)
                                .frame(minHeight: 20)
                                .background(Capsule().fill(palette.accent))
                        }
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
        .buttonStyle(.plain)
    }
}

private struct IdentifiedString: Identifiable, Hashable {
    let value: String
    var id: String { value }
}

private struct DirectChatInfoSheet: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var manager: AppManager
    let chatId: String

    private var chat: CurrentChatSnapshot? {
        manager.state.currentChat?.chatId == chatId ? manager.state.currentChat : nil
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    if let chat {
                        IrisAvatar(
                            label: chat.displayName,
                            size: 96,
                            emphasize: true,
                            pictureUrl: chat.pictureUrl,
                            preferences: manager.state.preferences,
                            manager: manager
                        )
                        .padding(.top, 8)

                        Text(chat.displayName)
                            .font(.system(.title2, design: .rounded, weight: .bold))
                            .foregroundStyle(palette.textPrimary)
                            .multilineTextAlignment(.center)

                        if let subtitle = chat.subtitle, !subtitle.isEmpty {
                            Text(subtitle)
                                .font(.system(.footnote, design: .rounded))
                                .foregroundStyle(palette.muted)
                                .multilineTextAlignment(.center)
                                .padding(.horizontal, 24)
                        }

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
                                                    .foregroundStyle(palette.accent)
                                            }
                                        }
                                        .padding(.vertical, 10)
                                        .contentShape(Rectangle())
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    } else {
                        ProgressView()
                            .padding(.top, 40)
                    }
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 24)
                .frame(maxWidth: .infinity)
            }
            .background(palette.background)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .navigationBarTitleDisplayMode(.inline)
        }
    }
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
            VStack(spacing: 18) {
                IrisSectionCard(accent: true) {
                    Color.clear
                        .frame(height: 0)
                        .accessibilityIdentifier("welcomeChooserCard")

                    VStack(alignment: .leading, spacing: 18) {
                        IrisAvatar(label: "Iris Chat", size: 62, emphasize: true)

                        Text("Iris Chat")
                            .font(.system(.largeTitle, design: .rounded, weight: .bold))
                            .foregroundStyle(palette.textPrimary)

                        VStack(spacing: 10) {
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
                    }
                }

                if manager.trustedTestBuildEnabled() {
                    IrisSectionCard(accent: true) {
                        Color.clear
                            .frame(height: 0)
                            .accessibilityIdentifier("welcomeSecondaryCard")

                        CardHeader(title: "Trusted test build")

                        Text(manager.buildSummaryText())
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(palette.muted)
                    }
                }
            }
            .frame(maxWidth: 480)
            .frame(maxWidth: .infinity)
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
                        TextField("User ID", text: $ownerInput)
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
        ScrollView {
            if manager.state.chatList.isEmpty {
                Text("No chats yet")
                    .font(.system(.body, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
            } else {
                TimelineView(.periodic(from: .now, by: 15)) { timeline in
                    VStack(spacing: 0) {
                        ForEach(Array(manager.state.chatList.enumerated()), id: \.element.chatId) { index, chat in
                            ChatListRowContainer(
                                manager: manager,
                                chat: chat,
                                timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: timeline.date)
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
        .background(palette.background)
    }
}

private struct ChatListRowContainer: View {
    @ObservedObject var manager: AppManager
    let chat: ChatThreadSnapshot
    let timeLabel: String?

    var body: some View {
        IrisChatRow(
            title: chat.displayName,
            preview: chat.isTyping ? "Typing" : (chat.lastMessagePreview ?? chat.subtitle ?? "No messages yet"),
            subtitle: nil,
            timeLabel: timeLabel,
            unreadCount: chat.unreadCount,
            pictureUrl: chat.pictureUrl,
            preferences: manager.state.preferences,
            manager: manager,
            onTap: {
                manager.dispatch(.openChat(chatId: chat.chatId))
            }
        )
    }
}

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

    private var looksLikeInviteLink: Bool {
        let lower = trimmedInput.lowercased()
        return lower.contains("://") && lower.contains("#")
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
        }
        .sheet(isPresented: $showingInviteQr) {
            inviteQrSheet
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
            Text("New Chat")
                .font(.system(.title2, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)
                .frame(maxWidth: .infinity, alignment: .center)

            if let invite = manager.state.publicInvite {
                Text("Share an invite link to start a chat")
                    .font(.system(.footnote, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .frame(maxWidth: .infinity, alignment: .center)

                HStack(spacing: 10) {
                    Button("Copy") {
                        manager.copyToClipboard(invite.url)
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("newChatInviteCopyButton")

                    Button(action: { showingInviteQr = true }) {
                        Image(systemName: "qrcode")
                            .font(.system(size: 16, weight: .semibold))
                            .frame(width: 36, height: 36)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(IrisSecondaryButtonStyle())
                    .accessibilityIdentifier("newChatInviteQrButton")
                    .accessibilityLabel("Show QR Code")
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

            TextField("Paste invite link", text: $peerInput)
                .irisIdentifierInputModifiers()
                .textFieldStyle(.plain)
                .irisInputField()
                .accessibilityIdentifier("newChatPeerInput")

            if irisSupportsQrScanning {
                Button(action: { showingScanner = true }) {
                    HStack(spacing: 8) {
                        Image(systemName: "qrcode.viewfinder")
                        Text("Scan QR Code")
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
                    .foregroundStyle(palette.accent)
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
        .buttonStyle(.plain)
        .accessibilityIdentifier("newChatNewGroupButton")
    }

    @ViewBuilder
    private var inviteQrSheet: some View {
        if let invite = manager.state.publicInvite {
            VStack(spacing: 18) {
                Text("Invite QR Code")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                    .foregroundStyle(palette.textPrimary)

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

struct CreateInviteScreen: View {
    @ObservedObject var manager: AppManager
    @State private var shareText: String?

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

                    TextField("Group name", text: $name)
                        .textFieldStyle(.plain)
                        .irisInputField()
                        .accessibilityIdentifier("newGroupNameInput")
                }
            } trailing: {
                IrisSectionCard {
                    CardHeader(title: "Add members")

                    TextField("User ID or nostr:…", text: $memberInput)
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
                                            .foregroundStyle(palette.accent)
                                    }
                                }
                                .padding(.vertical, 10)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
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
                            subtitle: "Approve a new member by scan or paste."
                        )

                        TextField("Member user ID or nostr:…", text: $memberInput)
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

                    TextField("Device ID or approval code", text: $deviceInput)
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
    @State private var profilePictureViewerURL: URL?
    @State private var newRelayURL = ""
    @State private var editingRelayURL: String?
    @State private var editingRelayDraft = ""

    var body: some View {
        ZStack {
            BackgroundFill()

            IrisScrollScreen {
                VStack(alignment: .leading, spacing: 18) {
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

    private func relayRowStatusColor(_ relayURL: String, status: NetworkStatusSnapshot?, palette: IrisPalette) -> Color {
        guard let status, status.relayUrls.contains(relayURL) else {
            return palette.muted.opacity(0.55)
        }
        return relayStatusColor(status, palette: palette)
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
                    Text(account.displayName.isEmpty ? "Owner profile" : account.displayName)
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

            MonoValue(label: "Device ID", value: account.deviceNpub)

            VStack(spacing: 10) {
                Button("Copy user ID") { manager.copyToClipboard(account.npub) }
                    .buttonStyle(IrisSecondaryButtonStyle())
                Button("Copy device ID") { manager.copyToClipboard(account.deviceNpub) }
                    .buttonStyle(IrisSecondaryButtonStyle())
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
        if let testPath = ProcessInfo.processInfo.environment["NDR_UI_TEST_PROFILE_PICTURE_PATH"],
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
            .buttonStyle(.plain)
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
