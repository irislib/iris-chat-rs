package to.iris.chat.ui.screens

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material.icons.rounded.Key
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import to.iris.chat.BuildConfig
import to.iris.chat.R
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
    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 28.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            WelcomeHero(
                appManager = appManager,
                modifier =
                    Modifier
                        .widthIn(max = 360.dp)
                        .fillMaxWidth()
                        .testTag("welcomeChooserCard"),
            )
            if (BuildConfig.TRUSTED_TEST_BUILD) {
                WelcomeTrustedBuildCard(
                    modifier =
                        Modifier
                            .widthIn(max = 360.dp)
                            .fillMaxWidth()
                            .testTag("welcomeSecondaryCard"),
                )
            }
        }
    }
}

@Composable
private fun WelcomeHero(
    appManager: AppManager,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        Image(
            painter = painterResource(id = R.drawable.iris_logo),
            contentDescription = null,
            modifier = Modifier.size(132.dp),
        )

        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = "iris",
                style = MaterialTheme.typography.headlineLarge,
                color = IrisTheme.palette.accent,
                fontWeight = FontWeight.ExtraBold,
            )
            Text(
                text = " chat",
                style = MaterialTheme.typography.headlineLarge,
                color = MaterialTheme.colorScheme.onBackground,
                fontWeight = FontWeight.ExtraBold,
            )
        }

        Column(
            modifier =
                Modifier
                    .widthIn(max = 320.dp)
                    .fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            IrisPrimaryButton(
                text = "Create account",
                onClick = { appManager.pushScreen(Screen.CreateAccount) },
                icon = {
                    Icon(
                        imageVector = Icons.Rounded.Add,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp),
                    )
                },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("welcomeCreateAction"),
            )
            IrisSecondaryButton(
                text = "Restore account",
                onClick = { appManager.pushScreen(Screen.RestoreAccount) },
                icon = {
                    Icon(
                        imageVector = Icons.Rounded.Key,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp),
                    )
                },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("welcomeRestoreAction"),
            )
            IrisSecondaryButton(
                text = "Link this device",
                onClick = { appManager.pushScreen(Screen.AddDevice) },
                icon = {
                    Icon(
                        imageVector = Icons.Rounded.Devices,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp),
                    )
                },
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
    IrisSectionCard(modifier = modifier) {
        Text(
            text = "Test build",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
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
            TextField(
                value = displayName,
                onValueChange = { displayName = it },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("signupNameField"),
                placeholder = {
                    Text(
                        text = "Name",
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
                text = "Paste your secret key.",
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
                        text = "Secret key",
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
                text = if (awaitingApproval) "Finish linking" else "Link this device",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text =
                    if (awaitingApproval) {
                        "Use your signed-in device to approve this one. If it asks for a code, scan the code below."
                    } else {
                        "Scan the account code from your signed-in device, or paste its user ID."
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
                        text = "That code or user ID is not valid.",
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
                        text = "Scan account code",
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
                    IrisSecondaryButton(
                        text = "Copy user ID",
                        onClick = { clipboard.setText("User ID", account.npub) },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("awaitingApprovalOwnerNpub"),
                    )
                    IrisSecondaryButton(
                        text = "Copy this device code",
                        onClick = { clipboard.setText("Link code", account.deviceNpub) },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("awaitingApprovalDeviceNpub"),
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
                    text = "Sign out",
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
                    "That code or user ID is not valid."
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
            text = "Approval code",
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            text = "Scan this from Manage devices on your signed-in device.",
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
                    contentDescription = "Approval code",
                    modifier =
                        Modifier
                            .size(260.dp)
                            .testTag("awaitingApprovalDeviceQrCode"),
                )
            }
        }
        IrisSecondaryButton(
            text = "Copy approval code",
            onClick = { clipboard.setText("Approval code", approvalQrValue) },
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
private fun irisTextFieldColors() =
    TextFieldDefaults.colors(
        focusedContainerColor = IrisTheme.palette.panelAlt,
        unfocusedContainerColor = IrisTheme.palette.panelAlt,
        disabledContainerColor = IrisTheme.palette.panelAlt,
        focusedIndicatorColor = Color.Transparent,
        unfocusedIndicatorColor = Color.Transparent,
        disabledIndicatorColor = Color.Transparent,
    )
