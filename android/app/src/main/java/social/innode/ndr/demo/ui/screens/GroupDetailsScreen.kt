package social.innode.ndr.demo.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.rust.GroupMemberSnapshot
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
fun GroupDetailsScreen(
    appManager: AppManager,
    appState: AppState,
    groupId: String,
) {
    val details = appState.groupDetails?.takeIf { it.groupId == groupId }
    val clipboard = rememberIrisClipboard()
    val context = androidx.compose.ui.platform.LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    var renameValue by remember(groupId, details?.name) { mutableStateOf(details?.name.orEmpty()) }
    var memberInput by remember(groupId) { mutableStateOf("") }
    var showScanner by remember { mutableStateOf(false) }
    val normalizedInput = normalizePeerInput(memberInput)
    val pictureData by rememberNhashImageData(appManager, details?.pictureUrl)
    val pictureUrl =
        details
            ?.pictureUrl
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
    val picturePicker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri == null) {
                return@rememberLauncherForActivityResult
            }
            coroutineScope.launch {
                val picked =
                    withContext(Dispatchers.IO) {
                        copyAttachmentToCache(context, uri)
                    }
                if (picked != null) {
                    appManager.updateGroupPicture(groupId, picked.path, picked.filename)
                }
            }
        }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = details?.name ?: "Group details",
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(appState.router.screenStack.dropLast(1)),
                    )
                },
            )
        },
    ) { padding ->
        if (details == null) {
            Surface(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding),
                color = MaterialTheme.colorScheme.background,
            ) {
                Text(
                    text = "Loading group…",
                    modifier = Modifier.padding(24.dp),
                )
            }
            return@Scaffold
        }

        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 14.dp)
                    .testTag("groupDetailsScreen"),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IrisSectionCard {
                val creatorPrimary = primaryDisplayName(details.createdByDisplayName, details.createdByNpub)
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IrisAvatar(
                        label = details.name,
                        size = 62.dp,
                        emphasize = true,
                        imageUrl = pictureUrl,
                        imageData = pictureData,
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = details.name,
                            style = MaterialTheme.typography.headlineSmall,
                        )
                        Text(
                            text = "${details.members.size} members · revision ${details.revision}",
                            style = MaterialTheme.typography.bodyMedium,
                            color = IrisTheme.palette.muted,
                        )
                    }
                }
                Text(
                    text = "Created by $creatorPrimary",
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                )
                if (details.canManage) {
                    IrisSecondaryButton(
                        text = if (appState.busy.uploadingAttachment) "Uploading…" else "Change photo",
                        onClick = { picturePicker.launch(arrayOf("image/*")) },
                        enabled = !appState.busy.uploadingAttachment,
                        modifier = Modifier.testTag("groupDetailsChangePictureButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Image,
                                contentDescription = null,
                            )
                        },
                    )
                }
            }

            run {
                val groupChatId = "group:$groupId"
                val groupChat = appState.currentChat?.takeIf { it.chatId == groupChatId }
                DisappearingMessagesCard(
                    currentTtlSeconds = groupChat?.messageTtlSeconds,
                    onSelect = { ttlSeconds ->
                        appManager.dispatch(AppAction.SetChatMessageTtl(groupChatId, ttlSeconds))
                    },
                )
            }

            IrisSectionCard {
                Text(
                    text = "Members",
                    style = MaterialTheme.typography.titleMedium,
                )
                details.members.forEach { member ->
                    val primary = primaryDisplayName(member.displayName, member.npub)
                    val roles = member.roleLabels()
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.Top,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        IrisAvatar(
                            label = primary,
                            emphasize = member.isLocalOwner,
                            size = 38.dp,
                        )
                        Column(
                            modifier =
                                Modifier
                                    .weight(1f)
                                    .padding(start = 12.dp),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                text = primary,
                                style = MaterialTheme.typography.bodyMedium,
                                fontWeight = FontWeight.SemiBold,
                            )
                            if (roles.isNotEmpty()) {
                                Text(
                                    text = roles.joinToString(" · "),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = IrisTheme.palette.muted,
                                )
                            }
                        }
                        if (details.canManage && !member.isLocalOwner) {
                            Column(
                                horizontalAlignment = Alignment.End,
                                verticalArrangement = Arrangement.spacedBy(6.dp),
                            ) {
                                Text(
                                    text = if (member.isAdmin) "Dismiss admin" else "Make admin",
                                    modifier =
                                        Modifier
                                            .testTag("groupDetailsToggleAdmin-${member.ownerPubkeyHex.take(12)}")
                                            .clickable {
                                                appManager.setGroupAdmin(
                                                    groupId,
                                                    member.ownerPubkeyHex,
                                                    !member.isAdmin,
                                                )
                                            },
                                    color = IrisTheme.palette.accent,
                                    style = MaterialTheme.typography.labelLarge,
                                )
                                Text(
                                    text = "Remove",
                                    modifier =
                                        Modifier
                                            .testTag("groupDetailsRemoveMember-${member.ownerPubkeyHex.take(12)}")
                                            .clickable {
                                                appManager.removeGroupMember(groupId, member.ownerPubkeyHex)
                                            },
                                    color = MaterialTheme.colorScheme.error,
                                    style = MaterialTheme.typography.labelLarge,
                                )
                            }
                        }
                    }
                }
            }

            if (details.canManage) {
                IrisSectionCard {
                    Text(
                        text = "Rename group",
                        style = MaterialTheme.typography.titleMedium,
                    )
                    TextField(
                        value = renameValue,
                        onValueChange = { renameValue = it },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("groupDetailsNameInput"),
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
                    IrisPrimaryButton(
                        text = if (appState.busy.updatingGroup) "Saving…" else "Save name",
                        onClick = { appManager.updateGroupName(groupId, renameValue) },
                        enabled = renameValue.isNotBlank() && !appState.busy.updatingGroup,
                        modifier = Modifier.testTag("groupDetailsRenameButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.Edit,
                                contentDescription = null,
                            )
                        },
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
                                .testTag("groupDetailsAddMemberInput"),
                        placeholder = {
                            Text(
                                text = "User ID or nostr:...",
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
                            modifier = Modifier.testTag("groupDetailsScanQrButton"),
                            icon = {
                                Icon(
                                    imageVector = IrisIcons.ScanQr,
                                    contentDescription = null,
                                )
                            },
                        )
                    }
                    IrisPrimaryButton(
                        text = if (appState.busy.updatingGroup) "Adding…" else "Add member",
                        onClick = {
                            appManager.addGroupMembers(groupId, listOf(normalizedInput))
                            memberInput = ""
                        },
                        enabled = normalizedInput.isNotBlank() && isValidPeerInput(normalizedInput) && !appState.busy.updatingGroup,
                        modifier = Modifier.testTag("groupDetailsAddMembersButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.NewGroup,
                                contentDescription = null,
                            )
                        },
                    )
                }
            }
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
                    memberInput = normalized
                    showScanner = false
                    null
                }
            },
        )
    }
}

private fun primaryDisplayName(
    displayName: String,
    fallback: String,
): String =
    displayName.trim().ifEmpty { fallback.trim() }

private fun GroupMemberSnapshot.roleLabels(): List<String> =
    buildList {
        if (isCreator) add("Creator")
        if (isAdmin) add("Admin")
        if (isLocalOwner) add("You")
    }
