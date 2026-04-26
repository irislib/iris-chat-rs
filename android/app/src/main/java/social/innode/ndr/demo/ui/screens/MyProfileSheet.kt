package social.innode.ndr.demo.ui.screens

import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.widget.Toast
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
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.NetworkStatusSnapshot
import social.innode.ndr.demo.rust.PreferencesSnapshot
import social.innode.ndr.demo.rust.proxiedImageUrl
import social.innode.ndr.demo.ui.components.IrisAvatar
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisInlineAction
import social.innode.ndr.demo.ui.components.IrisPrimaryButton
import social.innode.ndr.demo.ui.components.IrisSectionCard
import social.innode.ndr.demo.ui.components.IrisSecondaryButton
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

private const val IrisSourceUrl =
    "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs"
private const val IrisSourceLabel =
    "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs"

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
    onManageDevices: () -> Unit,
    onLogout: () -> Unit,
    onDismiss: () -> Unit,
) {
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    val qrBitmap =
        remember(npub) {
            createQrBitmap(npub, size = 768)
        }
    var supportBusy by remember { mutableStateOf(false) }
    var pendingSecretExport by remember { mutableStateOf<SecretExportKind?>(null) }
    var showDeleteAllConfirmation by remember { mutableStateOf(false) }
    var profileName by remember(displayName) { mutableStateOf(displayName) }
    var profilePictureUrl by remember(pictureUrl) { mutableStateOf(pictureUrl.orEmpty()) }
    var showProfilePicture by remember { mutableStateOf(false) }
    var newRelayUrl by remember { mutableStateOf("") }
    var editingRelayUrl by remember { mutableStateOf<String?>(null) }
    var editingRelayDraft by remember { mutableStateOf("") }
    val trimmedPictureUrl = pictureUrl?.trim().orEmpty()
    val relayUrls = networkStatus?.relayUrls ?: preferences.nostrRelayUrls
    val proxiedAvatarUrl =
        trimmedPictureUrl.ifEmpty { null }?.let { url ->
            proxiedImageUrl(
                originalSrc = url,
                preferences = preferences,
                width = 108u,
                height = 108u,
                square = true,
            )
        }
    val proxiedProfilePictureUrl =
        trimmedPictureUrl.ifEmpty { null }?.let { url ->
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
                        modifier =
                            Modifier
                                .then(
                                    if (trimmedPictureUrl.isNotEmpty()) {
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
                            text = displayName.ifBlank { "Owner profile" },
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
                TextField(
                    value = profilePictureUrl,
                    onValueChange = { profilePictureUrl = it },
                    label = { Text("Profile picture URL") },
                    singleLine = true,
                    enabled = canManageDevices,
                    modifier = Modifier.fillMaxWidth().testTag("myProfilePictureUrlInput"),
                )
                IrisSecondaryButton(
                    text = "Save profile",
                    onClick = {
                        appManager.updateProfileMetadata(
                            name = profileName,
                            pictureUrl = profilePictureUrl,
                        )
                    },
                    enabled = canManageDevices &&
                        profileName.trim().isNotEmpty() &&
                        (
                            profileName.trim() != displayName.trim() ||
                                profilePictureUrl.trim() != pictureUrl.orEmpty().trim()
                        ),
                    modifier = Modifier.testTag("myProfileSaveProfileButton"),
                )
                Text(
                    text = "Scan this owner QR from a fresh device to start linking it. The primary device still controls roster approval.",
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
                            contentDescription = "My user ID QR code",
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
                    text = "Copy device ID",
                    onClick = { clipboard.setText("Device ID", deviceNpub) },
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
                        text = "Notifications",
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
                    label = { Text("Key hex") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().testTag("myProfileImageProxyKeyInput"),
                )
                TextField(
                    value = imageProxySaltHex,
                    onValueChange = { value ->
                        appManager.dispatch(AppAction.SetImageProxySaltHex(value))
                    },
                    label = { Text("Salt hex") },
                    singleLine = true,
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
                        text = "Trusted test build",
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        text = "This beta uses a controlled relay set and is not for sensitive conversations. Expect occasional resets and export a support bundle before reporting issues.",
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
                    text = "Export device key",
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
                    text = "User ID",
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    npub,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.testTag("myProfileNpubValue"),
                )
                Text(
                    text = "Current device ID",
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = deviceNpub,
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                )
            }

            IrisSectionCard {
                Text(
                    text = "Nostr relays",
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
                            IconButton(
                                onClick = {
                                    editingRelayUrl = relayUrl
                                    editingRelayDraft = relayUrl
                                },
                            ) {
                                Icon(IrisIcons.Edit, contentDescription = "Edit relay")
                            }
                            IconButton(
                                onClick = {
                                    appManager.dispatch(AppAction.RemoveNostrRelay(relayUrl))
                                },
                            ) {
                                Icon(IrisIcons.DeleteForever, contentDescription = "Delete relay")
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
                        label = { Text("Relay URL") },
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
                    text = "Reset relays",
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
                Text(
                    text = "Relay set ${appManager.relaySetId()}",
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                )
                networkStatus?.let { status ->
                    Text(
                        text =
                            "Network ${if (status.syncing) "syncing" else "idle"} · " +
                                "${status.relayUrls.size} relays · ${status.recentEventCount} events",
                        style = MaterialTheme.typography.bodySmall,
                        color = IrisTheme.palette.muted,
                        modifier = Modifier.testTag("myProfileNetworkStatusValue"),
                    )
                    Text(
                        text = status.relayUrls.joinToString(", "),
                        style = MaterialTheme.typography.bodySmall,
                        color = IrisTheme.palette.muted,
                        modifier = Modifier.testTag("myProfileRelayUrlsValue"),
                    )
                    status.lastDebugCategory?.let { category ->
                        Text(
                            text = "Last debug $category",
                            style = MaterialTheme.typography.bodySmall,
                            color = IrisTheme.palette.muted,
                        )
                    }
                }
                IrisPrimaryButton(
                    text = if (supportBusy) "Preparing…" else "Share support bundle",
                    onClick = {
                        coroutineScope.launch {
                            supportBusy = true
                            val bundle = appManager.exportSupportBundleJson()
                            supportBusy = false
                            val intent =
                                Intent(Intent.ACTION_SEND).apply {
                                    type = "application/json"
                                    putExtra(Intent.EXTRA_SUBJECT, "Iris Chat support bundle")
                                    putExtra(Intent.EXTRA_TEXT, bundle)
                                }
                            context.startActivity(
                                Intent.createChooser(intent, "Share support bundle"),
                            )
                        }
                    },
                    enabled = !supportBusy,
                    modifier = Modifier.testTag("myProfileShareSupportBundleButton"),
                    icon = {
                        Icon(
                            imageVector = IrisIcons.Copy,
                            contentDescription = null,
                        )
                    },
                )
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
                    text = "Local identity, keys, messages, and cached files are removed from this device.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                    modifier = Modifier.testTag("myProfileDangerZoneText"),
                )
                IrisSecondaryButton(
                    text = "Logout",
                    onClick = onLogout,
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
            imageUrl = proxiedProfilePictureUrl ?: trimmedPictureUrl,
            onDismiss = { showProfilePicture = false },
        )
    }

    if (showDeleteAllConfirmation) {
        AlertDialog(
            onDismissRequest = { showDeleteAllConfirmation = false },
            title = { Text("Delete All Data?") },
            text = {
                Text(
                    "This permanently deletes your identity, keys, messages, and cached files from this device. This action cannot be undone.",
                )
            },
            dismissButton = {
                TextButton(onClick = { showDeleteAllConfirmation = false }) {
                    Text("Cancel")
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDeleteAllConfirmation = false
                        onDismiss()
                        appManager.resetAppState()
                    },
                    modifier = Modifier.testTag("myProfileConfirmDeleteAllDataButton"),
                ) {
                    Text(
                        text = "Delete Everything",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            },
        )
    }

    pendingSecretExport?.let { exportKind ->
        val isDeviceExport = exportKind == SecretExportKind.Device
        AlertDialog(
            onDismissRequest = { pendingSecretExport = null },
            title = {
                Text(if (isDeviceExport) "Export Device Key" else "Export Secret Key")
            },
            text = {
                Text(
                    if (isDeviceExport) {
                        "This device key only unlocks this linked device. Copy it from this device?"
                    } else {
                        "Your secret key gives full access to your identity. Never share it with anyone. Store it securely."
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
                    Text(if (isDeviceExport) "Copy Device Key" else "Copy")
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
    when {
        status == null || !status.relayUrls.contains(relayUrl) ->
            IrisTheme.palette.muted.copy(alpha = 0.55f)
        status.syncing || status.pendingOutboundCount > 0UL || status.pendingGroupControlCount > 0UL ->
            Color(0xFFEAB308)
        else -> Color(0xFF22C55E)
    }

@Composable
private fun ProfilePictureDialog(
    imageUrl: String,
    onDismiss: () -> Unit,
) {
    val bitmap =
        produceState<android.graphics.Bitmap?>(initialValue = null, imageUrl) {
            value =
                withContext(Dispatchers.IO) {
                    runCatching {
                        URL(imageUrl).openStream().use { stream ->
                            BitmapFactory.decodeStream(stream)
                        }
                    }.getOrNull()
                }
        }
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
            bitmap.value?.let { loadedBitmap ->
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
