package social.innode.ndr.demo.ui.screens

import android.content.Intent
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
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisPrimaryButton
import social.innode.ndr.demo.ui.components.IrisSectionCard
import social.innode.ndr.demo.ui.components.IrisSecondaryButton
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

@Composable
fun CreateInviteScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
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
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(appState.router.screenStack.dropLast(1)),
                    )
                },
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
                    contentDescription = "Invite QR code",
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
                        onClick = { clipboard.setText("Invite link", inviteUrl) },
                        modifier = Modifier.weight(1f).testTag("createInviteCopyButton"),
                        icon = {
                            Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                        },
                    )
                    IrisPrimaryButton(
                        text = "Share",
                        onClick = {
                            val intent =
                                Intent(Intent.ACTION_SEND)
                                    .setType("text/plain")
                                    .putExtra(Intent.EXTRA_TEXT, inviteUrl)
                            context.startActivity(Intent.createChooser(intent, "Share invite"))
                        },
                        modifier = Modifier.weight(1f).testTag("createInviteShareButton"),
                        icon = {
                            Icon(imageVector = IrisIcons.Share, contentDescription = null)
                        },
                    )
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
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(appState.router.screenStack.dropLast(1)),
                    )
                },
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
                            text = "Invite link",
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
                        text = "Scan QR",
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
