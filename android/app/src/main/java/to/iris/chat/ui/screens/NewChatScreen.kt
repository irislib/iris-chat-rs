package to.iris.chat.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatInputShortcut
import to.iris.chat.rust.Screen
import to.iris.chat.rust.classifyChatInput
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisListSection
import to.iris.chat.ui.components.IrisMenuRow
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.irisTextFieldColors
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
    var qrDialogTab by remember { mutableStateOf<ProfileQrDialogTab?>(null) }
    val trimmedInput = peerInput.trim()
    val normalizedInput = normalizePeerInput(peerInput)
    val isValidPeer = normalizedInput.isNotBlank() && isValidPeerInput(normalizedInput)
    // Core decides what counts as an invite URL — same parser the chat
    // list search bar and any future share-link handler use, so we
    // can't drift on "is this an invite or an npub?".
    val looksLikeInviteLink =
        classifyChatInput(trimmedInput) is ChatInputShortcut.Invite

    val inviteUrl = appState.publicInvite?.url
    val canShareInvite = remember(context) { canShareText(context) }
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
            Column(
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "Share invite",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(horizontal = 2.dp),
                )

                if (inviteUrl != null) {
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        NewChatInviteActionButton(
                            text = "Copy",
                            onClick = { clipboard.setText("Invite", inviteUrl) },
                            modifier = Modifier.weight(1f).testTag("newChatInviteCopyButton"),
                            icon = { Icon(imageVector = IrisIcons.Copy, contentDescription = null) },
                        )
                        if (canShareInvite) {
                            NewChatInviteActionButton(
                                text = "Share",
                                onClick = { shareText(context, inviteUrl, "Share invite") },
                                modifier = Modifier.weight(1f).testTag("newChatInviteShareButton"),
                                icon = { Icon(imageVector = IrisIcons.Share, contentDescription = null) },
                            )
                        }
                        NewChatInviteActionButton(
                            text = "Show",
                            onClick = { qrDialogTab = ProfileQrDialogTab.Code },
                            modifier = Modifier.weight(1f).testTag("newChatInviteQrButton"),
                            icon = { Icon(imageVector = IrisIcons.ScanQr, contentDescription = null) },
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

            Column(
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "Start chat",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(horizontal = 2.dp),
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
                            text = "Paste invite or user id",
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
                    shape = RoundedCornerShape(10.dp),
                    colors = irisTextFieldColors(),
                )

                IrisSecondaryButton(
                    text = "Scan code",
                    onClick = { qrDialogTab = ProfileQrDialogTab.Scan },
                    modifier = Modifier.fillMaxWidth().testTag("newChatScanQrButton"),
                    icon = {
                        Icon(imageVector = IrisIcons.ScanQr, contentDescription = null)
                    },
                )
            }

            IrisListSection {
                IrisMenuRow(
                    title = "Create group",
                    icon = IrisIcons.NewGroup,
                    onClick = { appManager.pushScreen(Screen.NewGroup) },
                    modifier = Modifier.testTag("newChatNewGroupButton"),
                )
            }
        }
    }

    qrDialogTab?.let { initialTab ->
        ProfileQrDialog(
            qrBitmap = qrBitmap,
            displayName = "Invite code",
            canShare = canShareInvite && inviteUrl != null,
            initialTab = initialTab,
            qrTag = "newChatInviteQrCode",
            qrContentDescription = "Invite code",
            scanTag = "newChatQrScanner",
            copyContentDescription = "Copy invite",
            shareContentDescription = "Share invite",
            onDismiss = { qrDialogTab = null },
            onCopy = { inviteUrl?.let { clipboard.setText("Invite", it) } },
            onShare = { inviteUrl?.let { shareText(context, it, "Share invite") } },
            onScanned = { scanned ->
                if (scanned.isNotBlank()) {
                    handleNewChatInput(scanned)
                    qrDialogTab = null
                    null
                } else {
                    "That code was empty."
                }
            },
        )
    }
}

@Composable
private fun NewChatInviteActionButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: @Composable () -> Unit,
) {
    OutlinedButton(
        onClick = onClick,
        modifier = modifier.defaultMinSize(minHeight = 66.dp),
        shape = RoundedCornerShape(8.dp),
        border = BorderStroke(1.dp, IrisTheme.palette.border),
        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 10.dp),
        colors =
            ButtonDefaults.outlinedButtonColors(
                containerColor = MaterialTheme.colorScheme.background,
                contentColor = MaterialTheme.colorScheme.onSurface,
            ),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            icon()
            Text(
                text = text,
                style = MaterialTheme.typography.labelMedium,
                maxLines = 1,
                softWrap = false,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}
