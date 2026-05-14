package to.iris.chat.ui.screens

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
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
import to.iris.chat.rust.AppState
import to.iris.chat.rust.NetworkStatusSnapshot
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisInlineAction
import to.iris.chat.ui.components.IrisListSection
import to.iris.chat.ui.components.IrisMenuRow
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisToggleRow
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisHapticFeedback
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
    Devices("Devices", "settingsDevicesRow"),
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
                Devices,
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
    appState: AppState,
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
    onLogout: () -> Unit,
    onDismiss: () -> Unit,
) {
    val clipboard = rememberIrisClipboard()
    val haptics = rememberIrisHapticFeedback()
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
    var showProfileQr by remember { mutableStateOf(false) }
    var showProfileQrScanner by remember { mutableStateOf(false) }
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
    val profilePictureInteractionSource = remember { MutableInteractionSource() }
    val qrBitmap =
        remember(npub) {
            createQrBitmap(npub, size = 768)
        }
    val canShareProfileCode = remember(context) { canShareText(context) }

    fun handleProfileQrScan(raw: String): String? {
        val normalized = normalizePeerInput(raw)
        if (normalized.isNotBlank() && isValidPeerInput(normalized)) {
            showProfileQrScanner = false
            showProfileQr = false
            appManager.createChat(normalized)
            return null
        }

        val trimmed = raw.trim()
        if (trimmed.isNotBlank()) {
            showProfileQrScanner = false
            showProfileQr = false
            appManager.dispatch(AppAction.AcceptInvite(trimmed))
            return null
        }

        return "That code was empty."
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
                        onQrClick = { showProfileQr = true },
                    )
                    SettingsMenuSection {
                        SettingsPage.menuPages.take(7).forEach { page ->
                            SettingsMenuRow(page = page, onClick = { selectedPage = page })
                        }
                    }
                    SettingsMenuSection {
                        SettingsPage.menuPages.drop(7).forEach { page ->
                            SettingsMenuRow(page = page, onClick = { selectedPage = page })
                        }
                    }
                } else {
                    when (selectedPage) {
                        SettingsPage.Profile -> {
                            ProfileSettingsPage(
                                displayName = displayName,
                                profileName = profileName,
                                onProfileNameChange = { profileName = it },
                                canManageDevices = canManageDevices,
                                imageUrl = proxiedAvatarUrl,
                                imageData = avatarBytes,
                                canOpenPicture = isHttpPictureUrl || (isHashtreePictureUrl && avatarBytes != null),
                                profilePictureInteractionSource = profilePictureInteractionSource,
                                onOpenPicture = {
                                    haptics.press()
                                    showProfilePicture = true
                                },
                                onChangePicture = { profilePicturePicker.launch(arrayOf("image/*")) },
                                onSaveProfile = {
                                    appManager.updateProfileMetadata(
                                        name = profileName,
                                        pictureUrl = pictureUrl,
                                    )
                                },
                                onShowQr = { showProfileQr = true },
                                onCopyUserId = { clipboard.setText("User ID", npub) },
                                onCopyDeviceCode = { clipboard.setText("Link code", deviceNpub) },
                            )
                        }

                        SettingsPage.Devices -> {
                            DeviceRosterContent(
                                appManager = appManager,
                                appState = appState,
                                embedded = true,
                            )
                        }

                        SettingsPage.Messaging -> {
                            SettingsRowsSection {
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
                            SettingsFormSection {
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
                                    shape = RoundedCornerShape(10.dp),
                                    colors = irisTextFieldColors(),
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
                            SettingsFormSection {
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
                                    shape = RoundedCornerShape(10.dp),
                                    colors = irisTextFieldColors(),
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
                                    shape = RoundedCornerShape(10.dp),
                                    colors = irisTextFieldColors(),
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
                                    shape = RoundedCornerShape(10.dp),
                                    colors = irisTextFieldColors(),
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
                            SettingsRowsSection {
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
                            SettingsFormSection {
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
                                                shape = RoundedCornerShape(10.dp),
                                                colors = irisTextFieldColors(),
                                            )
                                            TextButton(
                                                onClick = {
                                                    haptics.press()
                                                    appManager.dispatch(
                                                        AppAction.UpdateNostrRelay(relayUrl, editingRelayDraft),
                                                    )
                                                    editingRelayUrl = null
                                                    editingRelayDraft = ""
                                                },
                                                colors = settingsTextButtonColors(),
                                            ) {
                                                Text("Save")
                                            }
                                            TextButton(
                                                onClick = {
                                                    haptics.press()
                                                    editingRelayUrl = null
                                                    editingRelayDraft = ""
                                                },
                                                colors = settingsTextButtonColors(),
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
                                        shape = RoundedCornerShape(10.dp),
                                        colors = irisTextFieldColors(),
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
                            SettingsRowsSection {
                                if (canManageDevices) {
                                    IrisMenuRow(
                                        title = "Export secret key",
                                        onClick = { pendingSecretExport = SecretExportKind.Owner },
                                        icon = IrisIcons.Key,
                                        modifier = Modifier.testTag("myProfileExportOwnerKeyButton"),
                                    )
                                }
                                IrisMenuRow(
                                    title = "Export this device's key",
                                    onClick = { pendingSecretExport = SecretExportKind.Device },
                                    icon = IrisIcons.Key,
                                    modifier = Modifier.testTag("myProfileExportDeviceKeyButton"),
                                )
                            }
                        }

                        SettingsPage.About -> {
                            SettingsFormSection {
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
                            SettingsFormSection {
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
                            SettingsFormSection {
                                Text(
                                    text = "Remove this profile from this device.",
                                    style = MaterialTheme.typography.bodyLarge,
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

                        else -> Unit
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

    if (showProfileQr && qrBitmap != null) {
        ProfileQrDialog(
            qrBitmap = qrBitmap,
            displayName = displayName,
            canShare = canShareProfileCode,
            onDismiss = { showProfileQr = false },
            onCopy = { clipboard.setText("User ID", npub) },
            onShare = { shareText(context, npub, "Share user ID") },
            onScan = { showProfileQrScanner = true },
        )
    }

    if (showProfileQrScanner) {
        QrScannerDialog(
            onDismiss = { showProfileQrScanner = false },
            onScanned = ::handleProfileQrScan,
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
                TextButton(
                    onClick = {
                        haptics.press()
                        pendingSecretExport = null
                    },
                    colors = settingsTextButtonColors(),
                ) {
                    Text("Cancel")
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        haptics.confirm()
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
                    colors = settingsTextButtonColors(),
                ) {
                    Text(if (isDeviceExport) "Copy Key" else "Copy")
                }
            },
        )
    }
}

@Composable
private fun settingsTextButtonColors() =
    ButtonDefaults.textButtonColors(
        contentColor = MaterialTheme.colorScheme.onSurface,
    )

@Composable
private fun ProfileSettingsPage(
    displayName: String,
    profileName: String,
    onProfileNameChange: (String) -> Unit,
    canManageDevices: Boolean,
    imageUrl: String?,
    imageData: ByteArray?,
    canOpenPicture: Boolean,
    profilePictureInteractionSource: MutableInteractionSource,
    onOpenPicture: () -> Unit,
    onChangePicture: () -> Unit,
    onSaveProfile: () -> Unit,
    onShowQr: () -> Unit,
    onCopyUserId: () -> Unit,
    onCopyDeviceCode: () -> Unit,
) {
    val canSaveProfile =
        canManageDevices &&
            profileName.trim().isNotEmpty() &&
            profileName.trim() != displayName.trim()

    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        ProfileHero(
            displayName = displayName,
            imageUrl = imageUrl,
            imageData = imageData,
            canManageDevices = canManageDevices,
            canOpenPicture = canOpenPicture,
            profilePictureInteractionSource = profilePictureInteractionSource,
            onOpenPicture = onOpenPicture,
            onChangePicture = onChangePicture,
        )

        IrisListSection {
            ProfileNameRow(
                value = profileName,
                onValueChange = onProfileNameChange,
                enabled = canManageDevices,
            )
            if (canSaveProfile) {
                ProfileActionRow(
                    title = "Save profile",
                    icon = IrisIcons.Check,
                    onClick = onSaveProfile,
                    modifier = Modifier.testTag("myProfileSaveProfileButton"),
                )
            }
        }

        IrisListSection {
            ProfileActionRow(
                title = "Show code",
                icon = IrisIcons.ScanQr,
                onClick = onShowQr,
                modifier = Modifier.testTag("myProfileShowQrButton"),
            )
            ProfileActionRow(
                title = "Copy user ID",
                icon = IrisIcons.Copy,
                onClick = onCopyUserId,
            )
            ProfileActionRow(
                title = "Copy this device code",
                icon = IrisIcons.Copy,
                onClick = onCopyDeviceCode,
            )
        }
    }
}

@Composable
private fun ProfileHero(
    displayName: String,
    imageUrl: String?,
    imageData: ByteArray?,
    canManageDevices: Boolean,
    canOpenPicture: Boolean,
    profilePictureInteractionSource: MutableInteractionSource,
    onOpenPicture: () -> Unit,
    onChangePicture: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(top = 8.dp, bottom = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        IrisAvatar(
            label = displayName.ifBlank { "Profile" },
            size = 80.dp,
            emphasize = true,
            imageUrl = imageUrl,
            imageData = imageData,
            modifier =
                Modifier
                    .then(
                        if (canOpenPicture) {
                            Modifier
                                .clickable(
                                    interactionSource = profilePictureInteractionSource,
                                    indication = ripple(bounded = false, radius = 42.dp),
                                    onClick = onOpenPicture,
                                )
                                .testTag("myProfilePictureButton")
                        } else {
                            Modifier
                        },
                    ),
        )
        if (canManageDevices) {
            ProfilePhotoButton(onClick = onChangePicture)
        }
    }
}

@Composable
private fun ProfilePhotoButton(onClick: () -> Unit) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Surface(
        modifier =
            Modifier
                .heightIn(min = 36.dp)
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                ) {
                    haptics.press()
                    onClick()
                }
                .testTag("myProfilePictureUploadButton"),
        color = IrisTheme.palette.panelAlt,
        contentColor = MaterialTheme.colorScheme.onSurface,
        shape = RoundedCornerShape(100.dp),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Box(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = "Edit photo",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

@Composable
private fun ProfileNameRow(
    value: String,
    onValueChange: (String) -> Unit,
    enabled: Boolean,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .padding(horizontal = 16.dp, vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(24.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = IrisIcons.Person,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.size(24.dp),
        )
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            enabled = enabled,
            singleLine = true,
            cursorBrush = SolidColor(MaterialTheme.colorScheme.onSurface),
            textStyle =
                MaterialTheme.typography.bodyLarge.copy(
                    color =
                        if (enabled) {
                            MaterialTheme.colorScheme.onSurface
                        } else {
                            IrisTheme.palette.muted
                        },
                ),
            modifier =
                Modifier
                    .weight(1f)
                    .testTag("myProfileDisplayNameInput"),
            decorationBox = { innerTextField ->
                Box(
                    modifier = Modifier.fillMaxWidth(),
                    contentAlignment = Alignment.CenterStart,
                ) {
                    if (value.isBlank()) {
                        Text(
                            text = "Display name",
                            style = MaterialTheme.typography.bodyLarge,
                            color = IrisTheme.palette.muted,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    innerTextField()
                }
            },
        )
    }
}

@Composable
private fun ProfileActionRow(
    title: String,
    icon: ImageVector,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember(title) { MutableInteractionSource() }
    val rowColor = MaterialTheme.colorScheme.onSurface
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                ) {
                    haptics.press()
                    onClick()
                }
                .padding(horizontal = 16.dp, vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(24.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = rowColor,
            modifier = Modifier.size(24.dp),
        )
        Text(
            text = title,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyLarge,
            color = rowColor,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun ProfileQrDialog(
    qrBitmap: Bitmap,
    displayName: String,
    canShare: Boolean,
    onDismiss: () -> Unit,
    onCopy: () -> Unit,
    onShare: () -> Unit,
    onScan: () -> Unit,
) {
    Dialog(onDismissRequest = onDismiss) {
        Surface(
            color = IrisTheme.palette.panel,
            shape = RoundedCornerShape(20.dp),
            border = BorderStroke(1.dp, IrisTheme.palette.border),
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    ProfileQrTabButton(
                        text = "Code",
                        selected = true,
                        onClick = {},
                    )
                    ProfileQrTabButton(
                        text = "Scan",
                        selected = false,
                        onClick = onScan,
                    )
                }

                ProfileQrBadge(
                    qrBitmap = qrBitmap,
                    label = displayName.ifBlank { "User ID" },
                    onCopy = onCopy,
                )

                Row(
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    ProfileQrIconAction(
                        icon = IrisIcons.Copy,
                        contentDescription = "Copy user ID",
                        onClick = onCopy,
                        modifier = Modifier.testTag("settingsProfileQrCopyButton"),
                    )
                    if (canShare) {
                        ProfileQrIconAction(
                            icon = IrisIcons.Share,
                            contentDescription = "Share user ID",
                            onClick = onShare,
                            modifier = Modifier.testTag("settingsProfileQrShareButton"),
                        )
                    }
                    IrisSecondaryButton(
                        text = "Scan",
                        onClick = onScan,
                        modifier = Modifier.testTag("settingsProfileQrScanButton"),
                        icon = {
                            Icon(imageVector = IrisIcons.ScanQr, contentDescription = null)
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun ProfileQrTabButton(
    text: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Surface(
        color = if (selected) IrisTheme.palette.panelRaised else IrisTheme.palette.panelAlt,
        contentColor = MaterialTheme.colorScheme.onSurface,
        shape = RoundedCornerShape(12.dp),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Box(
            modifier =
                Modifier
                    .width(104.dp)
                    .heightIn(min = 40.dp)
                    .clickable(
                        interactionSource = interactionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onClick()
                    },
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = text,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

@Composable
private fun ProfileQrBadge(
    qrBitmap: Bitmap,
    label: String,
    onCopy: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Surface(
        color = IrisTheme.palette.panelAlt,
        shape = RoundedCornerShape(24.dp),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Column(
            modifier =
                Modifier
                    .width(296.dp)
                    .padding(top = 32.dp, start = 40.dp, end = 40.dp, bottom = 20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            IrisQrCodeImage(
                bitmap = qrBitmap,
                contentDescription = "User ID code",
                size = 216.dp,
                tag = "settingsProfileQrCode",
            )

            Row(
                modifier =
                    Modifier
                        .padding(top = 8.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .clickable(
                            interactionSource = interactionSource,
                            indication = null,
                        ) {
                            haptics.press()
                            onCopy()
                        }
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    imageVector = IrisIcons.Copy,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(18.dp),
                )
                Text(
                    text = label,
                    modifier = Modifier.padding(start = 6.dp),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun ProfileQrIconAction(
    icon: ImageVector,
    contentDescription: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Surface(
        modifier = modifier,
        color = IrisTheme.palette.panel,
        contentColor = MaterialTheme.colorScheme.onSurface,
        shape = CircleShape,
        border = BorderStroke(1.dp, IrisTheme.palette.border),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Box(
            modifier =
                Modifier
                    .size(54.dp)
                    .clickable(
                        interactionSource = interactionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onClick()
                    },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = contentDescription,
                modifier = Modifier.size(22.dp),
            )
        }
    }
}

@Composable
private fun SettingsProfileMenuRow(
    displayName: String,
    imageUrl: String?,
    imageData: ByteArray?,
    onClick: () -> Unit,
    onQrClick: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    val qrInteractionSource = remember { MutableInteractionSource() }
    IrisListSection {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 128.dp)
                    .clickable(
                        interactionSource = interactionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onClick()
                    }
                    .padding(horizontal = 16.dp, vertical = 24.dp)
                    .testTag(SettingsPage.Profile.tag),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IrisAvatar(
                label = displayName.ifBlank { "Profile" },
                size = 80.dp,
                emphasize = true,
                imageUrl = imageUrl,
                imageData = imageData,
            )
            Column(
                modifier =
                    Modifier
                        .weight(1f)
                        .padding(start = 24.dp, end = 12.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = displayName.ifBlank { "Profile" },
                    style = MaterialTheme.typography.titleLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = "My profile",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Box(
                modifier =
                    Modifier
                        .size(36.dp)
                        .background(IrisTheme.palette.panelAlt, CircleShape)
                        .clickable(
                            interactionSource = qrInteractionSource,
                            indication = null,
                        ) {
                            haptics.press()
                            onQrClick()
                        }
                        .testTag("settingsProfileQrButton"),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = IrisIcons.ScanQr,
                    contentDescription = "Show code",
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
    }
}

@Composable
private fun SettingsMenuSection(content: @Composable () -> Unit) {
    IrisListSection {
        content()
    }
}

@Composable
private fun SettingsRowsSection(
    content: @Composable () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        IrisListSection {
            content()
        }
    }
}

@Composable
private fun SettingsFormSection(
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        IrisListSection {
            Column(
                modifier = Modifier.padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
                content = content,
            )
        }
    }
}

@Composable
private fun SettingsMenuRow(
    page: SettingsPage,
    onClick: () -> Unit,
) {
    IrisMenuRow(
        title = page.title,
        onClick = onClick,
        icon = settingsPageIcon(page),
        modifier = Modifier.testTag(page.tag),
    )
}

@Composable
private fun SettingsToggleRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    tag: String,
) {
    IrisToggleRow(
        title = title,
        checked = checked,
        onCheckedChange = onCheckedChange,
        modifier = Modifier.testTag(tag),
    )
}

private fun settingsPageIcon(page: SettingsPage): ImageVector =
    when (page) {
        SettingsPage.Profile -> IrisIcons.Devices
        SettingsPage.Devices -> IrisIcons.Devices
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
    val haptics = rememberIrisHapticFeedback()
    val dismissInteractionSource = remember { MutableInteractionSource() }
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
                    .clickable(
                        interactionSource = dismissInteractionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onDismiss()
                    }
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
