package to.iris.chat.ui.screens

import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.NetworkStatusSnapshot
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisInlineAction
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

private const val IrisSourceUrl =
    "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs"
private const val IrisSourceLabel =
    "Iris Chat source code"
private const val NotificationsServerDefault = "https://notifications.iris.to"
private const val NotificationsServerProjectUrl =
    "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-notification-server"
private const val NotificationsServerProjectLabel = "Notification server source code"

private enum class SecretExportKind {
    Owner,
    Device,
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MyProfileSheet(
    appManager: AppManager,
    npub: String,
    displayName: String,
    pictureUrl: String?,
    deviceNpub: String,
    canManageDevices: Boolean,
    sendTypingIndicators: Boolean,
    sendReadReceipts: Boolean,
    desktopNotificationsEnabled: Boolean,
    imageProxyEnabled: Boolean,
    imageProxyUrl: String,
    imageProxyKeyHex: String,
    imageProxySaltHex: String,
    preferences: PreferencesSnapshot,
    networkStatus: NetworkStatusSnapshot?,
    bluetoothOnProvider: () -> Boolean,
    onNearbyBluetoothChange: (Boolean) -> Unit,
    onNearbyLanChange: (Boolean) -> Unit,
    onManageDevices: () -> Unit,
    onLogout: () -> Unit,
    onDismiss: () -> Unit,
) {
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
    val canShareSupport = remember(context) { canShareText(context, "application/json") }
    val coroutineScope = rememberCoroutineScope()
    val bluetoothOn by produceState(initialValue = bluetoothOnProvider(), key1 = bluetoothOnProvider) {
        while (true) {
            value = bluetoothOnProvider()
            delay(1_000L)
        }
    }
    val profilePicturePicker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri == null) {
                return@rememberLauncherForActivityResult
            }
            coroutineScope.launch {
                val picked =
                    withContext(Dispatchers.IO) {
                        copyAttachmentToCache(context, uri)
                    }
                if (picked != null) {
                    appManager.uploadProfilePicture(picked.path)
                }
            }
        }
    val qrBitmap =
        remember(npub) {
            createQrBitmap(npub, size = 768)
        }
    var supportBusy by remember { mutableStateOf(false) }
    var pendingSecretExport by remember { mutableStateOf<SecretExportKind?>(null) }
    var showLogoutConfirmation by remember { mutableStateOf(false) }
    var showDeleteAllConfirmation by remember { mutableStateOf(false) }
    var profileName by remember(displayName) { mutableStateOf(displayName) }
    var showProfilePicture by remember { mutableStateOf(false) }
    var newRelayUrl by remember { mutableStateOf("") }
    var editingRelayUrl by remember { mutableStateOf<String?>(null) }
    var editingRelayDraft by remember { mutableStateOf("") }
    val trimmedPictureUrl = pictureUrl?.trim().orEmpty()
    val isHttpPictureUrl =
        trimmedPictureUrl.startsWith("http://") || trimmedPictureUrl.startsWith("https://")
    val isHashtreePictureUrl =
        trimmedPictureUrl.startsWith("htree://") || trimmedPictureUrl.startsWith("nhash://")
    val relayUrls = networkStatus?.relayUrls ?: preferences.nostrRelayUrls
    val avatarBytes by rememberNhashImageData(appManager, pictureUrl)
    val proxiedAvatarUrl =
        trimmedPictureUrl.takeIf { isHttpPictureUrl }?.let { url ->
            proxiedImageUrl(
                originalSrc = url,
                preferences = preferences,
                width = 108u,
                height = 108u,
                square = true,
            )
        }
    val proxiedProfilePictureUrl =
        trimmedPictureUrl.takeIf { isHttpPictureUrl }?.let { url ->
            proxiedImageUrl(
                originalSrc = url,
                preferences = preferences,
                width = 1024u,
                height = 1024u,
                square = false,
            )
        }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Settings",
                onBack = onDismiss,
            )
        },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .testTag("myProfileSheet")
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IrisSectionCard {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IrisAvatar(
                        label = displayName.ifBlank { "Profile" },
                        size = 54.dp,
                        emphasize = true,
                        imageUrl = proxiedAvatarUrl,
                        imageData = avatarBytes,
                        modifier =
                            Modifier
                                .then(
                                    if (isHttpPictureUrl || (isHashtreePictureUrl && avatarBytes != null)) {
                                        Modifier
                                            .clickable { showProfilePicture = true }
                                            .testTag("myProfilePictureButton")
                                    } else {
                                        Modifier
                                    },
                                ),
                    )
                    Column {
                        Text(
                            text = displayName.ifBlank { "Profile" },
                            style = MaterialTheme.typography.headlineSmall,
                        )
                        Text(
                            text = "My profile",
                            style = MaterialTheme.typography.titleMedium,
                            color = IrisTheme.palette.muted,
                        )
                    }
                }
                TextField(
                    value = profileName,
                    onValueChange = { profileName = it },
                    label = { Text("Display name") },
                    singleLine = true,
                    enabled = canManageDevices,
                    modifier = Modifier.fillMaxWidth().testTag("myProfileDisplayNameInput"),
                )
                if (canManageDevices) {
                    IrisSecondaryButton(
                        text = "Change picture",
                        onClick = { profilePicturePicker.launch(arrayOf("image/*")) },
                        enabled = true,
                        modifier = Modifier.testTag("myProfilePictureUploadButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Image,
                                contentDescription = null,
                            )
                        },
                    )
                }
                IrisSecondaryButton(
                    text = "Save profile",
                    onClick = {
                        appManager.updateProfileMetadata(
                            name = profileName,
                            pictureUrl = pictureUrl,
                        )
                    },
                    enabled = canManageDevices &&
                        profileName.trim().isNotEmpty() &&
                        profileName.trim() != displayName.trim(),
                    modifier = Modifier.testTag("myProfileSaveProfileButton"),
                )
                Text(
                    text = "Scan this code on another device to link it.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                )
                if (canManageDevices) {
                    IrisPrimaryButton(
                        text = "Manage devices",
                        onClick = onManageDevices,
                        modifier = Modifier.testTag("myProfileManageDevicesButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Devices,
                                contentDescription = null,
                            )
                        },
                    )
                }
                Box(
                    modifier = Modifier.fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    if (qrBitmap != null) {
                        Image(
                            bitmap = qrBitmap.asImageBitmap(),
                            contentDescription = "My user ID code",
                            modifier =
                                Modifier
                                    .size(260.dp)
                                    .testTag("myProfileQrCode"),
                        )
                    }
                }
                IrisInlineAction(
                    text = "Copy user ID",
                    onClick = { clipboard.setText("User ID", npub) },
                ) {
                    Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                }
                IrisInlineAction(
                    text = "Copy this device code",
                    onClick = { clipboard.setText("Link code", deviceNpub) },
                ) {
                    Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                }
            }

            IrisSectionCard {
                Text(
                    text = "Messaging",
                    style = MaterialTheme.typography.titleMedium,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Typing indicators",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = sendTypingIndicators,
                        onCheckedChange = { enabled ->
                            appManager.dispatch(AppAction.SetTypingIndicatorsEnabled(enabled))
                        },
                        modifier = Modifier.testTag("myProfileTypingIndicatorsSwitch"),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Received / seen",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = sendReadReceipts,
                        onCheckedChange = { enabled ->
                            appManager.dispatch(AppAction.SetReadReceiptsEnabled(enabled))
                        },
                        modifier = Modifier.testTag("myProfileReadReceiptsSwitch"),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Image proxy",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = imageProxyEnabled,
                        onCheckedChange = { enabled ->
                            appManager.dispatch(AppAction.SetImageProxyEnabled(enabled))
                        },
                        modifier = Modifier.testTag("myProfileImageProxySwitch"),
                    )
                }
                TextField(
                    value = imageProxyUrl,
                    onValueChange = { value ->
                        appManager.dispatch(AppAction.SetImageProxyUrl(value))
                    },
                    label = { Text("Proxy URL") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().testTag("myProfileImageProxyUrlInput"),
                )
                TextField(
                    value = imageProxyKeyHex,
                    onValueChange = { value ->
                        appManager.dispatch(AppAction.SetImageProxyKeyHex(value))
                    },
                    label = { Text("Proxy key") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth().testTag("myProfileImageProxyKeyInput"),
                )
                TextField(
                    value = imageProxySaltHex,
                    onValueChange = { value ->
                        appManager.dispatch(AppAction.SetImageProxySaltHex(value))
                    },
                    label = { Text("Proxy salt") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth().testTag("myProfileImageProxySaltInput"),
                )
                IrisSecondaryButton(
                    text = "Reset image proxy",
                    onClick = {
                        appManager.dispatch(AppAction.ResetImageProxySettings)
                    },
                    modifier = Modifier.testTag("myProfileResetImageProxyButton"),
                )
            }

            IrisSectionCard {
                Text(
                    text = "Nearby",
                    style = MaterialTheme.typography.titleMedium,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Bluetooth",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = preferences.nearbyBluetoothEnabled,
                        onCheckedChange = onNearbyBluetoothChange,
                        modifier = Modifier.testTag("myProfileNearbyBluetoothSwitch"),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Local network",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = preferences.nearbyLanEnabled,
                        onCheckedChange = onNearbyLanChange,
                        modifier = Modifier.testTag("myProfileNearbyLanSwitch"),
                    )
                }
            }

            IrisSectionCard {
                Text(
                    text = "Notifications",
                    style = MaterialTheme.typography.titleMedium,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Enabled",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = desktopNotificationsEnabled,
                        onCheckedChange = { enabled ->
                            appManager.dispatch(AppAction.SetDesktopNotificationsEnabled(enabled))
                        },
                        modifier = Modifier.testTag("myProfileDesktopNotificationsSwitch"),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Invite accepted",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Switch(
                        checked = preferences.inviteAcceptanceNotificationsEnabled,
                        onCheckedChange = { enabled ->
                            appManager.dispatch(
                                AppAction.SetInviteAcceptanceNotificationsEnabled(enabled),
                            )
                        },
                        modifier = Modifier.testTag("myProfileInviteAcceptedNotificationsSwitch"),
                    )
                }
                TextField(
                    value = preferences.mobilePushServerUrl,
                    onValueChange = { value ->
                        appManager.dispatch(AppAction.SetMobilePushServerUrl(value))
                    },
                    label = { Text("Server URL") },
                    placeholder = { Text(NotificationsServerDefault) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().testTag("myProfileNotificationsServerUrlInput"),
                )
                IrisInlineAction(
                    text = NotificationsServerProjectLabel,
                    onClick = {
                        context.startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse(NotificationsServerProjectUrl)),
                        )
                    },
                    modifier = Modifier.testTag("myProfileNotificationsServerProjectLink"),
                ) {
                    Icon(imageVector = IrisIcons.File, contentDescription = null)
                }
            }

            IrisSectionCard {
                Text(
                    text = "About",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = "Version",
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = appManager.buildSummary(),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.testTag("myProfileVersionValue"),
                )
                IrisInlineAction(
                    text = "Source code",
                    onClick = {
                        context.startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse(IrisSourceUrl)),
                        )
                    },
                    modifier = Modifier.testTag("myProfileSourceCodeButton"),
                ) {
                    Icon(imageVector = IrisIcons.File, contentDescription = null)
                }
                Text(
                    text = IrisSourceLabel,
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                    modifier = Modifier.testTag("myProfileSourceCodeValue"),
                )
            }

            if (appManager.isTrustedTestBuild()) {
                IrisSectionCard {
                    Text(
                        text = "Test build",
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        text = "For trusted testing only.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = IrisTheme.palette.muted,
                    )
                }
            }

            IrisSectionCard {
                Text(
                    text = "Security",
                    style = MaterialTheme.typography.titleMedium,
                )
                if (canManageDevices) {
                    IrisSecondaryButton(
                        text = "Export secret key",
                        onClick = { pendingSecretExport = SecretExportKind.Owner },
                        modifier = Modifier.testTag("myProfileExportOwnerKeyButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Key,
                                contentDescription = null,
                            )
                        },
                    )
                }
                IrisSecondaryButton(
                    text = "Export this device's key",
                    onClick = { pendingSecretExport = SecretExportKind.Device },
                    modifier = Modifier.testTag("myProfileExportDeviceKeyButton"),
                    icon = {
                        Icon(
                            imageVector = IrisIcons.Key,
                            contentDescription = null,
                        )
                    },
                )
            }

            IrisSectionCard {
                Text(
                    text = "Message servers",
                    style = MaterialTheme.typography.titleMedium,
                )
                relayUrls.forEach { relayUrl ->
                    if (editingRelayUrl == relayUrl) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            TextField(
                                value = editingRelayDraft,
                                onValueChange = { editingRelayDraft = it },
                                singleLine = true,
                                modifier =
                                    Modifier
                                        .weight(1f)
                                        .testTag("myProfileEditRelayInput"),
                            )
                            TextButton(
                                onClick = {
                                    appManager.dispatch(
                                        AppAction.UpdateNostrRelay(relayUrl, editingRelayDraft),
                                    )
                                    editingRelayUrl = null
                                    editingRelayDraft = ""
                                },
                            ) {
                                Text("Save")
                            }
                            TextButton(
                                onClick = {
                                    editingRelayUrl = null
                                    editingRelayDraft = ""
                                },
                            ) {
                                Text("Cancel")
                            }
                        }
                    } else {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Box(
                                modifier =
                                    Modifier
                                        .size(8.dp)
                                        .background(relayStatusColor(networkStatus, relayUrl), CircleShape),
                            )
                            Text(
                                text = relayUrl,
                                style = MaterialTheme.typography.bodySmall,
                                modifier =
                                    Modifier
                                        .weight(1f)
                                        .testTag("myProfileRelayRow"),
                            )
                            relayStatusLabel(networkStatus, relayUrl)?.let { label ->
                                Text(
                                    text = label,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = IrisTheme.palette.muted,
                                )
                            }
                            IconButton(
                                onClick = {
                                    editingRelayUrl = relayUrl
                                    editingRelayDraft = relayUrl
                                },
                            ) {
                                Icon(IrisIcons.Edit, contentDescription = "Edit server")
                            }
                            IconButton(
                                onClick = {
                                    appManager.dispatch(AppAction.RemoveNostrRelay(relayUrl))
                                },
                            ) {
                                Icon(IrisIcons.DeleteForever, contentDescription = "Delete server")
                            }
                        }
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    TextField(
                        value = newRelayUrl,
                        onValueChange = { newRelayUrl = it },
                        label = { Text("Server URL") },
                        singleLine = true,
                        modifier = Modifier.weight(1f).testTag("myProfileNewRelayInput"),
                    )
                    IrisSecondaryButton(
                        text = "Add",
                        onClick = {
                            appManager.dispatch(AppAction.AddNostrRelay(newRelayUrl))
                            newRelayUrl = ""
                        },
                        modifier = Modifier.testTag("myProfileAddRelayButton"),
                    )
                }
                IrisSecondaryButton(
                    text = "Reset servers",
                    onClick = { appManager.dispatch(AppAction.ResetNostrRelays) },
                    modifier = Modifier.testTag("myProfileResetRelaysButton"),
                )
            }

            IrisSectionCard {
                Text(
                    text = "Support",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = "Build ${appManager.buildSummary()}",
                    style = MaterialTheme.typography.bodyMedium,
                )
                networkStatus?.let { status ->
                    Text(
                        text =
                            "Network ${if (status.syncing) "syncing" else "idle"} · " +
                                "${status.connectedRelayCount}/${status.relayUrls.size} connected · " +
                                "${status.recentEventCount} updates",
                        style = MaterialTheme.typography.bodySmall,
                        color = IrisTheme.palette.muted,
                        modifier = Modifier.testTag("myProfileNetworkStatusValue"),
                    )
                    status.lastDebugCategory?.let { category ->
                        Text(
                            text = "Last debug $category",
                            style = MaterialTheme.typography.bodySmall,
                            color = IrisTheme.palette.muted,
                        )
                    }
                }
                Text(
                    text = "Bluetooth ${if (bluetoothOn) "on" else "off"}",
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                    modifier = Modifier.testTag("myProfileBluetoothStatusValue"),
                )
                Text(
                    text = "Local network ${if (preferences.nearbyLanEnabled) "on" else "off"}",
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                    modifier = Modifier.testTag("myProfileLanStatusValue"),
                )
                if (canShareSupport) {
                    IrisPrimaryButton(
                        text = if (supportBusy) "Preparing…" else "Share support bundle",
                        onClick = {
                            coroutineScope.launch {
                                supportBusy = true
                                val bundle = appManager.exportSupportBundleJson()
                                supportBusy = false
                                shareText(
                                    context = context,
                                    text = bundle,
                                    title = "Share support bundle",
                                    mimeType = "application/json",
                                    subject = "Iris Chat support bundle",
                                )
                            }
                        },
                        enabled = !supportBusy,
                        modifier = Modifier.testTag("myProfileShareSupportBundleButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Share,
                                contentDescription = null,
                            )
                        },
                    )
                }
                IrisSecondaryButton(
                    text = "Copy support bundle",
                    onClick = {
                        coroutineScope.launch {
                            supportBusy = true
                            val bundle = appManager.exportSupportBundleJson()
                            supportBusy = false
                            clipboard.setText("Support bundle", bundle)
                            Toast.makeText(context, "Support bundle copied", Toast.LENGTH_SHORT).show()
                        }
                    },
                    enabled = !supportBusy,
                    modifier = Modifier.testTag("myProfileCopySupportBundleButton"),
                )
            }

            IrisSectionCard {
                Text(
                    text = "Danger Zone",
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.error,
                )
                Text(
                    text = "Your account, secret keys, messages, and cached files are removed from this device.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                    modifier = Modifier.testTag("myProfileDangerZoneText"),
                )
                IrisSecondaryButton(
                    text = "Logout",
                    onClick = { showLogoutConfirmation = true },
                    modifier = Modifier.testTag("myProfileLogoutButton"),
                    icon = {
                        Icon(
                            imageVector = IrisIcons.Logout,
                            contentDescription = null,
                        )
                    },
                )
                IrisSecondaryButton(
                    text = "Delete all data",
                    onClick = { showDeleteAllConfirmation = true },
                    modifier = Modifier.testTag("myProfileDeleteAllDataButton"),
                    icon = {
                        Icon(
                            imageVector = IrisIcons.DeleteForever,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                    },
                )
            }
        }
    }

    if (showProfilePicture && trimmedPictureUrl.isNotEmpty()) {
        ProfilePictureDialog(
            imageUrl = if (isHttpPictureUrl) proxiedProfilePictureUrl ?: trimmedPictureUrl else null,
            imageData = if (isHashtreePictureUrl) avatarBytes else null,
            onDismiss = { showProfilePicture = false },
        )
    }

    if (showLogoutConfirmation) {
        DeleteAppDataConfirmationDialog(
            onDismiss = { showLogoutConfirmation = false },
            onConfirm = {
                showLogoutConfirmation = false
                onDismiss()
                onLogout()
            },
            confirmTag = "myProfileConfirmLogoutButton",
        )
    }

    if (showDeleteAllConfirmation) {
        DeleteAppDataConfirmationDialog(
            onDismiss = { showDeleteAllConfirmation = false },
            onConfirm = {
                showDeleteAllConfirmation = false
                onDismiss()
                appManager.resetAppState()
            },
            confirmTag = "myProfileConfirmDeleteAllDataButton",
        )
    }

    pendingSecretExport?.let { exportKind ->
        val isDeviceExport = exportKind == SecretExportKind.Device
        AlertDialog(
            onDismissRequest = { pendingSecretExport = null },
            title = {
                Text(if (isDeviceExport) "Export This Device's Key" else "Export Secret Key")
            },
            text = {
                Text(
                    if (isDeviceExport) {
                        "This key only unlocks this device. Copy it now?"
                    } else {
                        "Your secret key gives full access to your profile. Never share it with anyone. Store it securely."
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { pendingSecretExport = null }) {
                    Text("Cancel")
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        pendingSecretExport = null
                        coroutineScope.launch {
                            val secret =
                                if (isDeviceExport) {
                                    appManager.exportDeviceNsec()
                                } else {
                                    appManager.exportOwnerNsec()
                                }
                            if (secret.isNullOrBlank()) {
                                Toast.makeText(context, "Key unavailable", Toast.LENGTH_SHORT).show()
                            } else {
                                clipboard.setText("Secret key", secret)
                                Toast.makeText(context, "Copied to clipboard", Toast.LENGTH_SHORT).show()
                            }
                        }
                    },
                    modifier = Modifier.testTag(
                        if (isDeviceExport) {
                            "myProfileConfirmExportDeviceKeyButton"
                        } else {
                            "myProfileConfirmExportOwnerKeyButton"
                        },
                    ),
                ) {
                    Text(if (isDeviceExport) "Copy Key" else "Copy")
                }
            },
        )
    }
}

