package to.iris.chat.ui.screens

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import to.iris.chat.BuildConfig
import to.iris.chat.core.AppManager
import to.iris.chat.qr.DeviceApprovalQr
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.Screen
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun WelcomeScreen(
    appManager: AppManager,
) {
    OnboardingColumn {
        Column(
            modifier = Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            WelcomeHeroCard(
                appManager = appManager,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .widthIn(max = 520.dp)
                        .testTag("welcomeChooserCard"),
            )
            if (BuildConfig.TRUSTED_TEST_BUILD) {
                WelcomeTrustedBuildCard(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .widthIn(max = 520.dp)
                            .testTag("welcomeSecondaryCard"),
                )
            }
        }
    }
}

@Composable
private fun WelcomeHeroCard(
    appManager: AppManager,
    modifier: Modifier = Modifier,
) {
    IrisSectionCard(modifier = modifier) {
        Text(
            text = "Iris Chat",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
        )

        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            IrisPrimaryButton(
                text = "Create account",
                onClick = { appManager.pushScreen(Screen.CreateAccount) },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("welcomeCreateAction"),
            )
            IrisSecondaryButton(
                text = "Restore account",
                onClick = { appManager.pushScreen(Screen.RestoreAccount) },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("welcomeRestoreAction"),
            )
            IrisSecondaryButton(
                text = "Add this device",
                onClick = { appManager.pushScreen(Screen.AddDevice) },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("welcomeAddDeviceAction"),
            )
        }
    }
}

@Composable
private fun WelcomeTrustedBuildCard(
    modifier: Modifier = Modifier,
) {
    val palette = IrisTheme.palette
    IrisSectionCard(modifier = modifier) {
        Text(
            text = "Trusted test build",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            text = "Build ${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD_GIT_SHA})",
            style = MaterialTheme.typography.bodyMedium,
            color = palette.muted,
        )
    }
}

