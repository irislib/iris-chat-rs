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
func mobileWifiEnabled(_ service: IrisNearbyService) -> Bool {
    service.isLanVisible && !mobileWifiBlockingStatuses.contains(service.lanStatus)
}

let mobileWifiBlockingStatuses: Set<String> = [
    "No local network access",
    "Local network unavailable",
    "Local network failed"
]
#endif

struct DesktopChatShell: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let onOpenNearby: () -> Void
    let onOpenNearbyPeerProfile: (String) -> Void

    var body: some View {
        HStack(spacing: 0) {
            DesktopChatSidebar(
                manager: manager,
                onOpenNearby: onOpenNearby,
                onOpenNearbyPeerProfile: onOpenNearbyPeerProfile
            )
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
                            manager.dispatch(.pushScreen(screen: .directChatInfo(chatId: current.chatId)))
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
                trailing: AnyView(
                    InChatSearchButton(
                        manager: manager,
                        target: InChatSearchTarget(
                            chatId: chatId,
                            displayName: chat?.displayName ?? "Chat"
                        )
                    )
                )
            )
            ChatScreen(manager: manager, chatId: chatId)
                .id(chatId)
        case .directChatInfo(let chatId):
            DesktopPaneTopBar(title: manager.state.currentChat?.displayName ?? "Details", canGoBack: true, onBack: manager.navigateBack)
            DirectChatInfoScreen(
                manager: manager,
                chatId: chatId,
                onClose: manager.navigateBack,
                showMessageAction: true,
                onMessage: {
                    manager.dispatch(.openChat(chatId: chatId))
                }
            )
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

struct DesktopPaneTopBar: View {
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

struct DesktopChatSidebar: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var manager: AppManager
    let onOpenNearby: () -> Void
    let onOpenNearbyPeerProfile: (String) -> Void
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
                    let preferences = manager.state.preferences

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
                    DesktopNearbyIrisRow(
                        manager: manager,
                        service: manager.nearbyIris,
                        onOpen: onOpenNearby,
                        onOpenPeerProfile: onOpenNearbyPeerProfile
                    )
                        .accessibilityIdentifier("desktopNearbyRow")
                    #endif

                    ForEach(filteredChats, id: \.chatId) { chat in
                        DesktopSidebarChatRow(
                            manager: manager,
                            chat: chat,
                            timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                            selected: selectedChatId == chat.chatId,
                            preferences: preferences
                        )
                        .equatable()
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
                            emphasize: false,
                            pictureUrl: account.pictureUrl,
                            preferences: manager.state.preferences,
                            manager: manager
                        )
                    }
                    .buttonStyle(.irisUnpressed)
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

struct DesktopSidebarActionRow: View {
    @Environment(\.irisPalette) private var palette

    let title: String
    let subtitle: String?
    let systemImage: String
    let selected: Bool
    var longPressAction: (() -> Void)? = nil
    let action: () -> Void

    var body: some View {
        if let longPressAction {
            rowContent
                .onTapGesture(perform: action)
                .onLongPressGesture(minimumDuration: 0.5, perform: longPressAction)
                .accessibilityAddTraits(.isButton)
        } else {
            Button(action: action) {
                rowContent
            }
            .buttonStyle(.irisPlain)
        }
    }

    private var rowContent: some View {
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

    private var rowBackground: some View {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
            .fill(selected ? palette.panelAlt : Color.clear)
    }
}

#if os(macOS)
struct DesktopNearbyIrisRow: View {
    @ObservedObject var manager: AppManager
    @ObservedObject var service: IrisNearbyService
    let onOpen: () -> Void
    let onOpenPeerProfile: (String) -> Void

    var body: some View {
        let nearbyEnabled = manager.state.preferences.nearbyEnabled
        let active = nearbyEnabled && service.isNearbyActive
        let peers = nearbyEnabled ? service.peers : []

        if peers.isEmpty {
            DesktopSidebarActionRow(
                title: "Nearby",
                subtitle: nearbyEnabled ? service.sidebarSubtitle : "Off",
                systemImage: "dot.radiowaves.left.and.right",
                selected: false,
                longPressAction: { toggleNearbyMaster(manager: manager) }
            ) {
                onOpen()
            }
        } else {
            NearbyPeerStripRow(
                manager: manager,
                peers: peers,
                avatarSize: 42,
                horizontalPadding: 10,
                verticalPadding: 8,
                rowHeight: nil,
                active: active,
                onOpenNearby: onOpen,
                onOpenPeerProfile: onOpenPeerProfile
            )
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.clear)
            )
        }
    }
}
#endif

struct DesktopSidebarChatRow: View, Equatable {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let chat: ChatThreadSnapshot
    let timeLabel: String?
    let selected: Bool
    let preferences: PreferencesSnapshot
    @State private var confirmingDelete = false

    static func == (lhs: DesktopSidebarChatRow, rhs: DesktopSidebarChatRow) -> Bool {
        lhs.chat == rhs.chat
            && lhs.timeLabel == rhs.timeLabel
            && lhs.selected == rhs.selected
            && lhs.preferences == rhs.preferences
    }

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
                    preferences: preferences,
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
            chatListItemContextMenu(manager: manager, chat: chat) {
                confirmingDelete = true
            }
        }
        .confirmationDialog(
            "Delete chat?",
            isPresented: $confirmingDelete,
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                manager.dispatch(.deleteChat(chatId: chat.chatId))
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes messages from this device.")
        }
    }
}

@ViewBuilder
func chatListItemContextMenu(
    manager: AppManager,
    chat: ChatThreadSnapshot,
    onDeleteRequest: @escaping () -> Void
) -> some View {
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

    Button(role: .destructive, action: onDeleteRequest) {
        Label("Delete", systemImage: "trash.fill")
    }
}

struct IrisContextMenuLabel: View {
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
struct DesktopUpdateStripe: View {
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
