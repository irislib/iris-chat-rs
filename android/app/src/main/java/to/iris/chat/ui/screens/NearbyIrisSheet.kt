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
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.delay
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppState
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
    onVisibleChange: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(service) {
        while (true) {
            delay(1_000L)
            tick += 1
        }
    }
    val snapshot = tick.let { service.snapshot }

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
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(16.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(
                                    modifier = Modifier.weight(1f),
                                    verticalArrangement = Arrangement.spacedBy(4.dp),
                                ) {
                                    val statusText = if (snapshot.visible) snapshot.status else "Off"
                                    Text(
                                        text = "Visible",
                                        style = MaterialTheme.typography.titleMedium,
                                    )
                                    if (statusText != "Visible") {
                                        Text(
                                            text = statusText,
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = IrisTheme.palette.muted,
                                        )
                                    }
                                }
                                Switch(
                                    checked = snapshot.visible,
                                    onCheckedChange = onVisibleChange,
                                    modifier = Modifier.testTag("nearbyVisibilitySwitch"),
                                )
                            }
                        }
                    }

                    if (snapshot.peers.isEmpty()) {
                        item(key = "empty") {
                            Text(
                                text = "No users nearby",
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                                style = MaterialTheme.typography.bodyLarge,
                                color = IrisTheme.palette.muted,
                            )
                        }
                    } else {
                        items(snapshot.peers, key = { it.id }) { peer ->
                            NearbyPeerRow(
                                appManager = appManager,
                                appState = appState,
                                peer = peer,
                                onOpenChat = {
                                    peer.ownerPubkeyHex?.let {
                                        appManager.createChat(it)
                                        onDismiss()
                                    }
                                },
                            )
                            IrisDivider(modifier = Modifier.padding(start = 70.dp))
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
private fun NearbyPeerRow(
    appManager: AppManager,
    appState: AppState,
    peer: IrisNearbyService.Peer,
    onOpenChat: () -> Unit,
) {
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
        title = peer.name,
        preview = if (peer.ownerPubkeyHex == null) "Found" else "Ready",
        timeLabel = null,
        unreadCount = 0,
        lastMessageMine = false,
        lastDelivery = null,
        onClick = onOpenChat,
        leadingContent = {
            IrisAvatar(
                label = peer.name,
                size = 42.dp,
                imageUrl = avatarUrl,
                imageData = avatarData,
            )
        },
        modifier = Modifier.testTag("nearbyIrisPeer-${peer.id.take(12)}"),
    )
}
