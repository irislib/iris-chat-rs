package social.innode.ndr.demo.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
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
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.rust.Screen
import social.innode.ndr.demo.rust.isValidPeerInput
import social.innode.ndr.demo.rust.normalizePeerInput
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisSecondaryButton
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

@Composable
fun NewChatScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val clipboard = rememberIrisClipboard()
    val focusManager = LocalFocusManager.current
    var peerInput by remember { mutableStateOf("") }
    var submittedPeerInput by remember { mutableStateOf<String?>(null) }
    var showScanner by remember { mutableStateOf(false) }
    val normalizedInput = normalizePeerInput(peerInput)
    val isValidPeer = normalizedInput.isNotBlank() && isValidPeerInput(normalizedInput)

    LaunchedEffect(isValidPeer, normalizedInput) {
        if (isValidPeer && submittedPeerInput != normalizedInput) {
            submittedPeerInput = normalizedInput
            focusManager.clearFocus()
            appManager.createChat(normalizedInput)
        }
    }

    fun handleNewChatInput(raw: String) {
        val normalized = normalizePeerInput(raw)
        if (normalized.isNotBlank() && isValidPeerInput(normalized)) {
            peerInput = normalized
            submittedPeerInput = normalized
            appManager.createChat(normalized)
            return
        }

        val trimmed = raw.trim()
        if (trimmed.isNotBlank()) {
            peerInput = trimmed
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
                    .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("newChatPrimaryCard"),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "User ID",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
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
                            text = "User ID, hex, or link",
                            color = IrisTheme.palette.muted,
                        )
                    },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions =
                        KeyboardActions(
                            onDone = {
                                focusManager.clearFocus()
                                if (isValidPeer) {
                                    appManager.createChat(normalizedInput)
                                }
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

                if (peerInput.isNotBlank() && !isValidPeer) {
                    Text(
                        text = "Invalid user ID.",
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }

                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    IrisSecondaryButton(
                        text = "Paste",
                        onClick = {
                            clipboard.getText { text ->
                                handleNewChatInput(text)
                            }
                        },
                        modifier = Modifier.testTag("newChatPasteButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Copy,
                                contentDescription = null,
                            )
                        },
                    )
                    IrisSecondaryButton(
                        text = "Scan QR",
                        onClick = { showScanner = true },
                        modifier = Modifier.testTag("newChatScanQrButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.ScanQr,
                                contentDescription = null,
                            )
                        },
                    )
                }

            }

            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                NewChatActionRow(
                    text = "Join with invite",
                    icon = { Icon(imageVector = IrisIcons.NewChat, contentDescription = null) },
                    modifier = Modifier.testTag("newChatJoinInviteButton"),
                    onClick = { appManager.pushScreen(Screen.JoinInvite) },
                )
                NewChatActionRow(
                    text = "Create invite",
                    icon = { Icon(imageVector = IrisIcons.Share, contentDescription = null) },
                    modifier = Modifier.testTag("newChatCreateInviteButton"),
                    onClick = { appManager.pushScreen(Screen.CreateInvite) },
                )
                NewChatActionRow(
                    text = "New group",
                    icon = { Icon(imageVector = IrisIcons.NewGroup, contentDescription = null) },
                    modifier = Modifier.testTag("newChatNewGroupButton"),
                    onClick = { appManager.pushScreen(Screen.NewGroup) },
                )
            }
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
