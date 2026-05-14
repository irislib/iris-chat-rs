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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material3.Icon
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import to.iris.chat.BuildConfig
import to.iris.chat.R
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.Screen
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
                text = "Create profile",
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
                text = "Restore profile",
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
    val focusRequester = remember { FocusRequester() }
    val keyboardController = LocalSoftwareKeyboardController.current
    val canCreateAccount =
        displayName.trim().isNotEmpty() &&
            !appState.busy.creatingAccount
    val submitCreateAccount = {
        if (canCreateAccount) {
            appManager.createAccount(displayName.trim())
        }
    }

    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
        keyboardController?.show()
    }

    OnboardingColumn {
        BackToWelcomeButton(appManager = appManager)

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .testTag("createAccountScreen"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "Create profile",
                style = MaterialTheme.typography.headlineSmall,
            )
            TextField(
                value = displayName,
                onValueChange = { displayName = it },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .focusRequester(focusRequester)
                        .testTag("signupNameField"),
                placeholder = {
                    Text(
                        text = "Name",
                        color = IrisTheme.palette.muted,
                    )
                },
                singleLine = true,
                enabled = !appState.busy.creatingAccount,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions =
                    KeyboardActions(
                        onDone = {
                            submitCreateAccount()
                        },
                    ),
                colors = irisTextFieldColors(),
            )
            IrisPrimaryButton(
                text = if (appState.busy.creatingAccount) "Creating…" else "Create profile",
                onClick = submitCreateAccount,
                enabled = canCreateAccount,
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
    var lastSubmittedSecret by rememberSaveable { mutableStateOf<String?>(null) }

    OnboardingColumn {
        BackToWelcomeButton(appManager = appManager)

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .testTag("restoreAccountScreen"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "Restore profile",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text = "Paste your secret key.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
            TextField(
                value = restoreInput,
                onValueChange = { value ->
                    val previous = restoreInput.trim()
                    restoreInput = value
                    val current = value.trim()
                    if (
                        !appState.busy.restoringSession &&
                        current != lastSubmittedSecret &&
                        shouldAutoSubmitSecret(previous = previous, current = current)
                    ) {
                        lastSubmittedSecret = current
                        appManager.restoreSession(current)
                    }
                },
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
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                enabled = !appState.busy.restoringSession,
                colors = irisTextFieldColors(),
            )
            IrisPrimaryButton(
                text = if (appState.busy.restoringSession) "Restoring…" else "Restore profile",
                onClick = {
                    lastSubmittedSecret = restoreInput.trim()
                    appManager.restoreSession(restoreInput)
                },
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

private fun shouldAutoSubmitSecret(
    previous: String,
    current: String,
): Boolean {
    if (current.isBlank()) return false
    val pasted = current.length > previous.length + 4
    val lower = current.lowercase()
    if (lower.startsWith("nsec1")) {
        return pasted || current.length >= 63
    }
    return current.length == 64 && current.all { it.isAsciiHexDigit() }
}

private fun Char.isAsciiHexDigit(): Boolean =
    this in '0'..'9' || this in 'a'..'f' || this in 'A'..'F'

@Composable
fun AddDeviceScreen(
    appManager: AppManager,
    appState: AppState,
    awaitingApproval: Boolean,
) {
    var showLogoutConfirmation by remember { mutableStateOf(false) }
    val clipboard = rememberIrisClipboard()

    LaunchedEffect(awaitingApproval, appState.linkDevice, appState.busy.linkingDevice) {
        if (!awaitingApproval && appState.linkDevice == null && !appState.busy.linkingDevice) {
            appManager.startLinkedDevice("")
        }
    }

    OnboardingColumn {
        if (!awaitingApproval) {
            BackToWelcomeButton(appManager = appManager)
        }

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .testTag("addDeviceScreen"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = if (awaitingApproval) "Finish linking" else "Link this device",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text =
                    if (awaitingApproval) {
                        "Waiting for approval from your signed-in device."
                    } else {
                        "Scan this code with your signed-in device."
                    },
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )

            if (!awaitingApproval) {
                val linkDevice = appState.linkDevice
                if (linkDevice == null) {
                    Box(
                        modifier = Modifier.fillMaxWidth(),
                        contentAlignment = Alignment.Center,
                    ) {
                        CircularProgressIndicator(modifier = Modifier.testTag("linkDeviceCreating"))
                    }
                } else {
                    val qrBitmap =
                        remember(linkDevice.url) {
                            createQrBitmap(linkDevice.url, size = 768)
                        }
                    Box(
                        modifier = Modifier.fillMaxWidth(),
                        contentAlignment = Alignment.Center,
                    ) {
                        if (qrBitmap != null) {
                            Image(
                                bitmap = qrBitmap.asImageBitmap(),
                                contentDescription = "Link code",
                                modifier =
                                    Modifier
                                        .size(260.dp)
                                        .testTag("linkDeviceQrCode"),
                            )
                        }
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        IrisSecondaryButton(
                            text = "Copy link code",
                            onClick = { clipboard.setText("Link code", linkDevice.url) },
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .testTag("linkDeviceCopyButton"),
                        )
                        IrisSecondaryButton(
                            text = if (appState.busy.linkingDevice) "Creating…" else "New code",
                            onClick = { appManager.startLinkedDevice("") },
                            enabled = !appState.busy.linkingDevice,
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .testTag("linkDeviceRefreshButton"),
                        )
                    }
                }
            }
        }

        if (awaitingApproval) {
            IrisSecondaryButton(
                text = "Sign out",
                onClick = { showLogoutConfirmation = true },
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("awaitingApprovalLogoutButton"),
            )
        } else {
            OnboardingMessageCard(message = appState.toast)
        }
    }

    if (showLogoutConfirmation) {
        DeleteAppDataConfirmationDialog(
            onDismiss = { showLogoutConfirmation = false },
            onConfirm = {
                showLogoutConfirmation = false
                appManager.logout()
            },
            confirmTag = "awaitingApprovalConfirmLogoutButton",
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
                .padding(horizontal = 24.dp, vertical = 40.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
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
