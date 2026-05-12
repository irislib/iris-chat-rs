package to.iris.chat.ui.screens

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun CreateInviteScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
    val canShareInvite = remember(context) { canShareText(context) }
    val inviteUrl = appState.publicInvite?.url
    val qrBitmap = remember(inviteUrl) {
        inviteUrl?.let { createQrBitmap(it, size = 768) }
    }

    LaunchedEffect(inviteUrl) {
        if (inviteUrl == null) {
            appManager.dispatch(AppAction.CreatePublicInvite)
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Invite",
                onBack = { appManager.navigateBack() },
            )
        },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            if (qrBitmap != null && inviteUrl != null) {
                Image(
                    bitmap = qrBitmap.asImageBitmap(),
                    contentDescription = "Invite code",
                    modifier =
                        Modifier
                            .align(Alignment.CenterHorizontally)
                            .size(260.dp)
                            .background(Color.White)
                            .padding(12.dp)
                            .testTag("createInviteQrCode"),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    IrisSecondaryButton(
                        text = "Copy",
                        onClick = { clipboard.setText("Invite", inviteUrl) },
                        modifier = Modifier.weight(1f).testTag("createInviteCopyButton"),
                        icon = {
                            Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                        },
                    )
                    if (canShareInvite) {
                        IrisPrimaryButton(
                            text = "Share",
                            onClick = { shareText(context, inviteUrl, "Share invite") },
                            modifier = Modifier.weight(1f).testTag("createInviteShareButton"),
                            icon = {
                                Icon(imageVector = IrisIcons.Share, contentDescription = null)
                            },
                        )
                    }
                }
            }

            IrisSecondaryButton(
                text = if (appState.busy.creatingInvite) "Creating…" else "New invite",
                onClick = { appManager.dispatch(AppAction.CreatePublicInvite) },
                enabled = !appState.busy.creatingInvite,
                modifier = Modifier.fillMaxWidth().testTag("createInviteRefreshButton"),
                icon = {
                    Icon(imageVector = IrisIcons.Refresh, contentDescription = null)
                },
            )
        }
    }
}

@Composable
fun JoinInviteScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val clipboard = rememberIrisClipboard()
    var inviteInput by remember { mutableStateOf("") }
    var showScanner by remember { mutableStateOf(false) }
    val trimmedInput = inviteInput.trim()

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Join chat",
                onBack = { appManager.navigateBack() },
            )
        },
    ) { padding ->
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
                    text = "Join Chat",
                    style = MaterialTheme.typography.titleLarge,
                )
                TextField(
                    value = inviteInput,
                    onValueChange = { inviteInput = it },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("joinInviteInput"),
                    placeholder = {
                        Text(
                            text = "Invite",
                            color = IrisTheme.palette.muted,
                        )
                    },
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

                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    IrisSecondaryButton(
                        text = "Paste",
                        onClick = {
                            clipboard.getText { text -> inviteInput = text }
                        },
                        modifier = Modifier.testTag("joinInvitePasteButton"),
                        icon = {
                            Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                        },
                    )
                    IrisSecondaryButton(
                        text = "Scan code",
                        onClick = { showScanner = true },
                        modifier = Modifier.testTag("joinInviteScanQrButton"),
                        icon = {
                            Icon(imageVector = IrisIcons.ScanQr, contentDescription = null)
                        },
                    )
                }

                IrisPrimaryButton(
                    text = if (appState.busy.acceptingInvite) "Joining…" else "Join chat",
                    onClick = {
                        appManager.dispatch(AppAction.AcceptInvite(trimmedInput))
                    },
                    enabled = trimmedInput.isNotEmpty() && !appState.busy.acceptingInvite,
                    modifier = Modifier.fillMaxWidth().testTag("joinInviteAcceptButton"),
                    icon = {
                        Icon(imageVector = IrisIcons.NewChat, contentDescription = null)
                    },
                )
            }
        }
    }

    if (showScanner) {
        QrScannerDialog(
            onDismiss = { showScanner = false },
            onScanned = { scanned ->
                inviteInput = scanned
                showScanner = false
                null
            },
        )
    }
}
