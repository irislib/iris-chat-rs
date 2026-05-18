package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.delay
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisChatListRow
import to.iris.chat.ui.components.IrisDivider
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun NearbyIrisSheet(
    appManager: AppManager,
    appState: AppState,
    service: IrisNearbyService,
    onNearbyEnabledChange: (Boolean) -> Unit,
    onVisibleChange: (Boolean) -> Unit,
    onLocalNetworkVisibleChange: (Boolean) -> Unit,
    onOpenPeerProfile: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val snapshot by rememberNearbySnapshotState(service)
    val nearbyEnabled = appState.preferences.nearbyEnabled
    val knownDirectChatIds = appState.knownDirectChatIds()
    val sortedBluetoothPeers =
        if (nearbyEnabled) {
            val bluetoothPeerIds = snapshot.bluetoothPeers.mapTo(mutableSetOf()) { it.id }
            rememberSortedNearbyPeers(
                peers = snapshot.bluetoothPeers,
                knownDirectChatIds = knownDirectChatIds,
                bluetoothPeerIds = bluetoothPeerIds,
                localNetworkPeerIds = emptySet(),
            )
        } else {
            emptyList()
        }
    val sortedLocalNetworkPeers =
        if (nearbyEnabled) {
            val localNetworkPeerIds = snapshot.localNetworkPeers.mapTo(mutableSetOf()) { it.id }
            rememberSortedNearbyPeers(
                peers = snapshot.localNetworkPeers,
                knownDirectChatIds = knownDirectChatIds,
                bluetoothPeerIds = emptySet(),
                localNetworkPeerIds = localNetworkPeerIds,
            )
        } else {
            emptyList()
        }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier =
                Modifier
                    .fillMaxSize()
                    .testTag("nearbyIrisSheet"),
            color = MaterialTheme.colorScheme.background,
        ) {
            Scaffold(
                containerColor = MaterialTheme.colorScheme.background,
                topBar = {
                    IrisTopBar(
                        title = "Nearby",
                        actions = {
                            IconButton(
                                onClick = onDismiss,
                                modifier = Modifier.testTag("nearbyCloseButton"),
                            ) {
                                Icon(
                                    imageVector = IrisIcons.Close,
                                    contentDescription = "Close",
                                )
                            }
                        },
                    )
                },
            ) { padding ->
                LazyColumn(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(padding)
                            .background(MaterialTheme.colorScheme.background),
                ) {
                    item(key = "visibility") {
                        IrisSectionCard(
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                        ) {
                            NearbyMasterRow(
                                checked = nearbyEnabled,
                                onCheckedChange = onNearbyEnabledChange,
                            )
                            IrisDivider()
                            NearbyTransportRow(
                                appManager = appManager,
                                appState = appState,
                                title = "Bluetooth",
                                status = if (nearbyEnabled) nearbyBluetoothTransportStatus(snapshot) else null,
                                checked = appState.preferences.nearbyBluetoothEnabled,
                                enabled = nearbyEnabled,
                                peers = sortedBluetoothPeers,
                                onCheckedChange = onVisibleChange,
                                onOpenPeer = { peer ->
                                    peer.ownerPubkeyHex?.takeIf { it.isNotBlank() }?.let { owner ->
                                        if (appState.hasKnownDirectChat(owner)) {
                                            appManager.openChat(owner)
                                            onDismiss()
                                        } else {
                                            onOpenPeerProfile(owner)
                                        }
                                    }
                                },
                                onOpenPeerProfile = { peer ->
                                    peer.ownerPubkeyHex?.let(onOpenPeerProfile)
                                },
                                modifier = Modifier.testTag("nearbyVisibilitySwitch"),
                            )
                            IrisDivider()
                            NearbyTransportRow(
                                appManager = appManager,
                                appState = appState,
                                title = "Wi-Fi",
                                status = if (nearbyEnabled) nearbyWifiTransportStatus(snapshot) else null,
                                checked = appState.preferences.nearbyLanEnabled,
                                enabled = nearbyEnabled,
                                peers = sortedLocalNetworkPeers,
                                onCheckedChange = onLocalNetworkVisibleChange,
                                onOpenPeer = { peer ->
                                    peer.ownerPubkeyHex?.takeIf { it.isNotBlank() }?.let { owner ->
                                        if (appState.hasKnownDirectChat(owner)) {
                                            appManager.openChat(owner)
                                            onDismiss()
                                        } else {
                                            onOpenPeerProfile(owner)
                                        }
                                    }
                                },
                                onOpenPeerProfile = { peer ->
                                    peer.ownerPubkeyHex?.let(onOpenPeerProfile)
                                },
                                modifier = Modifier.testTag("nearbyLanSwitch"),
                            )
                            IrisDivider()
                            NearbyMailbagRow(
                                appManager = appManager,
                                enabled = appState.preferences.nearbyMailbagEnabled,
                                rowEnabled = nearbyEnabled,
                                summary = snapshot.mailbagSummary,
                            )
                        }
                    }
                    item(key = "bottom") {
                        Spacer(modifier = Modifier.padding(bottom = 12.dp))
                    }
                }
            }
        }
    }
}

