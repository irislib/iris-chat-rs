package to.iris.chat.ui.screens

import android.os.Build
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
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
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.DeviceEntrySnapshot
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun DeviceRosterScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val roster = appState.deviceRoster
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

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Manage devices",
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(appState.router.screenStack.dropLast(1)),
                    )
                },
            )
        },
    ) { padding ->
        if (roster == null) {
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Loading roster…")
            }
            return@Scaffold
        }

        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IrisSectionCard {
                Text(
                    text = "Account devices",
                    style = MaterialTheme.typography.titleLarge,
                )
                Text(
                    text = "Primary devices publish the owner-signed roster. Linked devices can view it, publish their own invite, and send messages once authorized.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                )
                Text(
                    text = "User ID",
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = roster.ownerNpub,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.testTag("deviceRosterOwnerNpub"),
                )
                Text(
                    text = "Device ID",
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = roster.currentDeviceNpub,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.testTag("deviceRosterCurrentDeviceNpub"),
                )
            }

            IrisSectionCard {
                Text(
                    text = "Approve a new device",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text =
                        if (roster.canManageDevices) {
                            "Scan a link invite from the new device, or paste a device ID as fallback."
                        } else if (isCurrentDeviceRegistered) {
                            "Read-only on this device. Use a session with your main Secret Key to add or remove devices."
                        } else {
                            "This linked-device session is read-only and is not registered. Sign in here with your main Secret Key if you want to register this device."
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
                                text = "Device ID or approval code",
                                color = IrisTheme.palette.muted,
                            )
                        },
                        isError = deviceInput.isNotBlank() && resolvedInput?.errorMessage != null,
                        minLines = 2,
                        colors =
                            TextFieldDefaults.colors(
                                focusedContainerColor = IrisTheme.palette.panelAlt,
                                unfocusedContainerColor = IrisTheme.palette.panelAlt,
                                disabledContainerColor = IrisTheme.palette.panelAlt,
                                focusedIndicatorColor = Color.Transparent,
                                unfocusedIndicatorColor = Color.Transparent,
                                disabledIndicatorColor = Color.Transparent,
                            ),
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
                            text = "Scan QR",
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
                            text = if (appState.busy.updatingRoster) "Authorizing…" else "Authorize",
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
                text = "Device Access",
                style = MaterialTheme.typography.titleMedium,
            )

            LazyColumn(
                modifier =
                    Modifier
                        .weight(1f)
                        .testTag("deviceRosterList"),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (roster.devices.isEmpty()) {
                    item {
                        IrisSectionCard(
                            modifier = Modifier.testTag("deviceRosterEmptyState"),
                        ) {
                            Text(
                                text = "No registered devices",
                                style = MaterialTheme.typography.titleMedium,
                            )
                            Text(
                                text = "Authorized device keys will appear here after the roster is published.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = IrisTheme.palette.muted,
                            )
                        }
                    }
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

    if (showScanner && roster != null) {
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
                    deviceInput = resolved.deviceInput
                    showScanner = false
                    null
                }
            },
        )
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

    IrisSectionCard(
        modifier = Modifier.testTag("deviceRosterRow-${device.devicePubkeyHex.take(12)}"),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
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
                        text = if (device.isAuthorized) "Authorized" else "Pending",
                    )
                    if (device.isStale) {
                        DeviceStateChip(
                            text = "Stale",
                            containerColor = MaterialTheme.colorScheme.error.copy(alpha = 0.14f),
                            contentColor = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
        }

        if (canManageDevices && !device.isCurrentDevice) {
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (!device.isAuthorized) {
                    IrisPrimaryButton(
                        text = if (isUpdatingRoster) "Approving…" else "Approve",
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
            title = { Text("Delete Device?") },
            text = {
                Text("This device will no longer be authorized for encrypted messaging.")
            },
            dismissButton = {
                TextButton(onClick = { confirmRemoval = false }) {
                    Text("Cancel")
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmRemoval = false
                        onRemove()
                    },
                    modifier =
                        Modifier.testTag(
                            "deviceRosterConfirmRemove-${device.devicePubkeyHex.take(12)}",
                        ),
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
        currentDeviceLabel()
    } else {
        "Linked device"
    }

private fun deviceDisplaySubtitle(device: DeviceEntrySnapshot): String {
    val clientLabel =
        if (device.isCurrentDevice) {
            "Iris Chat Mobile"
        } else {
            "Iris Chat"
        }
    return "$clientLabel - ${device.deviceNpub}"
}

private fun currentDeviceLabel(): String {
    val model = Build.MODEL.trim()
    return model.ifEmpty { "Android device" }
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
                errorMessage = "This approval QR belongs to a different owner.",
            )
        }

        val normalizedDevice = normalizePeerInput(approvalPayload.deviceInput)
        if (!isValidPeerInput(normalizedDevice)) {
            return ResolvedDeviceAuthorizationInput(
                deviceInput = "",
                errorMessage = "The approval QR did not contain a valid device key.",
            )
        }
        return ResolvedDeviceAuthorizationInput(deviceInput = normalizedDevice, errorMessage = null)
    }

    val normalized = normalizePeerInput(trimmed)
    return if (isValidPeerInput(normalized)) {
        ResolvedDeviceAuthorizationInput(deviceInput = normalized, errorMessage = null)
    } else {
        ResolvedDeviceAuthorizationInput(
            deviceInput = "",
            errorMessage = "Not a valid device ID or approval code.",
        )
    }
}
