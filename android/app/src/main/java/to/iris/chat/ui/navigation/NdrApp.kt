package to.iris.chat.ui.navigation

import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlin.math.max
import kotlinx.coroutines.delay
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.core.AppContainer
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.ui.components.IrisOfflineBannerState
import to.iris.chat.ui.components.LocalIrisOfflineBannerState
import to.iris.chat.ui.screens.ChatListScreen
import to.iris.chat.ui.screens.ChatScreen
import to.iris.chat.ui.screens.CreateAccountScreen
import to.iris.chat.ui.screens.CreateInviteScreen
import to.iris.chat.ui.screens.DeviceRevokedScreen
import to.iris.chat.ui.screens.DeviceRosterScreen
import to.iris.chat.ui.screens.GroupDetailsScreen
import to.iris.chat.ui.screens.JoinInviteScreen
import to.iris.chat.ui.screens.NewChatScreen
import to.iris.chat.ui.screens.NewGroupScreen
import to.iris.chat.ui.screens.NearbyIrisSheet
import to.iris.chat.ui.screens.MyProfileSheet
import to.iris.chat.ui.screens.RestoreAccountScreen
import to.iris.chat.ui.screens.SplashScreen
import to.iris.chat.ui.screens.SplashViewModel
import to.iris.chat.ui.screens.AwaitingDeviceApprovalScreen
import to.iris.chat.ui.screens.AddDeviceScreen
import to.iris.chat.ui.screens.WelcomeScreen