@Composable
private fun relayStatusColor(
    status: NetworkStatusSnapshot?,
    relayUrl: String,
): Color =
    when (relayConnectionStatus(status, relayUrl)) {
        "connected" -> Color(0xFF22C55E)
        "connecting", "sleeping" -> Color(0xFFEAB308)
        "offline", "blocked" -> Color(0xFFEF4444)
        else -> IrisTheme.palette.muted.copy(alpha = 0.55f)
    }

private fun relayStatusLabel(
    status: NetworkStatusSnapshot?,
    relayUrl: String,
): String? =
    when (relayConnectionStatus(status, relayUrl)) {
        "connected" -> "Online"
        "connecting" -> "Connecting"
        "sleeping" -> "Waiting"
        "offline" -> "Offline"
        "blocked" -> "Blocked"
        else -> null
    }

private fun relayConnectionStatus(
    status: NetworkStatusSnapshot?,
    relayUrl: String,
): String? =
    status
        ?.relayConnections
        ?.firstOrNull { it.url == relayUrl }
        ?.status

@Composable
private fun ProfilePictureDialog(
    imageUrl: String?,
    imageData: ByteArray?,
    onDismiss: () -> Unit,
) {
    val dataBitmap =
        remember(imageData) {
            imageData?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
        }
    val urlBitmap by produceState<android.graphics.Bitmap?>(initialValue = null, imageUrl) {
        val url = imageUrl
        value =
            if (url == null) {
                null
            } else {
                withContext(Dispatchers.IO) {
                    runCatching {
                        URL(url).openStream().use { stream ->
                            BitmapFactory.decodeStream(stream)
                        }
                    }.getOrNull()
                }
            }
    }
    val resolvedBitmap = dataBitmap ?: urlBitmap
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.92f))
                    .clickable(onClick = onDismiss)
                    .testTag("myProfilePictureViewer"),
            contentAlignment = Alignment.Center,
        ) {
            resolvedBitmap?.let { loadedBitmap ->
                Image(
                    bitmap = loadedBitmap.asImageBitmap(),
                    contentDescription = "Profile picture",
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(18.dp),
                    contentScale = ContentScale.Fit,
                )
            } ?: CircularProgressIndicator(color = Color.White)
            IconButton(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.TopEnd),
            ) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Close profile picture",
                    tint = Color.White,
                )
            }
        }
    }
}
