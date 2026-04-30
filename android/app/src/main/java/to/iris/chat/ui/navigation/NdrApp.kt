package to.iris.chat.ui.navigation

import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import kotlinx.coroutines.delay
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.core.AppContainer
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
) {
    val appManager = container.appManager
    val splashViewModel = remember { SplashViewModel(appManager) }
    val bootstrapState by splashViewModel.bootstrapState.collectAsStateWithLifecycle()
    val appState by appManager.state.collectAsStateWithLifecycle()
    val pendingShare by appManager.pendingShare.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var showingNearbyIris by remember { mutableStateOf(false) }
    var offlineNowSecs by remember { mutableStateOf(System.currentTimeMillis() / 1_000L) }
    val nearbyBluetoothOnProvider =
        remember(container.nearbyIrisService) {
            { container.nearbyIrisService.snapshot.bluetoothOn }
        }
    val openNearbyIris = {
        showingNearbyIris = true
    }

    LaunchedEffect(appState.preferences.nearbyBluetoothEnabled) {
        if (
            appState.preferences.nearbyBluetoothEnabled &&
                container.nearbyIrisService.hasBluetoothPermission()
        ) {
            container.nearbyIrisService.setVisible(true)
        }
    }

    LaunchedEffect(appState.preferences.nearbyLanEnabled) {
        container.nearbyIrisService.setLocalNetworkVisible(appState.preferences.nearbyLanEnabled)
    }

    LaunchedEffect(appState.toast) {
        val message = appState.toast ?: return@LaunchedEffect
        Toast.makeText(context, message, Toast.LENGTH_LONG).show()
    }

    val offlineSinceSecs = appState.networkStatus?.allRelaysOfflineSinceSecs?.toLong()
    val allRelaysOffline =
        appState.networkStatus?.let { status ->
            status.relayUrls.isNotEmpty() &&
                status.connectedRelayCount == 0uL &&
                offlineSinceSecs != null
        } == true
    LaunchedEffect(allRelaysOffline, offlineSinceSecs) {
        while (allRelaysOffline) {
            offlineNowSecs = System.currentTimeMillis() / 1_000L
            delay(1_000L)
        }
    }
    val offlineBannerState =
        if (
            allRelaysOffline &&
            offlineSinceSecs != null &&
            offlineNowSecs.saturatingSubtract(offlineSinceSecs) >= 5L
        ) {
            val bluetoothState = if (nearbyBluetoothOnProvider()) "on" else "off"
            IrisOfflineBannerState("Offline, Bluetooth $bluetoothState")
        } else {
            null
        }

    val router = appState.router
    val activeScreen = router.screenStack.lastOrNull() ?: router.defaultScreen

    BackHandler(enabled = bootstrapState != AccountBootstrapState.Loading && router.screenStack.isNotEmpty()) {
        appManager.dispatch(AppAction.UpdateScreenStack(router.screenStack.dropLast(1)))
    }

    CompositionLocalProvider(LocalIrisOfflineBannerState provides offlineBannerState) {
        Box {
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
                        Screen.CreateAccount -> CreateAccountScreen(appManager = appManager, appState = appState)
                        Screen.RestoreAccount -> RestoreAccountScreen(appManager = appManager, appState = appState)
                        Screen.AddDevice -> AddDeviceScreen(appManager = appManager, appState = appState, awaitingApproval = false)
                        else -> WelcomeScreen(appManager = appManager)
                    }
                }

                is AccountBootstrapState.LoggedIn -> {
                    when (val screen = activeScreen) {
                        Screen.Welcome -> {
                            WelcomeScreen(appManager = appManager)
                        }

                        Screen.CreateAccount -> {
                            CreateAccountScreen(appManager = appManager, appState = appState)
                        }

                        Screen.RestoreAccount -> {
                            RestoreAccountScreen(appManager = appManager, appState = appState)
                        }

                        Screen.AddDevice -> {
                            AddDeviceScreen(appManager = appManager, appState = appState, awaitingApproval = false)
                        }

                        Screen.ChatList -> {
                            ChatListScreen(
                                appManager = appManager,
                                appState = appState,
                                nearbyService = container.nearbyIrisService,
                                onNearbyClick = openNearbyIris,
                            )
                        }

                        Screen.NewChat -> {
                            NewChatScreen(appManager = appManager, appState = appState)
                        }

                        Screen.NewGroup -> {
                            NewGroupScreen(appManager = appManager, appState = appState)
                        }

                        Screen.CreateInvite -> {
                            CreateInviteScreen(appManager = appManager, appState = appState)
                        }

                        Screen.JoinInvite -> {
                            JoinInviteScreen(appManager = appManager, appState = appState)
                        }

                        Screen.Settings -> {
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
                                    bluetoothOnProvider = nearbyBluetoothOnProvider,
                                    onNearbyBluetoothChange = onNearbyVisibilityChange,
                                    onNearbyLanChange = onNearbyLanVisibilityChange,
                                    onManageDevices = { appManager.pushScreen(Screen.DeviceRoster) },
                                    onLogout = { appManager.logout() },
                                    onDismiss = {
                                        appManager.dispatch(
                                            AppAction.UpdateScreenStack(router.screenStack.dropLast(1)),
                                        )
                                    },
                                )
                            }
                        }

                        Screen.DeviceRoster -> {
                            DeviceRosterScreen(appManager = appManager, appState = appState)
                        }

                        Screen.AwaitingDeviceApproval -> {
                            AwaitingDeviceApprovalScreen(appManager = appManager, appState = appState)
                        }

                        Screen.DeviceRevoked -> {
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
                ShareTargetDialog(
                    chats = appState.chatList,
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
    var selectedChatIds by remember(chats) { mutableStateOf(emptySet<String>()) }
    val filteredChats =
        remember(chats, query) {
            val normalized = query.trim().lowercase()
            if (normalized.isEmpty()) {
                chats
            } else {
                chats.filter { chat -> chat.matchesShareQuery(normalized) }
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

private fun Long.saturatingSubtract(other: Long): Long =
    if (this >= other) this - other else 0L
