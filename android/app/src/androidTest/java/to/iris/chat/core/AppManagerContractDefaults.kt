package to.iris.chat.core

import to.iris.chat.rust.AppState
import to.iris.chat.rust.BusyState
import to.iris.chat.rust.Router
import to.iris.chat.rust.Screen
import to.iris.chat.rust.MobilePushSyncSnapshot
import to.iris.chat.rust.PreferencesSnapshot

internal object AppManagerContractDefaults {
    fun initialState(): AppState =
        AppState(
            call = null, rev = 0u,
            router = Router(Screen.Welcome, emptyList()),
            account = null,
            deviceRoster = null,
            deviceHistorySync = null,
            busy =
                BusyState(
                    creatingAccount = false,
                    restoringSession = false,
                    linkingDevice = false,
                    creatingChat = false,
                    creatingGroup = false,
                    sendingMessage = false,
                    updatingRoster = false,
                    updatingGroup = false,
                    creatingInvite = false,
                    acceptingInvite = false,
                    syncingNetwork = false,
                    uploadingAttachment = false,
                    uploadProgress = null,
                ),
            chatList = emptyList(),
            currentChat = null,
            groupDetails = null,
            publicInvite = null,
            linkDevice = null, remoteSignerLogin = null,
            networkStatus = null,
            mobilePush = MobilePushSyncSnapshot(null, emptyList(), null, emptyList(), emptyList(), emptyList(), emptyList(), emptyList()),
            userDiscoveryRevision = 0u,
            userDiscoverySyncing = false,
            preferences = PreferencesSnapshot(
                    voiceCallsEnabled = true, videoCallsEnabled = true, sendTypingIndicators = true,
                    callQuality = "auto", callMaxBitrateBps = 2_000_000u,
                    sendReadReceipts = true,
                    desktopNotificationsEnabled = true,
                    inviteAcceptanceNotificationsEnabled = true,
                    startupAtLoginEnabled = false,
                    nearbyEnabled = true,
                    nearbyBluetoothEnabled = false,
                    nearbyLanEnabled = false,
                    nearbyShowInChatList = true,
                    nearbyMailbagEnabled = true,
                    nostrRelayUrls =
                        listOf(
                            "wss://relay.damus.io",
                            "wss://nos.lol",
                            "wss://relay.primal.net",
                            "wss://relay.snort.social",
                            "wss://temp.iris.to",
                        ),
                    imageProxyEnabled = true,
                    imageProxyFallbackEnabled = false,
                    imageProxyUrl = "https://imgproxy.iris.to",
                    imageProxyKeyHex = "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                    imageProxySaltHex = "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
                    mutedChatIds = emptyList(),
                timedChatMutes = emptyList(),
                    pinnedChatIds = emptyList(),
                    blockedOwnerPubkeys = emptyList(),
                    acceptedOwnerPubkeys = emptyList(),
                    debugLoggingEnabled = false,
                    acceptUnknownDirectMessages = true,
                    mobilePushServerUrl = "",
                ),
            toast = null,
        )
}