@Composable
private fun NearbyMasterRow(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth().padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "Nearby",
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.titleMedium,
        )
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = Modifier.testTag("nearbyEnabledSwitch"),
        )
    }
}

@Composable
internal fun rememberNearbySnapshotState(service: IrisNearbyService) = produceState(
    initialValue = service.snapshot,
    key1 = service,
) {
    while (true) {
        delay(2_000L)
        val next = service.snapshot
        if (next != value) {
            value = next
        }
    }
}

// Mirrors `NearbyTransportRow` (title + optional summary + Switch on
// the first line; expanded copy below when on) so the Mailbag reads
// as a peer to Bluetooth / Wi-Fi — another transport-layer thing the
// user can pause without losing data.
@Composable
private fun NearbyMailbagRow(
    appManager: AppManager,
    enabled: Boolean,
    rowEnabled: Boolean,
    summary: String?,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .alpha(if (rowEnabled) 1f else DisabledNearbyRowAlpha)
                .padding(vertical = 8.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(text = "Mailbag", style = MaterialTheme.typography.titleMedium)
                if (summary != null) {
                    Text(
                        text = summary,
                        style = MaterialTheme.typography.bodyMedium,
                        color = IrisTheme.palette.muted,
                    )
                }
            }
            Switch(
                checked = enabled,
                onCheckedChange = { next ->
                    appManager.dispatch(AppAction.SetNearbyMailbagEnabled(next))
                },
                enabled = rowEnabled,
                modifier = Modifier.testTag("nearbyMailbagSwitch"),
            )
        }
        if (rowEnabled && enabled) {
            Text(
                // Mailbag carries other people's messages too — call
                // that out so users understand what gets stored on
                // their device when this is on.
                text = "Anonymously carries messages by you and others over Bluetooth or Wi-Fi, so they keep moving where there's no internet.",
                modifier = Modifier.padding(top = 8.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
        }
    }
}

@Composable
private fun NearbyTransportRow(
    appManager: AppManager,
    appState: AppState,
    title: String,
    status: String?,
    checked: Boolean,
    enabled: Boolean,
    peers: List<IrisNearbyService.Peer>,
    onCheckedChange: (Boolean) -> Unit,
    onOpenPeer: (IrisNearbyService.Peer) -> Unit,
    onOpenPeerProfile: (IrisNearbyService.Peer) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .alpha(if (enabled) 1f else DisabledNearbyRowAlpha)
                .padding(vertical = 8.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleMedium,
                )
                if (status != null) {
                    Text(
                        text = status,
                        style = MaterialTheme.typography.bodyMedium,
                        color = IrisTheme.palette.muted,
                    )
                }
            }
            Switch(
                checked = checked,
                onCheckedChange = onCheckedChange,
                enabled = enabled,
                modifier = modifier,
            )
        }
        if (enabled && checked) {
            if (peers.isEmpty() && status == null) {
                Text(
                    text = "No users nearby",
                    modifier = Modifier.padding(top = 8.dp),
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                )
            } else {
                peers.forEachIndexed { index, peer ->
                    if (index > 0) {
                        IrisDivider(modifier = Modifier.padding(start = 54.dp))
                    }
                    NearbyPeerRow(
                        appManager = appManager,
                        appState = appState,
                        peer = peer,
                        onOpenChat = { onOpenPeer(peer) },
                        onOpenProfile = { onOpenPeerProfile(peer) },
                    )
                }
            }
        }
    }
}

