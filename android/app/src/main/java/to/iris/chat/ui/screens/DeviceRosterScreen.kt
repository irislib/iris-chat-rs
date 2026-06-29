package to.iris.chat.ui.screens

import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.qr.DeviceApprovalQr
import to.iris.chat.rust.AppState
import to.iris.chat.rust.DeviceEntrySnapshot
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisListSection
import to.iris.chat.ui.components.IrisMenuRow
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTextButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.formatRelativeTime
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun DeviceRosterScreen(
    appManager: AppManager,
    appState: AppState,
) {
    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Manage devices",
                onBack = { appManager.navigateBack() },
            )
        },
    ) { padding ->
        DeviceRosterContent(
            appManager = appManager,
            appState = appState,
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
        )
    }
}

@Composable
fun DeviceRosterContent(
    appManager: AppManager,
    appState: AppState,
    modifier: Modifier = Modifier,
    embedded: Boolean = false,
) {
    val roster = appState.deviceRoster
    val clipboard = rememberIrisClipboard()
    var deviceInput by remember { mutableStateOf("") }
    var showScanner by remember { mutableStateOf(false) }
    val resolvedInput =
        roster?.let {
            resolveDeviceAuthorizationInput(
                deviceInput,
                it.ownerNpub,
                it.ownerPublicKeyHex,
            )
        }
    val normalizedInput = resolvedInput?.deviceInput.orEmpty()
    val canAddDevice =
        roster?.canManageDevices == true &&
            normalizedInput.isNotBlank() &&
            !appState.busy.updatingRoster
    val isCurrentDeviceRegistered =
        roster?.devices?.any { it.devicePubkeyHex == roster.currentDevicePublicKeyHex } == true

    if (roster == null) {
        Column(
            modifier =
                modifier
                    .fillMaxWidth()
                    .padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Loading devices…")
        }
        return
    }

    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(
                    horizontal = if (embedded) 0.dp else 16.dp,
                    vertical = if (embedded) 0.dp else 14.dp,
                ),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                text = "Linked devices",
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(horizontal = 2.dp),
            )
            IrisListSection {
                IrisMenuRow(
                    title = "Copy user ID",
                    icon = IrisIcons.Copy,
                    onClick = { clipboard.setText("User ID", roster.ownerNpub) },
                    modifier = Modifier.testTag("deviceRosterOwnerNpub"),
                )
            }
        }

        Column(
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = "Link another device",
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(horizontal = 2.dp),
            )
            Text(
                text =
                    if (roster.canManageDevices) {
                        "Scan the code from the device you want to link, or paste it."
                    } else if (isCurrentDeviceRegistered) {
                        "This device can view the list but cannot change it."
                    } else {
                        "Sign in with your secret key before changing devices."
                    },
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )

            if (roster.canManageDevices) {
                TextField(
                    value = deviceInput,
                    onValueChange = { deviceInput = it },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("deviceRosterAddInput"),
                    placeholder = {
                        Text(
                            text = "Link code",
                            color = IrisTheme.palette.muted,
                        )
                    },
                    isError = deviceInput.isNotBlank() && resolvedInput?.errorMessage != null,
                    minLines = 2,
                    shape = RoundedCornerShape(10.dp),
                    colors = irisTextFieldColors(),
                )

                resolvedInput?.errorMessage?.let { error ->
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }

                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    IrisSecondaryButton(
                        text = "Scan code",
                        onClick = { showScanner = true },
                        modifier = Modifier.testTag("deviceRosterScanButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.ScanQr,
                                contentDescription = null,
                            )
                        },
                    )

                    IrisPrimaryButton(
                        text = if (appState.busy.updatingRoster) "Linking…" else "Link device",
                        onClick = {
                            appManager.addAuthorizedDevice(normalizedInput)
                            deviceInput = ""
                        },
                        enabled = canAddDevice,
                        modifier = Modifier.testTag("deviceRosterAddButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Devices,
                                contentDescription = null,
                            )
                        },
                    )
                }
            }
        }

        Text(
            text = "Devices",
            style = MaterialTheme.typography.titleMedium,
        )

        if (embedded) {
            DeviceRosterRows(
                appManager = appManager,
                appState = appState,
                rosterDevices = roster.devices,
                canManageDevices = roster.canManageDevices,
                modifier = Modifier.testTag("deviceRosterList"),
            )
        } else {
            LazyColumn(
                modifier =
                    Modifier
                        .weight(1f)
                        .testTag("deviceRosterList"),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (roster.devices.isEmpty()) {
                    item { DeviceRosterEmptyState() }
                }
                items(roster.devices, key = { it.devicePubkeyHex }) { device ->
                    DeviceRosterRow(
                        device = device,
                        canManageDevices = roster.canManageDevices,
                        isUpdatingRoster = appState.busy.updatingRoster,
                        onApprove = { appManager.addAuthorizedDevice(device.devicePubkeyHex) },
                        onRemove = { appManager.removeAuthorizedDevice(device.devicePubkeyHex) },
                    )
                }
            }
        }
    }

    if (showScanner) {
        QrScannerDialog(
            onDismiss = { showScanner = false },
            onScanned = { scanned ->
                val resolved =
                    resolveDeviceAuthorizationInput(
                        scanned,
                        roster.ownerNpub,
                        roster.ownerPublicKeyHex,
                    )
                if (resolved.errorMessage != null) {
                    resolved.errorMessage
                } else {
                    deviceInput = scanned.trim()
                    showScanner = false
                    null
                }
            },
        )
    }
}

