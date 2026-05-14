package to.iris.chat.ui.navigation

import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlin.math.abs
import kotlin.math.max
import kotlinx.coroutines.delay
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.core.AppContainer
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisOfflineBannerState
import to.iris.chat.ui.components.LocalIrisOfflineBannerState
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme
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
import to.iris.chat.ui.screens.rememberNhashImageData

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
    val activeRoute =
        remember(activeScreen, router.screenStack.size) {
            RouteTransitionTarget(
                screen = activeScreen,
                depth = router.screenStack.size,
                key = screenRouteKey(activeScreen),
            )
        }
    val canNavigateBack =
        bootstrapState != AccountBootstrapState.Loading && router.screenStack.isNotEmpty()

    BackHandler(enabled = canNavigateBack) {
        appManager.navigateBack()
    }

    CompositionLocalProvider(LocalIrisOfflineBannerState provides offlineBannerState) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .edgeSwipeBack(enabled = canNavigateBack) { appManager.navigateBack() }
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
                    AnimatedRoute(target = activeRoute) { screen ->
                        when (screen) {
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
                }

                is AccountBootstrapState.LoggedIn -> {
                    AnimatedRoute(target = activeRoute) { screen ->
                        when (screen) {
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
                                        onDismiss = { appManager.navigateBack() },
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
                    appManager = appManager,
                    chats = chatList,
                    preferences = preferences,
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

private data class RouteTransitionTarget(
    val screen: Screen,
    val depth: Int,
    val key: String,
)

@Composable
private fun Modifier.edgeSwipeBack(
    enabled: Boolean,
    onBack: () -> Unit,
): Modifier {
    if (!enabled) {
        return this
    }
    val density = LocalDensity.current
    val edgeWidthPx = with(density) { 28.dp.toPx() }
    val triggerPx = with(density) { 72.dp.toPx() }
    val horizontalConsumePx = with(density) { 18.dp.toPx() }
    val verticalCancelPx = with(density) { 28.dp.toPx() }

    return pointerInput(onBack, edgeWidthPx, triggerPx, horizontalConsumePx, verticalCancelPx) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val fromLeftEdge = down.position.x <= edgeWidthPx
            val fromRightEdge = down.position.x >= size.width - edgeWidthPx
            if (!fromLeftEdge && !fromRightEdge) {
                return@awaitEachGesture
            }

            val direction = if (fromLeftEdge) 1f else -1f
            val pointerId = down.id
            var totalX = 0f
            var totalY = 0f

            while (true) {
                val event = awaitPointerEvent()
                val change = event.changes.firstOrNull { it.id == pointerId } ?: break
                if (!change.pressed) {
                    break
                }

                val delta = change.positionChange()
                totalX += delta.x
                totalY += delta.y
                val directedX = totalX * direction
                if (abs(totalY) > verticalCancelPx && abs(totalY) > abs(totalX)) {
                    break
                }
                if (directedX > horizontalConsumePx) {
                    change.consume()
                }
                if (directedX > triggerPx && directedX > abs(totalY) * 1.2f) {
                    onBack()
                    change.consume()
                    break
                }
            }
        }
    }
}

@Composable
private fun AnimatedRoute(
    target: RouteTransitionTarget,
    content: @Composable (Screen) -> Unit,
) {
    AnimatedContent(
        targetState = target,
        transitionSpec = {
            routeContentTransform(initialState = initialState, targetState = targetState)
        },
        modifier = Modifier.fillMaxSize(),
        label = "IrisRouteTransition",
    ) { route ->
        content(route.screen)
    }
}

private fun routeContentTransform(
    initialState: RouteTransitionTarget,
    targetState: RouteTransitionTarget,
): ContentTransform {
    if (targetState.depth == initialState.depth) {
        return fadeIn(animationSpec = tween(durationMillis = 0))
            .togetherWith(fadeOut(animationSpec = tween(durationMillis = 0)))
    }
    val direction = if (targetState.depth < initialState.depth) -1 else 1
    return (
        slideInHorizontally(animationSpec = tween(durationMillis = 220)) { width ->
            width * direction
        } + fadeIn(animationSpec = tween(durationMillis = 90, delayMillis = 40))
    ).togetherWith(
        slideOutHorizontally(animationSpec = tween(durationMillis = 220)) { width ->
            -width * direction / 3
        } + fadeOut(animationSpec = tween(durationMillis = 90))
    )
}

private fun screenRouteKey(screen: Screen): String =
    when (screen) {
        Screen.Welcome -> "welcome"
        Screen.CreateAccount -> "createAccount"
        Screen.RestoreAccount -> "restoreAccount"
        Screen.AddDevice -> "addDevice"
        Screen.ChatList -> "chatList"
        Screen.NewChat -> "newChat"
        Screen.NewGroup -> "newGroup"
        Screen.CreateInvite -> "createInvite"
        Screen.JoinInvite -> "joinInvite"
        Screen.Settings -> "settings"
        Screen.DeviceRoster -> "deviceRoster"
        Screen.AwaitingDeviceApproval -> "awaitingDeviceApproval"
        Screen.DeviceRevoked -> "deviceRevoked"
        is Screen.Chat -> "chat:${screen.chatId}"
        is Screen.GroupDetails -> "groupDetails:${screen.groupId}"
    }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShareTargetDialog(
    appManager: AppManager,
    chats: List<ChatThreadSnapshot>,
    preferences: PreferencesSnapshot,
    onSend: (List<String>) -> Unit,
    onNewChat: () -> Unit,
    onDismiss: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var selectedChatIds by remember { mutableStateOf(emptySet<String>()) }
    val haptics = rememberIrisHapticFeedback()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false)
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
    val hasSelection = selectedChatIds.isNotEmpty()
    val selectedChats = chats.filter { chat -> chat.chatId in selectedChatIds }

    LaunchedEffect(availableChatIds) {
        val prunedSelection = selectedChatIds.intersect(availableChatIds)
        if (prunedSelection.size != selectedChatIds.size) {
            selectedChatIds = prunedSelection
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.background,
        contentColor = MaterialTheme.colorScheme.onSurface,
        dragHandle = { ShareTargetSheetHandle() },
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding(),
        ) {
            Text(
                text = "Share",
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 13.dp, bottom = 12.dp),
                textAlign = TextAlign.Center,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )

            if (chats.isEmpty()) {
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .height(220.dp),
                ) {
                    ShareEmptyChatsContent(onNewChat = onNewChat)
                }
            } else {
                ShareTargetSearchField(
                    query = query,
                    onQueryChange = { query = it },
                    onClear = { query = "" },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp)
                            .heightIn(min = 44.dp),
                )

                LazyColumn(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = 220.dp, max = 380.dp)
                            .padding(top = 12.dp),
                    contentPadding = PaddingValues(bottom = 10.dp),
                ) {
                    items(filteredChats, key = { it.chatId }) { chat ->
                        val selected = chat.chatId in selectedChatIds
                        ShareTargetRow(
                            appManager = appManager,
                            chat = chat,
                            preferences = preferences,
                            selected = selected,
                            onClick = {
                                haptics.press()
                                selectedChatIds =
                                    if (selected) {
                                        selectedChatIds - chat.chatId
                                    } else {
                                        selectedChatIds + chat.chatId
                                    }
                            },
                        )
                    }
                    if (filteredChats.isEmpty()) {
                        item {
                            Text(
                                text = "No matches",
                                modifier =
                                    Modifier
                                        .fillMaxWidth()
                                        .padding(horizontal = 24.dp, vertical = 24.dp),
                                textAlign = TextAlign.Center,
                                color = IrisTheme.palette.muted,
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }
                    }
                }

                if (hasSelection) {
                    ShareTargetBottomBar(
                        selectedChats = selectedChats,
                        onSend = {
                            haptics.confirm()
                            onSend(chats.map { it.chatId }.filter { it in selectedChatIds })
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun ShareTargetSheetHandle() {
    Box(
        modifier =
            Modifier
                .padding(top = 16.dp)
                .width(48.dp)
                .height(2.dp)
                .clip(CircleShape)
                .background(IrisTheme.palette.muted.copy(alpha = 0.55f)),
    )
}

@Composable
private fun ShareTargetSearchField(
    query: String,
    onQueryChange: (String) -> Unit,
    onClear: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberIrisHapticFeedback()
    val clearInteractionSource = remember { MutableInteractionSource() }
    Surface(
        modifier = modifier,
        color = IrisTheme.palette.panelAlt,
        shape = RoundedCornerShape(22.dp),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 44.dp)
                    .padding(start = 16.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Rounded.Search,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.size(24.dp),
            )
            Box(
                modifier =
                    Modifier
                        .weight(1f)
                        .padding(horizontal = 12.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (query.isEmpty()) {
                    Text(
                        text = "Search",
                        color = IrisTheme.palette.muted,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
                BasicTextField(
                    value = query,
                    onValueChange = onQueryChange,
                    singleLine = true,
                    textStyle =
                        MaterialTheme.typography.bodyLarge.copy(
                            color = MaterialTheme.colorScheme.onSurface,
                        ),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.onSurface),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            if (query.isNotEmpty()) {
                Box(
                    modifier =
                        Modifier
                            .size(40.dp)
                            .clip(CircleShape)
                            .clickable(
                                interactionSource = clearInteractionSource,
                                indication = null,
                            ) {
                                haptics.press()
                                onClear()
                            },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.Rounded.Close,
                        contentDescription = "Clear search",
                        tint = IrisTheme.palette.muted,
                        modifier = Modifier.size(24.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun ShareTargetRow(
    appManager: AppManager,
    chat: ChatThreadSnapshot,
    preferences: PreferencesSnapshot,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val avatarData by rememberNhashImageData(appManager, chat.pictureUrl)
    val avatarUrl =
        chat.pictureUrl
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
            ?.let { url ->
                proxiedImageUrl(
                    originalSrc = url,
                    preferences = preferences,
                    width = 80u,
                    height = 80u,
                    square = true,
                )
            }
    val interactionSource = remember(chat.chatId) { MutableInteractionSource() }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 64.dp)
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                    onClick = onClick,
                )
                .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IrisAvatar(
            label = chat.displayName,
            size = 40.dp,
            imageUrl = avatarUrl,
            imageData = avatarData,
        )
        Text(
            text = chat.displayName,
            modifier =
                Modifier
                    .weight(1f)
                    .padding(start = 16.dp, end = 16.dp),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurface,
        )
        ShareTargetSelectionIndicator(selected = selected)
    }
}

@Composable
private fun ShareTargetSelectionIndicator(selected: Boolean) {
    Surface(
        modifier = Modifier.size(24.dp),
        shape = CircleShape,
        color = if (selected) IrisTheme.palette.accent else Color.Transparent,
        border =
            if (selected) {
                null
            } else {
                BorderStroke(2.dp, IrisTheme.palette.border)
            },
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        if (selected) {
            Box(contentAlignment = Alignment.Center) {
                Icon(
                    imageVector = Icons.Rounded.Check,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onPrimary,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
    }
}

@Composable
private fun ShareTargetBottomBar(
    selectedChats: List<ChatThreadSnapshot>,
    onSend: () -> Unit,
) {
    val interactionSource = remember { MutableInteractionSource() }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.background,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(IrisTheme.palette.border),
            )
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = 56.dp)
                        .padding(start = 16.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    modifier =
                        Modifier
                            .weight(1f)
                            .height(44.dp)
                            .horizontalScroll(rememberScrollState()),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    selectedChats.forEachIndexed { index, chat ->
                        Text(
                            text = if (index == 0) chat.displayName else ", ${chat.displayName}",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurface,
                            maxLines = 1,
                        )
                    }
                }
                Box(
                    modifier =
                        Modifier
                            .size(56.dp)
                            .clip(CircleShape)
                            .clickable(
                                interactionSource = interactionSource,
                                indication = null,
                                onClick = onSend,
                            ),
                    contentAlignment = Alignment.Center,
                ) {
                    Box(
                        modifier =
                            Modifier
                                .size(40.dp)
                                .clip(CircleShape)
                                .background(IrisTheme.palette.accent),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            imageVector = IrisIcons.Send,
                            contentDescription = "Send",
                            tint = MaterialTheme.colorScheme.onPrimary,
                            modifier = Modifier.size(22.dp),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ShareEmptyChatsContent(onNewChat: () -> Unit) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Column(
        modifier =
            Modifier
                .fillMaxSize(),
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            text = "Start a chat first",
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp, vertical = 12.dp),
            textAlign = TextAlign.Center,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 64.dp)
                    .clickable(
                        interactionSource = interactionSource,
                        indication = null,
                    ) {
                        haptics.confirm()
                        onNewChat()
                    }
                    .padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier =
                    Modifier
                        .size(40.dp)
                        .clip(CircleShape)
                        .background(IrisTheme.palette.panelAlt),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = IrisIcons.NewChat,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(22.dp),
                )
            }
            Text(
                text = "New chat",
                modifier =
                    Modifier
                        .weight(1f)
                        .padding(start = 16.dp),
                style = MaterialTheme.typography.bodyLarge,
            )
            Icon(
                imageVector = IrisIcons.ChevronRight,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.size(24.dp),
            )
        }
    }
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
