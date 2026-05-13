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
import androidx.compose.foundation.text.selection.SelectionContainer
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import java.net.URL
import kotlinx.coroutines.Dispatchers
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

private enum class SettingsPage(
    val title: String,
    val tag: String,
) {
    Profile("Profile", "settingsProfileRow"),
    Messaging("Messaging", "settingsMessagingRow"),
    Notifications("Notifications", "settingsNotificationsRow"),
    Media("Media", "settingsMediaRow"),
    Nearby("Nearby", "settingsNearbyRow"),
    MessageServers("Message servers", "settingsMessageServersRow"),
    Security("Security", "settingsSecurityRow"),
    About("About", "settingsAboutRow"),
    Support("Support", "settingsSupportRow"),
    AccountData("Account data", "settingsAccountDataRow"),
    ;

    companion object {
        val menuPages =
            listOf(
                Messaging,
                Notifications,
                Media,
                Nearby,
                MessageServers,
                Security,
                About,
                Support,
                AccountData,
            )
    }
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
    var selectedPage by remember { mutableStateOf<SettingsPage?>(null) }
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
                title = selectedPage?.title ?: "Settings",
                onBack = {
                    if (selectedPage == null) {
                        onDismiss()
                    } else {
                        selectedPage = null
                    }
                },
            )
        },
    ) { padding ->
        // Wrap the profile body in a `SelectionContainer` so the
        // version, npub, device pubkey, build summary, etc. can be
        // long-pressed and copied. Buttons / IconButtons inside still
        // route taps the normal way — only inert `Text` picks up
        // selection.
        SelectionContainer {
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
                if (selectedPage == null) {
                    SettingsProfileMenuRow(
                        displayName = displayName,
                        imageUrl = proxiedAvatarUrl,
                        imageData = avatarBytes,
                        onClick = { selectedPage = SettingsPage.Profile },
                    )
                    SettingsMenuSection {
                        SettingsPage.menuPages.take(6).forEach { page ->
                            SettingsMenuRow(page = page, onClick = { selectedPage = page })
                        }
                    }
                    SettingsMenuSection {
                        SettingsPage.menuPages.drop(6).forEach { page ->
                            SettingsMenuRow(page = page, onClick = { selectedPage = page })
                        }
                    }
                } else {
                    when (selectedPage) {
                        SettingsPage.Profile -> {
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
                        }

                        SettingsPage.Messaging -> {
                            IrisSectionCard {
                                Text(
                                    text = "Messaging",
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                SettingsToggleRow(
                                    title = "Typing indicators",
                                    checked = sendTypingIndicators,
                                    onCheckedChange = { enabled ->
                                        appManager.dispatch(AppAction.SetTypingIndicatorsEnabled(enabled))
                                    },
                                    tag = "myProfileTypingIndicatorsSwitch",
                                )
                                SettingsToggleRow(
                                    title = "Received / seen",
                                    checked = sendReadReceipts,
                                    onCheckedChange = { enabled ->
                                        appManager.dispatch(AppAction.SetReadReceiptsEnabled(enabled))
                                    },
                                    tag = "myProfileReadReceiptsSwitch",
                                )
                            }
                        }

                        SettingsPage.Notifications -> {
                            IrisSectionCard {
                                Text(
                                    text = "Notifications",
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                SettingsToggleRow(
                                    title = "Enabled",
                                    checked = desktopNotificationsEnabled,
                                    onCheckedChange = { enabled ->
                                        appManager.dispatch(AppAction.SetDesktopNotificationsEnabled(enabled))
                                    },
                                    tag = "myProfileDesktopNotificationsSwitch",
                                )
                                SettingsToggleRow(
                                    title = "Invite accepted",
                                    checked = preferences.inviteAcceptanceNotificationsEnabled,
                                    onCheckedChange = { enabled ->
                                        appManager.dispatch(
                                            AppAction.SetInviteAcceptanceNotificationsEnabled(enabled),
                                        )
                                    },
                                    tag = "myProfileInviteAcceptedNotificationsSwitch",
                                )
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
                        }

                        SettingsPage.Media -> {
                            IrisSectionCard {
                                Text(
                                    text = "Media",
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                SettingsToggleRow(
                                    title = "Image proxy",
                                    checked = imageProxyEnabled,
                                    onCheckedChange = { enabled ->
                                        appManager.dispatch(AppAction.SetImageProxyEnabled(enabled))
                                    },
                                    tag = "myProfileImageProxySwitch",
                                )
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
                        }

                        SettingsPage.Nearby -> {
                            IrisSectionCard {
                                Text(
                                    text = "Nearby",
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                SettingsToggleRow(
                                    title = "Bluetooth",
                                    checked = preferences.nearbyBluetoothEnabled,
                                    onCheckedChange = onNearbyBluetoothChange,
                                    tag = "myProfileNearbyBluetoothSwitch",
                                )
                                SettingsToggleRow(
                                    title = "Wi-Fi",
                                    checked = preferences.nearbyLanEnabled,
                                    onCheckedChange = onNearbyLanChange,
                                    tag = "myProfileNearbyLanSwitch",
                                )
                            }
                        }

                        SettingsPage.MessageServers -> {
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
                        }

                        SettingsPage.Security -> {
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
                        }

                        SettingsPage.About -> {
                            IrisSectionCard {
                                Text(
                                    text = "About",
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                if (appManager.isTrustedTestBuild()) {
                                    Text(
                                        text = "Test build",
                                        style = MaterialTheme.typography.titleSmall,
                                    )
                                }
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
                        }

                        SettingsPage.Support -> {
                            IrisSectionCard {
                                Text(
                                    text = "Support",
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                SettingsToggleRow(
                                    title = "Debug logging",
                                    checked = preferences.debugLoggingEnabled,
                                    onCheckedChange = { enabled ->
                                        appManager.dispatch(AppAction.SetDebugLoggingEnabled(enabled))
                                    },
                                    tag = "myProfileDebugLoggingSwitch",
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
                                if (canShareSupport) {
                                    IrisPrimaryButton(
                                        text = if (supportBusy) "Preparing…" else "Share debug dump",
                                        onClick = {
                                            coroutineScope.launch {
                                                supportBusy = true
                                                val bundle = appManager.exportSupportBundleJson()
                                                supportBusy = false
                                                shareText(
                                                    context = context,
                                                    text = bundle,
                                                    title = "Share debug dump",
                                                    mimeType = "application/json",
                                                    subject = "Iris Chat debug dump",
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
                                    text = "Copy debug dump",
                                    onClick = {
                                        coroutineScope.launch {
                                            supportBusy = true
                                            val bundle = appManager.exportSupportBundleJson()
                                            supportBusy = false
                                            clipboard.setText("Debug dump", bundle)
                                            Toast.makeText(context, "Debug dump copied", Toast.LENGTH_SHORT).show()
                                        }
                                    },
                                    enabled = !supportBusy,
                                    modifier = Modifier.testTag("myProfileCopySupportBundleButton"),
                                )
                            }
                        }

                        SettingsPage.AccountData -> {
                            IrisSectionCard {
                                Text(
                                    text = "Account data",
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

                        null -> Unit
                    }
                }
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
private fun SettingsProfileMenuRow(
    displayName: String,
    imageUrl: String?,
    imageData: ByteArray?,
    onClick: () -> Unit,
) {
    IrisSectionCard {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onClick)
                    .testTag(SettingsPage.Profile.tag),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IrisAvatar(
                label = displayName.ifBlank { "Profile" },
                size = 54.dp,
                emphasize = true,
                imageUrl = imageUrl,
                imageData = imageData,
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = displayName.ifBlank { "Profile" },
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = "My profile",
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Icon(
                imageVector = IrisIcons.ChevronRight,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
            )
        }
    }
}

@Composable
private fun SettingsMenuSection(content: @Composable () -> Unit) {
    IrisSectionCard {
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            content()
        }
    }
}

@Composable
private fun SettingsMenuRow(
    page: SettingsPage,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .testTag(page.tag)
                .padding(vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(34.dp)
                    .background(IrisTheme.palette.toolbar, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = settingsPageIcon(page),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.size(19.dp),
            )
        }
        Text(
            text = page.title,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.weight(1f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Icon(
            imageVector = IrisIcons.ChevronRight,
            contentDescription = null,
            tint = IrisTheme.palette.muted,
        )
    }
}

@Composable
private fun SettingsToggleRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    tag: String,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.bodyLarge,
        )
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = Modifier.testTag(tag),
        )
    }
}

private fun settingsPageIcon(page: SettingsPage): ImageVector =
    when (page) {
        SettingsPage.Profile -> IrisIcons.Devices
        SettingsPage.Messaging -> IrisIcons.NewChat
        SettingsPage.Notifications -> IrisIcons.Notifications
        SettingsPage.Media -> IrisIcons.Image
        SettingsPage.Nearby -> IrisIcons.Nearby
        SettingsPage.MessageServers -> IrisIcons.Refresh
        SettingsPage.Security -> IrisIcons.Key
        SettingsPage.About -> IrisIcons.File
        SettingsPage.Support -> IrisIcons.Share
        SettingsPage.AccountData -> IrisIcons.DeleteForever
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