@Composable
fun CreateAccountScreen(
    appManager: AppManager,
    appState: AppState,
) {
    var displayName by rememberSaveable { mutableStateOf("") }

    OnboardingColumn {
        BackToWelcomeButton(appManager = appManager)

        IrisSectionCard(modifier = Modifier.testTag("createAccountScreen")) {
            Text(
                text = "Create account",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text = "Generate a fresh owner account on this device and jump straight into chats.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
            TextField(
                value = displayName,
                onValueChange = { displayName = it },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("signupNameField"),
                placeholder = {
                    Text(
                        text = "Display name",
                        color = IrisTheme.palette.muted,
                    )
                },
                singleLine = true,
                enabled = !appState.busy.creatingAccount,
                colors = irisTextFieldColors(),
            )
            IrisPrimaryButton(
                text = if (appState.busy.creatingAccount) "Creating…" else "Create account",
                onClick = { appManager.createAccount(displayName) },
                enabled =
                    displayName.trim().isNotEmpty() &&
                        !appState.busy.creatingAccount,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("generateKeyButton"),
            )
        }

        OnboardingMessageCard(message = appState.toast)
    }
}

@Composable
fun RestoreAccountScreen(
    appManager: AppManager,
    appState: AppState,
) {
    var restoreInput by rememberSaveable { mutableStateOf("") }

    OnboardingColumn {
        BackToWelcomeButton(appManager = appManager)

        IrisSectionCard(modifier = Modifier.testTag("restoreAccountScreen")) {
            Text(
                text = "Restore account",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text = "Use your owner secret key to recover your account on this device.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
            TextField(
                value = restoreInput,
                onValueChange = { restoreInput = it },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("importKeyField"),
                placeholder = {
                    Text(
                        text = "Owner nsec",
                        color = IrisTheme.palette.muted,
                    )
                },
                minLines = 3,
                enabled = !appState.busy.restoringSession,
                colors = irisTextFieldColors(),
            )
            IrisPrimaryButton(
                text = if (appState.busy.restoringSession) "Restoring…" else "Restore account",
                onClick = { appManager.restoreSession(restoreInput) },
                enabled =
                    restoreInput.trim().isNotEmpty() &&
                        !appState.busy.restoringSession,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("importKeyButton"),
            )
        }

        OnboardingMessageCard(message = appState.toast)
    }
}

@Composable
fun AddDeviceScreen(
    appManager: AppManager,
    appState: AppState,
    awaitingApproval: Boolean,
) {
    var ownerInput by rememberSaveable { mutableStateOf("") }
    var showScanner by remember { mutableStateOf(false) }
    val clipboard = rememberIrisClipboard()
    val normalizedOwnerInput = normalizePeerInput(ownerInput)
    val isValidOwnerInput =
        normalizedOwnerInput.isNotBlank() && isValidPeerInput(normalizedOwnerInput)

    OnboardingColumn {
        if (!awaitingApproval) {
            BackToWelcomeButton(appManager = appManager)
        }

        IrisSectionCard(modifier = Modifier.testTag("addDeviceScreen")) {
            Text(
                text = if (awaitingApproval) "Finish linking" else "Add this device",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text =
                    if (awaitingApproval) {
                        "Approve this device on the owner device. If it does not appear in the roster yet, scan the QR below as a fallback."
                    } else {
                        "Scan or paste the owner code from your primary device. This device will create its own invite and then wait for approval there."
                    },
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )

            if (!awaitingApproval) {
                TextField(
                    value = ownerInput,
                    onValueChange = { ownerInput = it },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("linkOwnerInput"),
                    placeholder = {
                        Text(
                            text = "User ID",
                            color = IrisTheme.palette.muted,
                        )
                    },
                    isError = ownerInput.isNotBlank() && !isValidOwnerInput,
                    enabled = !appState.busy.linkingDevice,
                    colors = irisTextFieldColors(),
                )

                if (ownerInput.isNotBlank() && !isValidOwnerInput) {
                    Text(
                        text = "Scanned or pasted owner key is not valid.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }

                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    IrisSecondaryButton(
                        text = "Paste",
                        onClick = {
                            clipboard.getText { text ->
                                ownerInput = normalizePeerInput(text)
                            }
                        },
                        enabled = !appState.busy.linkingDevice,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("linkOwnerPasteButton"),
                    )
                    IrisSecondaryButton(
                        text = "Scan owner QR",
                        onClick = { showScanner = true },
                        enabled = !appState.busy.linkingDevice,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("linkOwnerScanQrButton"),
                    )
                    IrisPrimaryButton(
                        text = if (appState.busy.linkingDevice) "Continuing…" else "Continue",
                        onClick = { appManager.startLinkedDevice(normalizedOwnerInput) },
                        enabled = isValidOwnerInput && !appState.busy.linkingDevice,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("linkExistingAccountButton"),
                    )
                }
            } else {
                appState.account?.let { account ->
                    MonoValue(
                        label = "Owner",
                        value = account.npub,
                        identifier = "awaitingApprovalOwnerNpub",
                    )
                    MonoValue(
                        label = "Device ID",
                        value = account.deviceNpub,
                        identifier = "awaitingApprovalDeviceNpub",
                    )
                }
            }
        }

        AddDeviceQrPanel(
            appManager = appManager,
            appState = appState,
            awaitingApproval = awaitingApproval,
        )

        if (awaitingApproval) {
            IrisSectionCard {
                IrisSecondaryButton(
                    text = "Logout",
                    onClick = appManager::logout,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        } else {
            OnboardingMessageCard(message = appState.toast)
        }
    }

    if (showScanner) {
        QrScannerDialog(
            onDismiss = { showScanner = false },
            onScanned = { scanned ->
                val normalized = normalizePeerInput(scanned)
                if (!isValidPeerInput(normalized)) {
                    "Scanned QR did not contain a valid owner public key."
                } else {
                    ownerInput = normalized
                    showScanner = false
                    null
                }
            },
        )
    }
}

@Composable
private fun AddDeviceQrPanel(
    appManager: AppManager,
    appState: AppState,
    awaitingApproval: Boolean,
) {
    val clipboard = rememberIrisClipboard()
    val account = appState.account

    if (!awaitingApproval || account == null) {
        IrisSectionCard(modifier = Modifier.testTag("addDeviceQrPlaceholder")) {
            Text(
                text = "Approval QR",
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                text = "After you continue, the approval QR for this device will appear here so the owner can authorize it.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 12.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = "QR placeholder",
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                )
            }
        }
        return
    }

    val approvalQrValue =
        remember(account.npub, account.deviceNpub) {
            DeviceApprovalQr.encode(
                ownerInput = account.npub,
                deviceInput = account.deviceNpub,
            )
        }
    val qrBitmap =
        remember(approvalQrValue) {
            createQrBitmap(approvalQrValue, size = 768)
        }

    IrisSectionCard(modifier = Modifier.testTag("awaitingApprovalScreen")) {
        Text(
            text = "Approval QR",
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            text = "Approve this device from Manage devices on the owner device, or scan the QR below there as a fallback.",
            style = MaterialTheme.typography.bodyMedium,
            color = IrisTheme.palette.muted,
        )
        Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            if (qrBitmap != null) {
                Image(
                    bitmap = qrBitmap.asImageBitmap(),
                    contentDescription = "Device approval QR code",
                    modifier =
                        Modifier
                            .size(260.dp)
                            .testTag("awaitingApprovalDeviceQrCode"),
                )
            }
        }
        IrisSecondaryButton(
            text = "Copy approval QR",
            onClick = { clipboard.setText("Approval QR", approvalQrValue) },
            modifier =
                Modifier
                    .fillMaxWidth()
                    .testTag("awaitingApprovalCopyDeviceButton"),
        )
    }
}

@Composable
private fun BackToWelcomeButton(appManager: AppManager) {
    TextButton(
        onClick = { appManager.dispatch(AppAction.UpdateScreenStack(emptyList())) },
        modifier = Modifier.testTag("onboardingBackButton"),
    ) {
        Text("Back")
    }
}

@Composable
private fun OnboardingMessageCard(message: String?) {
    val resolved = message?.takeIf { it.isNotBlank() } ?: return
    IrisSectionCard {
        Text(
            text = resolved,
            color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}

@Composable
private fun OnboardingColumn(
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
        content = content,
    )
}

@Composable
private fun MonoValue(
    label: String,
    value: String,
    identifier: String,
) {
    Text(
        text = label,
        style = MaterialTheme.typography.titleSmall,
    )
    Text(
        text = value,
        style = MaterialTheme.typography.bodyMedium,
        modifier = Modifier.testTag(identifier),
    )
}

@Composable
private fun irisTextFieldColors() =
    TextFieldDefaults.colors(
        focusedContainerColor = IrisTheme.palette.panelAlt,
        unfocusedContainerColor = IrisTheme.palette.panelAlt,
        disabledContainerColor = IrisTheme.palette.panelAlt,
        focusedIndicatorColor = Color.Transparent,
        unfocusedIndicatorColor = Color.Transparent,
        disabledIndicatorColor = Color.Transparent,
    )
