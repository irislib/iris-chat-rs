package to.iris.chat.ui.screens

import android.content.Intent
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.ui.window.Dialog
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
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.Screen
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun NewChatScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
    val focusManager = LocalFocusManager.current
    var peerInput by remember { mutableStateOf("") }
    var submittedInput by remember { mutableStateOf<String?>(null) }
    var showScanner by remember { mutableStateOf(false) }
    var showInviteQr by remember { mutableStateOf(false) }
    val trimmedInput = peerInput.trim()
    val normalizedInput = normalizePeerInput(peerInput)
    val isValidPeer = normalizedInput.isNotBlank() && isValidPeerInput(normalizedInput)
    val looksLikeInviteLink =
        trimmedInput.lowercase().let { it.contains("://") && it.contains("#") }

    val inviteUrl = appState.publicInvite?.url
    val qrBitmap = remember(inviteUrl) {
        inviteUrl?.let { createQrBitmap(it, size = 768) }
    }

    LaunchedEffect(Unit) {
        if (appState.publicInvite == null && !appState.busy.creatingInvite) {
            appManager.dispatch(AppAction.CreatePublicInvite)
        }
    }

    LaunchedEffect(isValidPeer, normalizedInput, looksLikeInviteLink, trimmedInput) {
        if (isValidPeer && submittedInput != normalizedInput) {
            submittedInput = normalizedInput
            focusManager.clearFocus()
            appManager.createChat(normalizedInput)
        } else if (looksLikeInviteLink && submittedInput != trimmedInput) {
            submittedInput = trimmedInput
            focusManager.clearFocus()
            appManager.dispatch(AppAction.AcceptInvite(trimmedInput))
        }
    }

    fun handleNewChatInput(raw: String) {
        val normalized = normalizePeerInput(raw)
        if (normalized.isNotBlank() && isValidPeerInput(normalized)) {
            peerInput = normalized
            submittedInput = normalized
            appManager.createChat(normalized)
            return
        }

        val trimmed = raw.trim()
        if (trimmed.isNotBlank()) {
            peerInput = trimmed
            submittedInput = trimmed
            appManager.dispatch(AppAction.AcceptInvite(trimmed))
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "New chat",
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
            // New Chat card
            IrisSectionCard {
                Text(
                    text = "New Chat",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.fillMaxWidth(),
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                )

                if (inviteUrl != null) {
                    Text(
                        text = "Share an invite link to start a chat",
                        style = MaterialTheme.typography.bodySmall,
                        color = IrisTheme.palette.muted,
                        modifier = Modifier.fillMaxWidth(),
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    )

                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        IrisSecondaryButton(
                            text = "Copy",
                            onClick = { clipboard.setText("Invite link", inviteUrl) },
                            modifier = Modifier.weight(1f).testTag("newChatInviteCopyButton"),
                            icon = {
                                Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                            },
                        )
                        IrisSecondaryButton(
                            text = "Show",
                            onClick = { showInviteQr = true },
                            modifier = Modifier.testTag("newChatInviteQrButton"),
                            icon = {
                                Icon(imageVector = IrisIcons.ScanQr, contentDescription = null)
                            },
                        )
                    }
                } else {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 24.dp),
                        horizontalArrangement = Arrangement.Center,
                    ) {
                        CircularProgressIndicator()
                    }
                }
            }

            // Join Chat card
            IrisSectionCard {
                Text(
                    text = "Join Chat",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.fillMaxWidth(),
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                )

                TextField(
                    value = peerInput,
                    onValueChange = { peerInput = it },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("newChatPeerInput"),
                    placeholder = {
                        Text(
                            text = "Paste invite link",
                            color = IrisTheme.palette.muted,
                        )
                    },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions =
                        KeyboardActions(
                            onDone = {
                                focusManager.clearFocus()
                                handleNewChatInput(peerInput)
                            },
                        ),
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

                IrisSecondaryButton(
                    text = "Scan QR Code",
                    onClick = { showScanner = true },
                    modifier = Modifier.fillMaxWidth().testTag("newChatScanQrButton"),
                    icon = {
                        Icon(imageVector = IrisIcons.ScanQr, contentDescription = null)
                    },
                )
            }

            NewChatActionRow(
                text = "Create group",
                icon = { Icon(imageVector = IrisIcons.NewGroup, contentDescription = null) },
                modifier = Modifier.testTag("newChatNewGroupButton"),
                onClick = { appManager.pushScreen(Screen.NewGroup) },
            )
        }
    }

    if (showScanner) {
        QrScannerDialog(
            onDismiss = { showScanner = false },
            onScanned = { scanned ->
                if (scanned.isNotBlank()) {
                    handleNewChatInput(scanned)
                    showScanner = false
                    null
                } else {
                    "Scanned QR was empty."
                }
            },
        )
    }

    if (showInviteQr && qrBitmap != null && inviteUrl != null) {
        Dialog(onDismissRequest = { showInviteQr = false }) {
            Surface(
                color = IrisTheme.palette.panel,
                shape = RoundedCornerShape(20.dp),
                border = BorderStroke(1.dp, IrisTheme.palette.border),
            ) {
                Column(
                    modifier = Modifier.padding(20.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Text(
                        text = "Invite QR Code",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                    )
                    Image(
                        bitmap = qrBitmap.asImageBitmap(),
                        contentDescription = "Invite QR code",
                        modifier =
                            Modifier
                                .size(280.dp)
                                .background(Color.White)
                                .padding(12.dp)
                                .testTag("newChatInviteQrCode"),
                    )
                    Text(
                        text = "Scan this code to start a chat",
                        style = MaterialTheme.typography.bodySmall,
                        color = IrisTheme.palette.muted,
                    )
                    IrisSecondaryButton(
                        text = "Copy",
                        onClick = { clipboard.setText("Invite link", inviteUrl) },
                        icon = {
                            Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun NewChatActionRow(
    text: String,
    icon: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        color = IrisTheme.palette.panel,
        shape = RoundedCornerShape(14.dp),
        border = BorderStroke(1.dp, IrisTheme.palette.border),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 13.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                modifier = Modifier.width(22.dp),
                color = Color.Transparent,
                contentColor = IrisTheme.palette.accent,
            ) {
                icon()
            }
            Text(
                text = text,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
            )
            Icon(
                imageVector = IrisIcons.ChevronRight,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
            )
        }
    }
}