@Composable
fun NdrApp(
    container: AppContainer,
    onNearbyVisibilityChange: (Boolean) -> Unit = { container.nearbyIrisService.setVisible(it) },
    onNearbyLanVisibilityChange: (Boolean) -> Unit = { visible ->
        container.nearbyIrisService.setLocalNetworkVisible(visible)
        container.appManager.dispatch(AppAction.SetNearbyLanEnabled(visible))
    },
    onNearbyOpen: () -> Unit = {},
) {
    val appManager = container.appManager
    val splashViewModel = remember { SplashViewModel(appManager) }
    val bootstrapState by splashViewModel.bootstrapState.collectAsStateWithLifecycle()
    val router by appManager.router.collectAsStateWithLifecycle()
    val preferences by appManager.preferences.collectAsStateWithLifecycle()
    val networkStatus by appManager.networkStatus.collectAsStateWithLifecycle()
    val toast by appManager.toast.collectAsStateWithLifecycle()
    val foregroundedAtSecs by appManager.foregroundedAtSecs.collectAsStateWithLifecycle()
    val pendingShare by appManager.pendingShare.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var showingNearbyIris by remember { mutableStateOf(false) }
    var offlineNowSecs by remember { mutableStateOf(System.currentTimeMillis() / 1_000L) }
    val nearbySnapshotProvider =
        remember(container.nearbyIrisService) {
            { container.nearbyIrisService.snapshot }
        }
    val openNearbyIris = {
        onNearbyOpen()
        showingNearbyIris = true
    }

    LaunchedEffect(preferences.nearbyBluetoothEnabled) {
        if (
            preferences.nearbyBluetoothEnabled &&
                container.nearbyIrisService.hasBluetoothPermission()
        ) {
            container.nearbyIrisService.setVisible(true)
        } else {
            container.nearbyIrisService.setVisible(false)
        }
    }

    LaunchedEffect(preferences.nearbyLanEnabled) {
        container.nearbyIrisService.setLocalNetworkVisible(preferences.nearbyLanEnabled)
    }

    LaunchedEffect(toast) {
        val message = toast ?: return@LaunchedEffect
        Toast.makeText(context, message, Toast.LENGTH_LONG).show()
    }

    val offlineSinceSecs = networkStatus?.allRelaysOfflineSinceSecs?.toLong()
    val allRelaysOffline =
        networkStatus?.let { status ->
            status.relayUrls.isNotEmpty() &&
                status.connectedRelayCount == 0uL &&
                offlineSinceSecs != null
        } == true
    LaunchedEffect(allRelaysOffline, offlineSinceSecs, foregroundedAtSecs) {
        val current = currentTimeSeconds()
        offlineNowSecs = current
        val deadline =
            offlineBannerDeadlineSecs(
                allRelaysOffline = allRelaysOffline,
                offlineSinceSecs = offlineSinceSecs,
                foregroundedAtSecs = foregroundedAtSecs,
            )
        if (deadline != null && current < deadline) {
            delay((deadline - current) * 1_000L)
            offlineNowSecs = currentTimeSeconds()
        }
    }
    val offlineBannerState =
        if (
            allRelaysOffline &&
            offlineSinceSecs != null &&
            offlineNowSecs.saturatingSubtract(offlineSinceSecs) >= OFFLINE_BANNER_GRACE_SECS &&
            offlineNowSecs.saturatingSubtract(foregroundedAtSecs) >= OFFLINE_BANNER_GRACE_SECS
        ) {
            val nearbySnapshot = nearbySnapshotProvider()
            val bluetoothState = if (nearbyBluetoothEnabled(nearbySnapshot)) "on" else "off"
            val wifiState = if (nearbyWifiEnabled(nearbySnapshot)) "on" else "off"
            IrisOfflineBannerState("Offline · Bluetooth $bluetoothState · Wi-Fi $wifiState")
        } else {
            null
        }

    val activeScreen = router.screenStack.lastOrNull() ?: router.defaultScreen

    BackHandler(enabled = bootstrapState != AccountBootstrapState.Loading && router.screenStack.isNotEmpty()) {
        appManager.dispatch(AppAction.NavigateBack)
    }

    CompositionLocalProvider(LocalIrisOfflineBannerState provides offlineBannerState) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.background),
        ) {
            when (bootstrapState) {
                AccountBootstrapState.Loading -> {
                    SplashScreen(
                        bootstrapState = bootstrapState,
                        onNeedsLogin = {},
                        onLoggedIn = {},
                    )
                }

                AccountBootstrapState.NeedsLogin -> {
                    when (activeScreen) {
                        Screen.Welcome -> WelcomeScreen(appManager = appManager)
                        Screen.CreateAccount -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            CreateAccountScreen(appManager = appManager, appState = appState)
                        }
                        Screen.RestoreAccount -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            RestoreAccountScreen(appManager = appManager, appState = appState)
                        }
                        Screen.AddDevice -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            AddDeviceScreen(appManager = appManager, appState = appState, awaitingApproval = false)
                        }
                        else -> WelcomeScreen(appManager = appManager)
                    }
                }

                is AccountBootstrapState.LoggedIn -> {
                    when (val screen = activeScreen) {
                        Screen.Welcome -> {
                            WelcomeScreen(appManager = appManager)
                        }

                        Screen.CreateAccount -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            CreateAccountScreen(appManager = appManager, appState = appState)
                        }

                        Screen.RestoreAccount -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            RestoreAccountScreen(appManager = appManager, appState = appState)
                        }

                        Screen.AddDevice -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            AddDeviceScreen(appManager = appManager, appState = appState, awaitingApproval = false)
                        }

                        Screen.ChatList -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            ChatListScreen(
                                appManager = appManager,
                                appState = appState,
                                nearbyService = container.nearbyIrisService,
                                onNearbyClick = openNearbyIris,
                            )
                        }

                        Screen.NewChat -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            NewChatScreen(appManager = appManager, appState = appState)
                        }

                        Screen.NewGroup -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            NewGroupScreen(appManager = appManager, appState = appState)
                        }

                        Screen.CreateInvite -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            CreateInviteScreen(appManager = appManager, appState = appState)
                        }

                        Screen.JoinInvite -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            JoinInviteScreen(appManager = appManager, appState = appState)
                        }

                        Screen.Settings -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            val account = appState.account
                            if (account == null) {
                                ChatListScreen(
                                    appManager = appManager,
                                    appState = appState,
                                    nearbyService = container.nearbyIrisService,
                                    onNearbyClick = openNearbyIris,
                                )
                            } else {
                                MyProfileSheet(
                                    appManager = appManager,
                                    npub = account.npub,
                                    displayName = account.displayName,
                                    pictureUrl = account.pictureUrl,
                                    deviceNpub = account.deviceNpub,
                                    canManageDevices = account.hasOwnerSigningAuthority,
                                    sendTypingIndicators = appState.preferences.sendTypingIndicators,
                                    sendReadReceipts = appState.preferences.sendReadReceipts,
                                    desktopNotificationsEnabled = appState.preferences.desktopNotificationsEnabled,
                                    imageProxyEnabled = appState.preferences.imageProxyEnabled,
                                    imageProxyUrl = appState.preferences.imageProxyUrl,
                                    imageProxyKeyHex = appState.preferences.imageProxyKeyHex,
                                    imageProxySaltHex = appState.preferences.imageProxySaltHex,
                                    preferences = appState.preferences,
                                    networkStatus = appState.networkStatus,
                                    onNearbyBluetoothChange = onNearbyVisibilityChange,
                                    onNearbyLanChange = onNearbyLanVisibilityChange,
                                    onManageDevices = { appManager.pushScreen(Screen.DeviceRoster) },
                                    onLogout = { appManager.logout() },
                                    onDismiss = {
                                        appManager.dispatch(AppAction.NavigateBack)
                                    },
                                )
                            }
                        }

                        Screen.DeviceRoster -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            DeviceRosterScreen(appManager = appManager, appState = appState)
                        }

                        Screen.AwaitingDeviceApproval -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            AwaitingDeviceApprovalScreen(appManager = appManager, appState = appState)
                        }

                        Screen.DeviceRevoked -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            DeviceRevokedScreen(appManager = appManager, appState = appState)
                        }

                        is Screen.Chat -> {
                            // ChatScreen takes only `(appManager, chatId)` and
                            // collects its own state slices internally. Passing
                            // `appState` here would invalidate ChatScreen's
                            // memoization on every relay event.
                            ChatScreen(
                                appManager = appManager,
                                chatId = screen.chatId,
                            )
                        }

                        is Screen.GroupDetails -> {
                            val appState by appManager.state.collectAsStateWithLifecycle()
                            GroupDetailsScreen(
                                appManager = appManager,
                                appState = appState,
                                groupId = screen.groupId,
                            )
                        }
                    }
                }
            }

            if (showingNearbyIris) {
                val appState by appManager.state.collectAsStateWithLifecycle()
                NearbyIrisSheet(
                    appManager = appManager,
                    appState = appState,
                    service = container.nearbyIrisService,
                    onVisibleChange = onNearbyVisibilityChange,
                    onLocalNetworkVisibleChange = onNearbyLanVisibilityChange,
                    onDismiss = { showingNearbyIris = false },
                )
            }

            if (pendingShare != null && bootstrapState is AccountBootstrapState.LoggedIn) {
                val chatList by appManager.chatList.collectAsStateWithLifecycle()
                ShareTargetDialog(
                    chats = chatList,
                    onSend = { chatIds -> appManager.sendPendingShareToChats(chatIds) },
                    onNewChat = {
                        appManager.clearPendingShare()
                        appManager.pushScreen(Screen.NewChat)
                    },
                    onDismiss = { appManager.clearPendingShare() },
                )
            }
        }
    }
}

