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

struct RootView: View {
    @ObservedObject var manager: AppManager
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
                            onOpenNearby: openNearbyIris,
                            onOpenNearbyPeerProfile: openNearbyPeerProfile
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
                    openPeerProfile: openNearbyPeerProfile,
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
        case .chatList, .newChat, .newGroup, .createInvite, .joinInvite, .settings, .chat, .directChatInfo, .groupDetails, .deviceRoster:
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
            ChatListScreen(
                manager: manager,
                onOpenNearby: openNearbyIris,
                onOpenNearbyPeerProfile: openNearbyPeerProfile
            )
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
        case .directChatInfo(let chatId):
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
                    // emphasize=false → background is the neutral panel
                    // tint, not brand purple. With emphasize on, the
                    // button-press opacity dip made the purple bleed
                    // through the loaded picture, which read as a weird
                    // purple flash on tap.
                    emphasize: false,
                    pictureUrl: account.pictureUrl,
                    preferences: manager.state.preferences,
                    manager: manager,
                    loadedImageIdentifier: "chatListProfileAvatarImage"
                )
            }
            .buttonStyle(.irisUnpressed)
            .accessibilityIdentifier("chatListProfileButton")
            .accessibilityValue(
                ProcessInfo.processInfo.environment["IRIS_UI_TEST_EXPOSE_ACCOUNT_NPUB"] == "1"
                    ? account.npub
                    : ""
            )
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
        // Direct chat — push the info route so back navigation lands on the chat.
        let chatId = chat.chatId
        return { [weak manager] in
            manager?.dispatch(.pushScreen(screen: .directChatInfo(chatId: chatId)))
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
                bluetoothEnabled: manager.state.preferences.nearbyEnabled &&
                    manager.state.preferences.nearbyBluetoothEnabled,
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
        case .directChatInfo:
            return manager.state.currentChat?.displayName ?? "Details"
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
        showingNearbyIris = true
#endif
    }

#if os(iOS) || os(macOS)
    private func openNearbyPeerProfile(_ ownerPubkeyHex: String) {
        showingNearbyIris = false
        manager.dispatch(.pushScreen(screen: .directChatInfo(chatId: ownerPubkeyHex)))
    }
#endif
}
