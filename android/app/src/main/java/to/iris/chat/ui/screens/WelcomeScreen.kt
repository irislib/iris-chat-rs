package to.iris.chat.ui.screens

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
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
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun WelcomeScreen(
    appManager: AppManager,
) {
    val scrollState = rememberScrollState()
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            Box(
                modifier =
                    Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .verticalScroll(scrollState)
                        .padding(horizontal = 32.dp, vertical = 40.dp),
                contentAlignment = Alignment.Center,
            ) {
                WelcomeBrand(
                    modifier =
                        Modifier
                            .widthIn(max = 360.dp)
                            .fillMaxWidth()
                            .testTag("welcomeChooserCard"),
                )
            }

            Surface(
                modifier = Modifier.fillMaxWidth(),
                color = MaterialTheme.colorScheme.background,
                shadowElevation = if (scrollState.canScrollForward) 8.dp else 0.dp,
            ) {
                WelcomeActions(
                    appManager = appManager,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .navigationBarsPadding()
                            .padding(horizontal = 32.dp, vertical = 24.dp),
                )
            }
        }
    }
}

@Composable
private fun WelcomeBrand(
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
    }
}

@Composable
private fun WelcomeActions(
    appManager: AppManager,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
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
        if (BuildConfig.TRUSTED_TEST_BUILD) {
            Text(
                text = "Test build",
                modifier = Modifier.testTag("welcomeSecondaryCard"),
                style = MaterialTheme.typography.labelMedium,
                color = IrisTheme.palette.muted,
            )
        }
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

    OnboardingScaffold(
        title = "Create profile",
        onBack = { appManager.dispatch(AppAction.UpdateScreenStack(emptyList())) },
        bottomContent = {
            IrisPrimaryButton(
                text = if (appState.busy.creatingAccount) "Creating…" else "Create profile",
                onClick = submitCreateAccount,
                enabled = canCreateAccount,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("generateKeyButton"),
            )
        },
    ) {
        Column(
            modifier = Modifier.testTag("createAccountScreen"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
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
                shape = RoundedCornerShape(10.dp),
                colors = irisTextFieldColors(),
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

    OnboardingScaffold(
        title = "Restore profile",
        subtitle = "Paste your secret key.",
        onBack = { appManager.dispatch(AppAction.UpdateScreenStack(emptyList())) },
        bottomContent = {
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
        },
    ) {
        Column(
            modifier = Modifier.testTag("restoreAccountScreen"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
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
                shape = RoundedCornerShape(10.dp),
                colors = irisTextFieldColors(),
            )
            Text(
                text = "Secret key = nostr nsec",
                color = IrisTheme.palette.muted,
                style = MaterialTheme.typography.bodySmall,
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

    OnboardingScaffold(
        title = if (awaitingApproval) "Finish linking" else "Link this device",
        subtitle =
            if (awaitingApproval) {
                "Waiting for approval from your signed-in device."
            } else {
                "Scan this code with your signed-in device."
            },
        onBack =
            if (awaitingApproval) {
                null
            } else {
                { appManager.dispatch(AppAction.UpdateScreenStack(emptyList())) }
            },
        bottomContent =
            if (awaitingApproval) {
                {
                    IrisSecondaryButton(
                        text = "Sign out",
                        onClick = { showLogoutConfirmation = true },
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("awaitingApprovalLogoutButton"),
                    )
                }
            } else {
                null
            },
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .testTag("addDeviceScreen"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
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

        if (!awaitingApproval) {
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
private fun OnboardingBackButton(onClick: () -> Unit) {
    IconButton(
        onClick = onClick,
        modifier = Modifier.testTag("onboardingBackButton"),
    ) {
        Icon(
            imageVector = Icons.AutoMirrored.Rounded.ArrowBack,
            contentDescription = "Back",
            tint = MaterialTheme.colorScheme.onSurface,
        )
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
private fun OnboardingScaffold(
    title: String,
    subtitle: String? = null,
    onBack: (() -> Unit)?,
    bottomContent: (@Composable ColumnScope.() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val scrollState = rememberScrollState()
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            Column(
                modifier =
                    Modifier
                        .weight(1f)
                        .verticalScroll(scrollState)
                        .padding(horizontal = 32.dp)
                        .padding(top = 24.dp, bottom = 16.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                if (onBack != null) {
                    OnboardingBackButton(onClick = onBack)
                } else {
                    Spacer(modifier = Modifier.height(48.dp))
                }

                Text(
                    text = title,
                    style = MaterialTheme.typography.headlineMedium,
                    color = MaterialTheme.colorScheme.onBackground,
                )

                if (!subtitle.isNullOrBlank()) {
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.bodyLarge,
                        color = IrisTheme.palette.muted,
                    )
                }

                Spacer(modifier = Modifier.height(8.dp))
                content()
            }

            if (bottomContent != null) {
                Surface(
                    modifier = Modifier.fillMaxWidth(),
                    color = MaterialTheme.colorScheme.background,
                    shadowElevation = if (scrollState.canScrollForward) 8.dp else 0.dp,
                ) {
                    Column(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .navigationBarsPadding()
                                .imePadding()
                                .padding(horizontal = 32.dp)
                                .padding(top = 8.dp, bottom = 24.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                        content = bottomContent,
                    )
                }
            }
        }
    }
}