@Composable
private fun DeviceRosterRows(
    appManager: AppManager,
    appState: AppState,
    rosterDevices: List<DeviceEntrySnapshot>,
    canManageDevices: Boolean,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (rosterDevices.isEmpty()) {
            DeviceRosterEmptyState()
        }
        rosterDevices.forEach { device ->
            DeviceRosterRow(
                device = device,
                canManageDevices = canManageDevices,
                isUpdatingRoster = appState.busy.updatingRoster,
                onApprove = { appManager.addAuthorizedDevice(device.devicePubkeyHex) },
                onRemove = { appManager.removeAuthorizedDevice(device.devicePubkeyHex) },
            )
        }
    }
}

@Composable
private fun DeviceRosterEmptyState() {
    IrisListSection(
        modifier = Modifier.testTag("deviceRosterEmptyState"),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "No linked devices",
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                text = "Linked devices will appear here.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
        }
    }
}

@Composable
private fun DeviceRosterRow(
    device: DeviceEntrySnapshot,
    canManageDevices: Boolean,
    isUpdatingRoster: Boolean,
    onApprove: () -> Unit,
    onRemove: () -> Unit,
) {
    val displayTitle = deviceDisplayTitle(device)
    val displaySubtitle = deviceDisplaySubtitle(device)
    var confirmRemoval by remember { mutableStateOf(false) }

    IrisListSection(
        modifier = Modifier.testTag("deviceRosterRow-${device.devicePubkeyHex.take(12)}"),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IrisAvatar(label = displayTitle, size = 42.dp)
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text(
                    text = displayTitle,
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    text = displaySubtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    DeviceStateChip(
                        text = if (device.isAuthorized) "Linked" else "Pending",
                    )
                    if (device.isStale) {
                        DeviceStateChip(
                            text = "Needs attention",
                            containerColor = MaterialTheme.colorScheme.error.copy(alpha = 0.14f),
                            contentColor = MaterialTheme.colorScheme.error,
                        )
                    }
                    val ago = formatRelativeTime(device.addedAtSecs?.toLong())
                    if (ago != null) {
                        DeviceStateChip(text = "Added $ago ago")
                    }
                }
            }
        }

        if (canManageDevices && !device.isCurrentDevice) {
            Row(
                modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (!device.isAuthorized) {
                    IrisPrimaryButton(
                        text = if (isUpdatingRoster) "Linking…" else "Link",
                        onClick = onApprove,
                        enabled = !isUpdatingRoster,
                        modifier =
                            Modifier.testTag(
                                "deviceRosterApprove-${device.devicePubkeyHex.take(12)}",
                            ),
                    )
                }

                IrisSecondaryButton(
                    text = "Remove device",
                    onClick = { confirmRemoval = true },
                    enabled = !isUpdatingRoster,
                    modifier =
                        Modifier.testTag(
                            "deviceRosterRemove-${device.devicePubkeyHex.take(12)}",
                        ),
                )
            }
        }
    }

    if (confirmRemoval) {
        AlertDialog(
            onDismissRequest = { confirmRemoval = false },
            title = { Text("Remove device?") },
            text = {
                Text("This device will no longer use your profile.")
            },
            dismissButton = {
                IrisTextButton(onClick = { confirmRemoval = false }) {
                    Text("Cancel")
                }
            },
            confirmButton = {
                IrisTextButton(
                    onClick = {
                        confirmRemoval = false
                        onRemove()
                    },
                    modifier =
                        Modifier.testTag(
                            "deviceRosterConfirmRemove-${device.devicePubkeyHex.take(12)}",
                        ),
                    destructive = true,
                ) {
                    Text(
                        text = "Delete",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            },
        )
    }
}

private fun deviceDisplayTitle(device: DeviceEntrySnapshot): String =
    if (device.isCurrentDevice) {
        "This device"
    } else {
        device.deviceLabel?.trim()?.takeIf { it.isNotEmpty() } ?: "Linked device"
    }

private fun deviceDisplaySubtitle(device: DeviceEntrySnapshot): String {
    val clientLabel =
        device.clientLabel?.trim()?.takeIf { it.isNotEmpty() }
            ?: if (device.isCurrentDevice) {
                "Iris Chat Android"
            } else {
                "Iris Chat"
            }
    val deviceLabel = device.deviceLabel?.trim()?.takeIf { it.isNotEmpty() }
    if (device.isCurrentDevice && deviceLabel != null) {
        return "$deviceLabel - $clientLabel"
    }
    return clientLabel
}

@Composable
private fun DeviceStateChip(
    text: String,
    containerColor: Color = IrisTheme.palette.panelAlt,
    contentColor: Color = MaterialTheme.colorScheme.onSurface,
) {
    Surface(
        color = containerColor,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(100.dp),
    ) {
        Text(
            text = text,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 5.dp),
            style = MaterialTheme.typography.labelMedium,
            color = contentColor,
        )
    }
}

