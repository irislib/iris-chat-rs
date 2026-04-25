package social.innode.ndr.demo.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.rust.ChatKind
import social.innode.ndr.demo.rust.ChatThreadSnapshot
import social.innode.ndr.demo.rust.isValidPeerInput
import social.innode.ndr.demo.rust.normalizePeerInput
import social.innode.ndr.demo.ui.components.IrisAvatar
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisPrimaryButton
import social.innode.ndr.demo.ui.components.IrisSectionCard
import social.innode.ndr.demo.ui.components.IrisSecondaryButton
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

@Composable
fun NewGroupScreen(
    appManager: AppManager,
    appState: AppState,
) {
    val clipboard = rememberIrisClipboard()
    var name by remember { mutableStateOf("") }
    var memberInput by remember { mutableStateOf("") }
    var showScanner by remember { mutableStateOf(false) }
    var selectedOwners by remember { mutableStateOf(setOf<String>()) }
    val localOwner = appState.account?.publicKeyHex
    val normalizedInput = normalizePeerInput(memberInput)
    val existingDirectChats =
        appState.chatList.filter { it.kind == ChatKind.DIRECT && it.chatId != localOwner }
    val canCreate = name.isNotBlank() && selectedOwners.isNotEmpty() && !appState.busy.creatingGroup

    fun addOwner(ownerInput: String) {
        val normalized = normalizePeerInput(ownerInput)
        if (normalized.isBlank() || !isValidPeerInput(normalized)) {
            return
        }
        if (normalized == localOwner) {
            return
        }
        selectedOwners = selectedOwners + normalized
        memberInput = ""
    }

    ScaffoldScreen(
        title = "New group",
        appManager = appManager,
        appState = appState,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IrisSectionCard {
                Text(
                    text = "Group name",
                    style = MaterialTheme.typography.titleMedium,
                )
                TextField(
                    value = name,
                    onValueChange = { name = it },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("newGroupNameInput"),
                    placeholder = {
                        Text(
                            text = "Weekend plans",
                            color = IrisTheme.palette.muted,
                        )
                    },
                    singleLine = true,
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
            }

            IrisSectionCard {
                Text(
                    text = "Add members",
                    style = MaterialTheme.typography.titleMedium,
                )
                TextField(
                    value = memberInput,
                    onValueChange = { memberInput = it },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("newGroupMemberInput"),
                    placeholder = {
                        Text(
                            text = "User ID, hex, or nostr:...",
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
                            clipboard.getText { text ->
                                memberInput = normalizePeerInput(text)
                            }
                        },
                        modifier = Modifier.testTag("newGroupPasteButton"),
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
                        modifier = Modifier.testTag("newGroupScanQrButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.ScanQr,
                                contentDescription = null,
                            )
                        },
                    )
                    IrisPrimaryButton(
                        text = "Add",
                        onClick = { addOwner(normalizedInput) },
                        enabled = normalizedInput.isNotBlank() && isValidPeerInput(normalizedInput),
                        modifier = Modifier.testTag("newGroupAddMemberButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.NewGroup,
                                contentDescription = null,
                            )
                        },
                    )
                }

                if (selectedOwners.isNotEmpty()) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        selectedOwners.toList().sorted().forEach { owner ->
                            val presentation = ownerPresentation(
                                owner = owner,
                                existingDirectChats = existingDirectChats,
                                localOwnerHex = localOwner,
                                localOwnerDisplayName = appState.account?.displayName.orEmpty(),
                                localOwnerNpub = appState.account?.npub,
                            )
                            MemberChip(
                                title = presentation.primary,
                                subtitle = presentation.secondary,
                                onRemove = { selectedOwners = selectedOwners - owner },
                            )
                        }
                    }
                }
            }

            if (existingDirectChats.isNotEmpty()) {
                IrisSectionCard {
                    Text(
                        text = "Existing chats",
                        style = MaterialTheme.typography.titleMedium,
                    )
                    existingDirectChats.forEach { chat ->
                        val selected = chat.chatId in selectedOwners
                        val presentation = ownerPresentation(
                            owner = chat.chatId,
                            existingDirectChats = existingDirectChats,
                            localOwnerHex = localOwner,
                            localOwnerDisplayName = appState.account?.displayName.orEmpty(),
                            localOwnerNpub = appState.account?.npub,
                        )
                        ExistingMemberRow(
                            title = presentation.primary,
                            subtitle = presentation.secondary,
                            selected = selected,
                            onClick = {
                                selectedOwners =
                                    if (selected) {
                                        selectedOwners - chat.chatId
                                    } else {
                                        selectedOwners + chat.chatId
                                    }
                            },
                        )
                    }
                }
            }

            IrisPrimaryButton(
                text = if (appState.busy.creatingGroup) "Creating…" else "Create group",
                onClick = { appManager.createGroup(name, selectedOwners.toList()) },
                enabled = canCreate,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("newGroupCreateButton"),
                icon = {
                    Icon(
                        imageVector = IrisIcons.NewGroup,
                        contentDescription = null,
                    )
                },
            )
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
                    addOwner(normalized)
                    showScanner = false
                    null
                }
            },
        )
    }
}

