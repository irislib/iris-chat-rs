package to.iris.chat.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.GroupMemberSnapshot
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisListSection
import to.iris.chat.ui.components.IrisMenuRow
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

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
    val haptics = rememberIrisHapticFeedback()
    var renameValue by remember(groupId, details?.name) { mutableStateOf(details?.name.orEmpty()) }
    var memberInput by remember(groupId) { mutableStateOf("") }
    var showScanner by remember { mutableStateOf(false) }
    var showGroupPhotoSourceMenu by remember { mutableStateOf(false) }
    var pendingCameraPhoto by remember { mutableStateOf<PendingCameraImage?>(null) }
    val normalizedInput = normalizePeerInput(memberInput)
    val localOwnerHex = appState.account?.publicKeyHex
    val existingMemberHexes =
        details?.members?.map { it.ownerPubkeyHex }?.toSet().orEmpty()
    val knownUsers =
        appState.chatList
            .filter { chat ->
                chat.kind == to.iris.chat.rust.ChatKind.DIRECT &&
                    chat.chatId != localOwnerHex &&
                    chat.chatId !in existingMemberHexes
            }
            .filterByQuery(memberInput)
    val pictureData by rememberNhashImageData(appManager, details?.pictureUrl)
    val pictureUrl =
        details
            ?.pictureUrl
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
    val pictureFilePicker =
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
    val pictureLibraryPicker =
        rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
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
    val pictureCameraPicker =
        rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { didTakePhoto ->
            val pending = pendingCameraPhoto
            pendingCameraPhoto = null
            if (didTakePhoto && pending != null) {
                appManager.updateGroupPicture(groupId, pending.attachment.path, pending.attachment.filename)
            }
        }

    fun takeGroupPhoto() {
        val pending = createPendingCameraImage(context) ?: return
        pendingCameraPhoto = pending
        pictureCameraPicker.launch(pending.uri)
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = details?.name ?: "Group details",
                onBack = { appManager.navigateBack() },
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
            Column(
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
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
                        onClick = { showGroupPhotoSourceMenu = true },
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

            run {
                val groupChatId = "group:$groupId"
                IrisListSection {
                    IrisMenuRow(
                        title = if (details.isMuted) "Unmute chat" else "Mute chat",
                        icon =
                            if (details.isMuted) {
                                IrisIcons.Notifications
                            } else {
                                IrisIcons.NotificationsOff
                            },
                        onClick = {
                            appManager.dispatch(
                                AppAction.SetChatMuted(groupChatId, !details.isMuted),
                            )
                        },
                        modifier = Modifier.testTag("groupDetailsMuteButton"),
                    )
                }
            }

            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = "Members",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(horizontal = 2.dp),
                )
                IrisListSection {
                    details.members.forEach { member ->
                        val primary = primaryDisplayName(member.displayName, member.npub)
                        val roles = member.roleLabels()
                        val openProfileInteractionSource =
                            remember(member.ownerPubkeyHex) { MutableInteractionSource() }
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .then(
                                    if (member.isLocalOwner) {
                                        Modifier
                                    } else {
                                        Modifier.clickable(
                                            interactionSource = openProfileInteractionSource,
                                            indication = null,
                                        ) {
                                            haptics.press()
                                            appManager.createChat(member.ownerPubkeyHex)
                                        }
                                    },
                                )
                                .padding(16.dp),
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
                                val toggleAdminInteractionSource =
                                    remember(member.ownerPubkeyHex, member.isAdmin) { MutableInteractionSource() }
                                val removeInteractionSource =
                                    remember(member.ownerPubkeyHex) { MutableInteractionSource() }
                                Column(
                                    horizontalAlignment = Alignment.End,
                                    verticalArrangement = Arrangement.spacedBy(6.dp),
                                ) {
                                    Text(
                                        text = if (member.isAdmin) "Dismiss admin" else "Make admin",
                                        modifier =
                                            Modifier
                                                .testTag("groupDetailsToggleAdmin-${member.ownerPubkeyHex.take(12)}")
                                                .clickable(
                                                    interactionSource = toggleAdminInteractionSource,
                                                    indication = null,
                                                ) {
                                                    haptics.press()
                                                    appManager.setGroupAdmin(
                                                        groupId,
                                                        member.ownerPubkeyHex,
                                                        !member.isAdmin,
                                                    )
                                                },
                                        color = MaterialTheme.colorScheme.onBackground,
                                        fontWeight = FontWeight.SemiBold,
                                        style = MaterialTheme.typography.labelLarge,
                                    )
                                    Text(
                                        text = "Remove",
                                        modifier =
                                            Modifier
                                                .testTag("groupDetailsRemoveMember-${member.ownerPubkeyHex.take(12)}")
                                                .clickable(
                                                    interactionSource = removeInteractionSource,
                                                    indication = null,
                                                ) {
                                                    haptics.confirm()
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
            }

            if (details.canManage) {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Rename group",
                        style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.padding(horizontal = 2.dp),
                    )
                    TextField(
                        value = renameValue,
                        onValueChange = { renameValue = it },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("groupDetailsNameInput"),
                        singleLine = true,
                        shape = RoundedCornerShape(10.dp),
                        colors = irisTextFieldColors(),
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

                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Add members",
                        style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.padding(horizontal = 2.dp),
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
                                text = "Search or paste user ID",
                                color = IrisTheme.palette.muted,
                            )
                        },
                        singleLine = true,
                        shape = RoundedCornerShape(10.dp),
                        colors = irisTextFieldColors(),
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
                            text = "Scan code",
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

                if (knownUsers.isNotEmpty()) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(
                            text = if (memberInput.isBlank()) "Known users" else "Search results",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.padding(horizontal = 2.dp),
                        )
                        IrisListSection {
                            knownUsers.forEach { chat ->
                                val title =
                                    chat.displayName.trim().ifEmpty {
                                        chat.subtitle.orEmpty().ifEmpty { chat.chatId }
                                    }
                                val subtitle =
                                    chat.subtitle?.takeIf { it.isNotBlank() && it != title }
                                val interactionSource = remember(chat.chatId) { MutableInteractionSource() }
                                Row(
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .clickable(
                                                interactionSource = interactionSource,
                                                indication = null,
                                            ) {
                                                haptics.press()
                                                appManager.addGroupMembers(groupId, listOf(chat.chatId))
                                                memberInput = ""
                                            }
                                            .padding(16.dp)
                                            .testTag("groupDetailsKnownUser-${chat.chatId.take(12)}"),
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    IrisAvatar(label = title, size = 38.dp)
                                    Column(
                                        modifier = Modifier.weight(1f),
                                        verticalArrangement = Arrangement.spacedBy(2.dp),
                                    ) {
                                        Text(
                                            text = title,
                                            style = MaterialTheme.typography.bodyMedium,
                                            fontWeight = FontWeight.SemiBold,
                                        )
                                        if (subtitle != null) {
                                            Text(
                                                text = subtitle,
                                                style = MaterialTheme.typography.bodySmall,
                                                color = IrisTheme.palette.muted,
                                            )
                                        }
                                    }
                                    Icon(
                                        imageVector = IrisIcons.NewGroup,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.onSurface,
                                    )
                                }
                            }
                        }
                    }
                }
            }

            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = "Removes this group from your chat list and forgets local messages.",
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                    modifier = Modifier.padding(horizontal = 2.dp),
                )
                IrisListSection {
                    IrisMenuRow(
                        title = "Delete chat",
                        icon = IrisIcons.DeleteForever,
                        onClick = {
                            appManager.dispatch(AppAction.DeleteChat("group:$groupId"))
                        },
                        modifier = Modifier.testTag("groupDetailsDeleteChatButton"),
                    )
                }
            }
        }
    }

    if (showGroupPhotoSourceMenu) {
        AlertDialog(
            onDismissRequest = { showGroupPhotoSourceMenu = false },
            confirmButton = {},
            title = { Text("Choose a group photo") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    TextButton(
                        onClick = {
                            showGroupPhotoSourceMenu = false
                            takeGroupPhoto()
                        },
                    ) {
                        Text("Take Photo")
                    }
                    TextButton(
                        onClick = {
                            showGroupPhotoSourceMenu = false
                            pictureLibraryPicker.launch(
                                PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly),
                            )
                        },
                    ) {
                        Text("Photo Library")
                    }
                    TextButton(
                        onClick = {
                            showGroupPhotoSourceMenu = false
                            pictureFilePicker.launch(arrayOf("image/*"))
                        },
                    ) {
                        Text("Files")
                    }
                }
            },
            dismissButton = {
                TextButton(onClick = { showGroupPhotoSourceMenu = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    if (showScanner) {
        QrScannerDialog(
            onDismiss = { showScanner = false },
            onScanned = { scanned ->
                val normalized = normalizePeerInput(scanned)
                if (!isValidPeerInput(normalized)) {
                    "That code or user ID is not valid."
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