private data class ResolvedDeviceAuthorizationInput(
    val deviceInput: String,
    val errorMessage: String?,
)

private fun resolveDeviceAuthorizationInput(
    rawInput: String,
    ownerNpub: String,
    ownerPublicKeyHex: String,
): ResolvedDeviceAuthorizationInput {
    val trimmed = rawInput.trim()
    if (trimmed.isEmpty()) {
        return ResolvedDeviceAuthorizationInput(deviceInput = "", errorMessage = null)
    }

    val approvalPayload = DeviceApprovalQr.decode(trimmed)
    if (approvalPayload != null) {
        val normalizedOwner = normalizePeerInput(approvalPayload.ownerInput)
        val acceptedOwnerInputs =
            setOf(
                normalizePeerInput(ownerNpub),
                normalizePeerInput(ownerPublicKeyHex),
            )
        if (normalizedOwner !in acceptedOwnerInputs) {
            return ResolvedDeviceAuthorizationInput(
                deviceInput = "",
                errorMessage = "This code is for a different profile.",
            )
        }

        val normalizedDevice = normalizePeerInput(approvalPayload.deviceInput)
        if (!isValidPeerInput(normalizedDevice)) {
            return ResolvedDeviceAuthorizationInput(
                deviceInput = "",
                errorMessage = "That code is not valid.",
            )
        }
        return ResolvedDeviceAuthorizationInput(deviceInput = normalizedDevice, errorMessage = null)
    }

    if (isLikelyLinkInvite(trimmed)) {
        return ResolvedDeviceAuthorizationInput(deviceInput = trimmed, errorMessage = null)
    }

    val normalizedManualDevice = normalizePeerInput(trimmed)
    if (isValidPeerInput(normalizedManualDevice)) {
        return ResolvedDeviceAuthorizationInput(
            deviceInput = normalizedManualDevice,
            errorMessage = null,
        )
    }

    return ResolvedDeviceAuthorizationInput(
        deviceInput = "",
        errorMessage = "Not a valid link code.",
    )
}

private fun isLikelyLinkInvite(input: String): Boolean {
    val uri = runCatching { Uri.parse(input) }.getOrNull() ?: return false
    if (uri.scheme?.lowercase() != "https" || uri.host?.lowercase() != "chat.iris.to") {
        return false
    }
    if (uri.fragment.isNullOrBlank()) {
        return false
    }
    val decoded = Uri.decode(input)
    return decoded.contains("\"purpose\":\"link\"") &&
        decoded.contains("\"ephemeralKey\"") &&
        decoded.contains("\"sharedSecret\"")
}
