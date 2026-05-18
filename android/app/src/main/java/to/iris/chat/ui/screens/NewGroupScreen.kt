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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisListSection
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun NewGroupScreen(
    appManager: AppManager,
    appState: AppState,
) {
    var step by remember { mutableStateOf(NewGroupStep.MEMBERS) }
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    val nameFocusRequester = remember { FocusRequester() }
    var name by remember { mutableStateOf("") }
    var memberInput by remember { mutableStateOf("") }
    var selectedOwners by remember { mutableStateOf(setOf<String>()) }
    var groupPhoto by remember { mutableStateOf<PickedAttachment?>(null) }
    var showGroupPhotoSourceMenu by remember { mutableStateOf(false) }
    var pendingCameraPhoto by remember { mutableStateOf<PendingCameraImage?>(null) }
    val localOwner = appState.account?.publicKeyHex
    val existingDirectChats =
        appState.chatList.filter { it.kind == ChatKind.DIRECT && it.chatId != localOwner }
    val filteredKnownChats = existingDirectChats.filterByQuery(memberInput)
    val canCreate = name.isNotBlank() && !appState.busy.creatingGroup
    val pictureFilePicker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri == null) {
                return@rememberLauncherForActivityResult
            }
            coroutineScope.launch {
                groupPhoto =
                    withContext(Dispatchers.IO) {
                        copyAttachmentToCache(context, uri)
                    }
            }
        }
    val pictureLibraryPicker =
        rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
            if (uri == null) {
                return@rememberLauncherForActivityResult
            }
            coroutineScope.launch {
                groupPhoto =
                    withContext(Dispatchers.IO) {
                        copyAttachmentToCache(context, uri)
                    }
            }
        }
    val pictureCameraPicker =
        rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { didTakePhoto ->
            val pending = pendingCameraPhoto
            pendingCameraPhoto = null
            if (didTakePhoto && pending != null) {
                groupPhoto = pending.attachment
            }
        }

    fun takeGroupPhoto() {
        val pending = createPendingCameraImage(context) ?: return
        pendingCameraPhoto = pending
        pictureCameraPicker.launch(pending.uri)
    }

    LaunchedEffect(step) {
        if (step == NewGroupStep.DETAILS) {
            runCatching { nameFocusRequester.requestFocus() }
        }
    }

    fun addOwner(ownerInput: String) {
        val normalized = normalizePeerInput(ownerInput)
        if (normalized.isBlank() || !isValidPeerInput(normalized)) {
            return
        }
        if (normalized == localOwner) {
            memberInput = ""
            return
        }
        selectedOwners = selectedOwners + normalized
        memberInput = ""
    }

    fun updateMemberInput(value: String) {
        val normalized = normalizePeerInput(value)
        if (normalized.isNotBlank() && isValidPeerInput(normalized)) {
            addOwner(normalized)
        } else {
            memberInput = value
        }
    }

    ScaffoldScreen(
        title = if (step == NewGroupStep.MEMBERS) "Select members" else "Group details",
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
            if (step == NewGroupStep.MEMBERS) {
                Column(
                    modifier = Modifier.testTag("newGroupMemberStep"),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    SelectedMemberChips(
                        selectedOwners = selectedOwners,
                        existingDirectChats = existingDirectChats,
                        localOwner = localOwner,
                        appState = appState,
                        onRemove = { owner -> selectedOwners = selectedOwners - owner },
                    )

                    TextField(
                        value = memberInput,
                        onValueChange = ::updateMemberInput,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("newGroupMemberInput"),
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
                }

                IrisPrimaryButton(
                    text = if (selectedOwners.isEmpty()) "Next" else "Next (${selectedOwners.size})",
                    onClick = { step = NewGroupStep.DETAILS },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("newGroupNextButton"),
                    icon = {
                        Icon(
                            imageVector = IrisIcons.NewGroup,
                            contentDescription = null,
                        )
                    },
                )

                if (filteredKnownChats.isNotEmpty()) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(
                            text = if (memberInput.isBlank()) "Known users" else "Search results",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.padding(horizontal = 2.dp),
                        )
                        IrisListSection {
                            filteredKnownChats.forEach { chat ->
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
                                        memberInput = ""
                                    },
                                )
                            }
                        }
                    }
                }
            } else {
                Column(
                    modifier = Modifier.testTag("newGroupDetailsStep"),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IrisAvatar(
                            label = name.ifBlank { "Group" },
                            size = 56.dp,
                            emphasize = true,
                        )
                        Column(
                            modifier = Modifier.weight(1f),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            IrisSecondaryButton(
                                text = if (groupPhoto == null) "Photo" else "Change photo",
                                onClick = { showGroupPhotoSourceMenu = true },
                                modifier = Modifier.testTag("newGroupPhotoButton"),
                            )
                            if (groupPhoto != null) {
                                Text(
                                    text = groupPhoto!!.filename,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = IrisTheme.palette.muted,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                IrisSecondaryButton(
                                    text = "Remove",
                                    onClick = { groupPhoto = null },
                                    modifier = Modifier.testTag("newGroupRemovePhotoButton"),
                                )
                            }
                        }
                    }

                    TextField(
                        value = name,
                        onValueChange = { name = it },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .focusRequester(nameFocusRequester)
                                .testTag("newGroupNameInput"),
                        placeholder = {
                            Text(
                                text = "Group name",
                                color = IrisTheme.palette.muted,
                            )
                        },
                        singleLine = true,
                        shape = RoundedCornerShape(10.dp),
                        colors = irisTextFieldColors(),
                    )

                    SelectedMemberChips(
                        selectedOwners = selectedOwners,
                        existingDirectChats = existingDirectChats,
                        localOwner = localOwner,
                        appState = appState,
                        onRemove = { owner -> selectedOwners = selectedOwners - owner },
                    )
                }

                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    IrisSecondaryButton(
                        text = "Back",
                        onClick = { step = NewGroupStep.MEMBERS },
                        modifier = Modifier.weight(1f),
                    )
                    IrisPrimaryButton(
                        text = if (appState.busy.creatingGroup) "Creating…" else "Create group",
                        onClick = {
                            appManager.createGroup(
                                name = name,
                                memberInputs = selectedOwners.toList(),
                                pictureFilePath = groupPhoto?.path,
                                pictureFilename = groupPhoto?.filename,
                            )
                        },
                        enabled = canCreate,
                        modifier =
                            Modifier
                                .weight(1f)
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
}

private enum class NewGroupStep {
    MEMBERS,
    DETAILS,
}

@Composable
private fun SelectedMemberChips(
    selectedOwners: Set<String>,
    existingDirectChats: List<ChatThreadSnapshot>,
    localOwner: String?,
    appState: AppState,
    onRemove: (String) -> Unit,
) {
    if (selectedOwners.isNotEmpty()) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            selectedOwners.toList().sorted().forEach { owner ->
                val presentation =
                    ownerPresentation(
                        owner = owner,
                        existingDirectChats = existingDirectChats,
                        localOwnerHex = localOwner,
                        localOwnerDisplayName = appState.account?.displayName.orEmpty(),
                        localOwnerNpub = appState.account?.npub,
                    )
                MemberChip(
                    title = presentation.primary,
                    subtitle = presentation.secondary,
                    onRemove = { onRemove(owner) },
                )
            }
        }
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
                onBack = { appManager.navigateBack() },
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
    val haptics = rememberIrisHapticFeedback()
    val removeInteractionSource = remember { MutableInteractionSource() }
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
                modifier =
                    Modifier
                        .testTag("memberChipRemove")
                        .clickable(
                            interactionSource = removeInteractionSource,
                            indication = null,
                        ) {
                            haptics.press()
                            onRemove()
                        },
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
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                ) {
                    haptics.press()
                    onClick()
                }
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
            tint = if (selected) MaterialTheme.colorScheme.onSurface else IrisTheme.palette.muted,
            modifier = Modifier.size(20.dp),
        )
    }
}