@Composable
private fun ShareTargetDialog(
    chats: List<ChatThreadSnapshot>,
    onSend: (List<String>) -> Unit,
    onNewChat: () -> Unit,
    onDismiss: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var selectedChatIds by remember { mutableStateOf(emptySet<String>()) }
    val availableChatIds = remember(chats) { chats.mapTo(mutableSetOf()) { it.chatId } }
    val filteredChats =
        remember(chats, query) {
            val normalized = query.trim().lowercase()
            if (normalized.isEmpty()) {
                chats
            } else {
                chats.filter { chat -> chat.matchesShareQuery(normalized) }
            }
        }

    LaunchedEffect(availableChatIds) {
        val prunedSelection = selectedChatIds.intersect(availableChatIds)
        if (prunedSelection.size != selectedChatIds.size) {
            selectedChatIds = prunedSelection
        }
    }

    if (chats.isEmpty()) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text("Start a chat first") },
            confirmButton = {
                TextButton(onClick = onNewChat) {
                    Text("New chat")
                }
            },
            dismissButton = {
                TextButton(onClick = onDismiss) {
                    Text("Cancel")
                }
            },
        )
        return
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Choose recipients") },
        text = {
            Column {
                TextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.fillMaxWidth(),
                    placeholder = { Text("Search") },
                    singleLine = true,
                )
                LazyColumn(modifier = Modifier.heightIn(max = 380.dp)) {
                    items(filteredChats, key = { it.chatId }) { chat ->
                        val selected = chat.chatId in selectedChatIds
                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        selectedChatIds =
                                            if (selected) {
                                                selectedChatIds - chat.chatId
                                            } else {
                                                selectedChatIds + chat.chatId
                                            }
                                    }
                                    .padding(vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Checkbox(
                                checked = selected,
                                onCheckedChange = { checked ->
                                    selectedChatIds =
                                        if (checked) {
                                            selectedChatIds + chat.chatId
                                        } else {
                                            selectedChatIds - chat.chatId
                                        }
                                },
                            )
                            Spacer(Modifier.width(8.dp))
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = chat.displayName,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                                val subtitle = chat.subtitle
                                if (subtitle?.isNotBlank() == true) {
                                    Text(
                                        text = subtitle,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                        style = MaterialTheme.typography.bodySmall,
                                    )
                                }
                            }
                        }
                    }
                    if (filteredChats.isEmpty()) {
                        item {
                            Text(
                                text = "No matches",
                                modifier = Modifier.padding(vertical = 16.dp),
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = selectedChatIds.isNotEmpty(),
                onClick = {
                    onSend(chats.map { it.chatId }.filter { it in selectedChatIds })
                },
            ) {
                Text(
                    text =
                        if (selectedChatIds.size > 1) {
                            "Send (${selectedChatIds.size})"
                        } else {
                            "Send"
                        },
                )
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

private fun ChatThreadSnapshot.matchesShareQuery(query: String): Boolean =
    displayName.lowercase().contains(query) ||
        (subtitle?.lowercase()?.contains(query) == true) ||
        (lastMessagePreview?.lowercase()?.contains(query) == true)

private fun nearbyWifiEnabled(snapshot: IrisNearbyService.Snapshot): Boolean =
    snapshot.localNetworkVisible &&
        snapshot.localNetworkStatus in nearbyWifiOnStatuses

private fun nearbyBluetoothEnabled(snapshot: IrisNearbyService.Snapshot): Boolean =
    snapshot.visible &&
        snapshot.status !in nearbyBluetoothBlockingStatuses &&
        snapshot.status !in nearbyTransportOffStatuses

private val nearbyBluetoothBlockingStatuses =
    setOf(
        "No Bluetooth access",
        "Bluetooth off",
        "Bluetooth unavailable",
        "Advertise unavailable",
        "Advertise failed",
        "Scan failed",
        "Connect failed",
    )

private val nearbyTransportOffStatuses =
    setOf(
        "Off",
        "Starting",
    )

private val nearbyWifiOnStatuses =
    setOf(
        "Visible",
        "Connected",
    )

private fun Long.saturatingSubtract(other: Long): Long =
    if (this >= other) this - other else 0L

private fun currentTimeSeconds(): Long = System.currentTimeMillis() / 1_000L

private fun offlineBannerDeadlineSecs(
    allRelaysOffline: Boolean,
    offlineSinceSecs: Long?,
    foregroundedAtSecs: Long,
): Long? {
    if (!allRelaysOffline || offlineSinceSecs == null) {
        return null
    }
    return max(
        offlineSinceSecs + OFFLINE_BANNER_GRACE_SECS,
        foregroundedAtSecs + OFFLINE_BANNER_GRACE_SECS,
    )
}

private const val OFFLINE_BANNER_GRACE_SECS = 30L