private data class OwnerPresentation(
    val primary: String,
    val secondary: String?,
)

private fun ownerPresentation(
    owner: String,
    existingDirectChats: List<ChatThreadSnapshot>,
    localOwnerHex: String?,
    localOwnerDisplayName: String,
    localOwnerNpub: String?,
): OwnerPresentation {
    existingDirectChats.firstOrNull { sameOwner(owner, hex = it.chatId, npub = it.subtitle) }?.let { chat ->
        val primary = primaryDisplayName(chat.displayName, normalizePeerInput(owner))
        return OwnerPresentation(primary, null)
    }

    if (localOwnerHex != null && sameOwner(owner, hex = localOwnerHex, npub = localOwnerNpub)) {
        val primary = primaryDisplayName(localOwnerDisplayName, localOwnerNpub ?: localOwnerHex)
        return OwnerPresentation(primary, null)
    }

    return OwnerPresentation(fallbackProfileNameForIdentity(normalizePeerInput(owner)), null)
}

private fun sameOwner(
    owner: String,
    hex: String?,
    npub: String?,
): Boolean {
    val rawOwner = owner.trim().lowercase()
    val normalizedOwner = normalizePeerInput(owner).trim().lowercase()
    return listOfNotNull(hex, npub)
        .map { it.trim().lowercase() }
        .any { it == rawOwner || it == normalizedOwner }
}

private fun primaryDisplayName(
    displayName: String,
    fallback: String,
): String =
    displayName.trim().ifEmpty { fallbackProfileNameForIdentity(fallback) }

private fun fallbackProfileNameForIdentity(identity: String): String {
    val adjectives =
        listOf(
            "Amber",
            "Bright",
            "Calm",
            "Clear",
            "Golden",
            "Lunar",
            "Nova",
            "Quiet",
            "Silver",
            "Solar",
            "Velvet",
            "Wild",
        )
    val nouns =
        listOf(
            "Aurora",
            "Comet",
            "Echo",
            "Falcon",
            "Harbor",
            "Listener",
            "Otter",
            "Raven",
            "Signal",
            "Sparrow",
            "Tide",
            "Voyager",
        )
    val trimmed = identity.trim()
    if (trimmed.isEmpty()) {
        return "Quiet Listener"
    }
    val hash = trimmed.fold(0) { acc, char -> acc * 31 + char.code }
    val positiveHash = hash and Int.MAX_VALUE
    return "${adjectives[positiveHash % adjectives.size]} ${nouns[(positiveHash / adjectives.size) % nouns.size]}"
}

@Composable
private fun ScaffoldScreen(
    title: String,
    appManager: AppManager,
    appState: AppState,
    content: @Composable () -> Unit,
) {
    androidx.compose.material3.Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = title,
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(appState.router.screenStack.dropLast(1)),
                    )
                },
            )
        },
    ) { padding ->
        Surface(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
            color = MaterialTheme.colorScheme.background,
        ) {
            content()
        }
    }
}

@Composable
private fun MemberChip(
    title: String,
    subtitle: String?,
    onRemove: () -> Unit,
) {
    Surface(
        color = IrisTheme.palette.panelAlt,
        shape = RoundedCornerShape(14.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.labelMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (subtitle != null) {
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.labelSmall,
                        color = IrisTheme.palette.muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Text(
                text = "Remove",
                modifier = Modifier.testTag("memberChipRemove").clickable(onClick = onRemove),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}

@Composable
private fun ExistingMemberRow(
    title: String,
    subtitle: String?,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IrisAvatar(label = title, emphasize = selected, size = 38.dp)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = title,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (subtitle != null) {
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Icon(
            imageVector = if (selected) IrisIcons.Devices else IrisIcons.NewChat,
            contentDescription = null,
            tint = if (selected) IrisTheme.palette.accent else IrisTheme.palette.muted,
            modifier = Modifier.size(20.dp),
        )
    }
}