private fun nearbyBluetoothTransportStatus(snapshot: IrisNearbyService.Snapshot): String? =
    if (snapshot.visible && snapshot.status in nearbyBluetoothBlockingStatuses) snapshot.status else null

private fun nearbyWifiTransportStatus(snapshot: IrisNearbyService.Snapshot): String? =
    if (snapshot.localNetworkVisible && snapshot.localNetworkStatus in nearbyWifiBlockingStatuses) {
        nearbyWifiStatusLabel(snapshot.localNetworkStatus)
    } else {
        null
    }

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

private val nearbyWifiBlockingStatuses =
    setOf(
        "No local network access",
        "Local network unavailable",
        "Local network failed",
    )

private fun nearbyWifiStatusLabel(status: String): String =
    when (status) {
        "No local network access" -> "No Wi-Fi access"
        "Local network unavailable" -> "Wi-Fi unavailable"
        "Local network failed" -> "Wi-Fi failed"
        else -> status
    }

private fun AppState.hasKnownDirectChat(ownerPubkeyHex: String): Boolean =
    chatList.any { chat ->
        chat.kind == ChatKind.DIRECT &&
            chat.chatId.equals(ownerPubkeyHex, ignoreCase = true)
    }

private fun AppState.knownDirectChatIds(): Set<String> =
    chatList
        .asSequence()
        .filter { it.kind == ChatKind.DIRECT }
        .map { it.chatId.trim().lowercase() }
        .filter { it.isNotEmpty() }
        .toSet()

private fun AppState.nearbyPeerResolvedName(peer: IrisNearbyService.Peer): String {
    val owner = peer.ownerPubkeyHex?.trim()
    if (!owner.isNullOrEmpty()) {
        chatList.firstOrNull { chat ->
            chat.kind == ChatKind.DIRECT &&
                chat.chatId.equals(owner, ignoreCase = true)
        }?.displayName?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
    }
    return peer.name.trim().ifEmpty { "Nearby user" }
}

@Composable
private fun NearbyPeerRow(
    appManager: AppManager,
    appState: AppState,
    peer: IrisNearbyService.Peer,
    onOpenChat: () -> Unit,
    onOpenProfile: () -> Unit,
) {
    val displayName = appState.nearbyPeerResolvedName(peer)
    val avatarData by rememberNhashImageData(appManager, peer.pictureUrl)
    val avatarUrl =
        peer.pictureUrl
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
            ?.let { url ->
                proxiedImageUrl(
                    originalSrc = url,
                    preferences = appState.preferences,
                    width = 84u,
                    height = 84u,
                    square = true,
                )
            }
    IrisChatListRow(
        title = displayName,
        preview = null,
        timeLabel = null,
        unreadCount = 0,
        lastMessageMine = false,
        lastDelivery = null,
        onClick = onOpenChat,
        onLongClick = onOpenProfile,
        leadingContent = {
            IrisAvatar(
                label = displayName,
                size = 42.dp,
                imageUrl = avatarUrl,
                imageData = avatarData,
            )
        },
        modifier = Modifier.testTag("nearbyIrisPeer-${peer.id.take(12)}"),
    )
}

private const val DisabledNearbyRowAlpha = 0.48f
