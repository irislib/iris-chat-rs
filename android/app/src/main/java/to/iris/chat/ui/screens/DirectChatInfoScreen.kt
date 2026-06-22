package to.iris.chat.ui.screens

import androidx.activity.compose.BackHandler
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
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Flag
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.PeerProfileDebugSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.rust.peerInputToNpub
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisDivider
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisInlineAction
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun DirectChatInfoScreen(
    appManager: AppManager,
    chatId: String,
    onBack: () -> Unit,
    showMessageAction: Boolean = false,
    onMessage: () -> Unit = { appManager.openChat(chatId) },
) {
    val currentChat by appManager.currentChat.collectAsStateWithLifecycle()
    val preferences by appManager.preferences.collectAsStateWithLifecycle()
    val chat = currentChat?.takeIf { it.chatId == chatId } ?: return
    val context = LocalContext.current
    val clipboard = rememberIrisClipboard()
    val avatarBytes by rememberNhashImageData(appManager, chat.pictureUrl)
    var advancedOpen by remember(chatId) { mutableStateOf(false) }
    var profileDebug by remember(chatId) { mutableStateOf<PeerProfileDebugSnapshot?>(null) }
    var commonGroups by remember(chatId) { mutableStateOf<List<ChatThreadSnapshot>>(emptyList()) }
    var nicknameDraft by remember(chatId) { mutableStateOf(chat.nickname.orEmpty()) }
    var editingNickname by remember(chatId) { mutableStateOf(false) }
    var showBlockDialog by remember(chatId) { mutableStateOf(false) }
    var showReportDialog by remember(chatId) { mutableStateOf(false) }
    val blocked = isUserBlocked(preferences, chatId)
    val proxiedAvatarUrl =
        chat.pictureUrl
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
            ?.let { url ->
                proxiedImageUrl(
                    originalSrc = url,
                    preferences = preferences,
                    width = 192u,
                    height = 192u,
                    square = true,
                )
            }

    LaunchedEffect(advancedOpen, chatId) {
        if (advancedOpen && profileDebug == null) {
            profileDebug = appManager.peerProfileDebug(chatId)
        }
    }
    LaunchedEffect(chatId) {
        commonGroups = appManager.mutualGroups(chatId)
    }
    LaunchedEffect(chatId, chat.nickname) {
        nicknameDraft = chat.nickname.orEmpty()
    }

    BackHandler {
        onBack()
    }

    Surface(
        modifier =
            Modifier
                .fillMaxSize()
                .testTag("directChatInfoScreen"),
        color = MaterialTheme.colorScheme.background,
    ) {
        Scaffold(
            containerColor = MaterialTheme.colorScheme.background,
            topBar = {
                IrisTopBar(
                    title = chat.displayName,
                    onBack = onBack,
                )
            },
        ) { padding ->
            // Chat info exposes the peer's hex pubkey + npub +
            // their relay debug counters — make them all
            // long-press-to-copy. SelectionContainer is inert for
            // buttons, IconButtons, and the avatar; only Text
            // children pick up selection.
            SelectionContainer(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IrisAvatar(
                            label = chat.displayName,
                            size = 72.dp,
                            emphasize = true,
                            imageUrl = proxiedAvatarUrl,
                            imageData = avatarBytes,
                        )
                        Column(
                            modifier = Modifier.weight(1f),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                text = chat.displayName,
                                style = MaterialTheme.typography.headlineSmall,
                                fontWeight = FontWeight.Bold,
                            )
                            chat.subtitle?.takeIf { it.isNotBlank() }?.let { subtitle ->
                                Text(
                                    text = subtitle,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = IrisTheme.palette.muted,
                                )
                            }
                        }
                    }
                    ProfileAboutCard(
                        about = chat.about,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    ContactNicknameCard(
                        chat = chat,
                        nicknameDraft = nicknameDraft,
                        onNicknameChange = { nicknameDraft = it },
                        onSave = {
                            appManager.dispatch(
                                AppAction.SetContactNickname(chatId, nicknameDraft),
                            )
                            editingNickname = false
                        },
                        onRemove = {
                            nicknameDraft = ""
                            editingNickname = false
                            appManager.dispatch(AppAction.SetContactNickname(chatId, ""))
                        },
                        editing = editingNickname,
                        onToggleEditing = { editingNickname = !editingNickname },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    if (showMessageAction) {
                        IrisInlineAction(
                            text = "Message",
                            onClick = onMessage,
                            modifier = Modifier.testTag("directChatMessageButton"),
                        ) {
                            Icon(imageVector = IrisIcons.NewChat, contentDescription = null)
                        }
                    }
                    if (commonGroups.isNotEmpty()) {
                        CommonGroupsCard(
                            appManager = appManager,
                            groups = commonGroups,
                            onBack = onBack,
                        )
                    }
                    IrisInlineAction(
                        text = "Copy user ID",
                        onClick = { clipboard.setText("User ID", peerInputToNpub(chatId)) },
                        modifier = Modifier.testTag("directChatCopyUserIdButton"),
                    ) {
                        Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                    }
                    IrisInlineAction(
                        text = if (chat.isMuted) "Unmute chat" else "Mute chat",
                        onClick = {
                            appManager.dispatch(AppAction.SetChatMuted(chatId, !chat.isMuted))
                        },
                        modifier = Modifier.testTag("directChatMuteButton"),
                    ) {
                        Icon(
                            imageVector =
                                if (chat.isMuted) {
                                    IrisIcons.Notifications
                                } else {
                                    IrisIcons.NotificationsOff
                                },
                            contentDescription = null,
                        )
                    }
                    DisappearingMessagesCard(
                        currentTtlSeconds = chat.messageTtlSeconds,
                        onSelect = { ttlSeconds ->
                            appManager.dispatch(AppAction.SetChatMessageTtl(chatId, ttlSeconds))
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    DirectChatAdvancedCard(
                        debug = profileDebug,
                        expanded = advancedOpen,
                        onToggle = { advancedOpen = !advancedOpen },
                        modifier = Modifier.testTag("directChatAdvancedCard"),
                    )
                    IrisInlineAction(
                        text = if (blocked) "Unblock user" else "Block user",
                        onClick = {
                            if (blocked) {
                                appManager.dispatch(AppAction.SetUserBlocked(chatId, false))
                            } else {
                                showBlockDialog = true
                            }
                        },
                        modifier = Modifier.testTag("directChatBlockButton"),
                    ) {
                        Icon(
                            imageVector = if (blocked) IrisIcons.Check else Icons.Rounded.Block,
                            contentDescription = null,
                            tint = if (blocked) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.error,
                        )
                    }
                    IrisInlineAction(
                        text = "Report user",
                        onClick = { showReportDialog = true },
                        modifier = Modifier.testTag("directChatReportButton"),
                    ) {
                        Icon(
                            imageVector = Icons.Rounded.Flag,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }
                    IrisInlineAction(
                        text = "Delete chat",
                        onClick = {
                            appManager.dispatch(AppAction.DeleteChat(chatId))
                            onBack()
                        },
                        modifier = Modifier.testTag("directChatDeleteButton"),
                    ) {
                        Icon(
                            imageVector = IrisIcons.DeleteForever,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
        }
    }

    if (showBlockDialog) {
        MessageRequestBlockDialog(
            displayName = chat.displayName,
            onDismiss = { showBlockDialog = false },
            onBlock = {
                appManager.dispatch(AppAction.SetUserBlocked(chatId, true))
                showBlockDialog = false
            },
            onReportAndBlock = {
                reportUser(context, appManager, clipboard, chatId, chat.displayName, block = true)
                showBlockDialog = false
            },
            onDelete = {
                appManager.dispatch(AppAction.DeleteChat(chatId))
                onBack()
                showBlockDialog = false
            },
        )
    }

    if (showReportDialog) {
        MessageRequestReportDialog(
            displayName = chat.displayName,
            onDismiss = { showReportDialog = false },
            onReport = {
                reportUser(context, appManager, clipboard, chatId, chat.displayName, block = false)
                showReportDialog = false
            },
            onReportAndBlock = {
                reportUser(context, appManager, clipboard, chatId, chat.displayName, block = true)
                showReportDialog = false
            },
            onDelete = {
                appManager.dispatch(AppAction.DeleteChat(chatId))
                onBack()
                showReportDialog = false
            },
        )
    }
}

@Composable
private fun ProfileAboutCard(
    about: String?,
    modifier: Modifier = Modifier,
) {
    val text = about?.trim()?.takeIf { it.isNotEmpty() } ?: return
    val linkColor = MaterialTheme.colorScheme.primary
    IrisSectionCard(modifier = modifier.testTag("directChatAboutCard")) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                imageVector = IrisIcons.Edit,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurface,
                modifier =
                    Modifier
                        .padding(top = 2.dp)
                        .size(22.dp),
            )
            Text(
                text = remember(text, linkColor) { linkHighlightedText(text, linkColor) },
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun linkHighlightedText(
    text: String,
    linkColor: Color,
): AnnotatedString =
    buildAnnotatedString {
        var cursor = 0
        for (match in messageUrlMatches(text)) {
            val range = match.range
            if (range.first > cursor) {
                append(text.substring(cursor, range.first))
            }
            withStyle(
                SpanStyle(
                    color = linkColor,
                    textDecoration = TextDecoration.Underline,
                ),
            ) {
                append(match.visible)
            }
            cursor = match.range.last + 1
        }
        if (cursor < text.length) {
            append(text.substring(cursor))
        }
    }

@Composable
private fun ContactNicknameCard(
    chat: to.iris.chat.rust.CurrentChatSnapshot,
    nicknameDraft: String,
    onNicknameChange: (String) -> Unit,
    onSave: () -> Unit,
    onRemove: () -> Unit,
    editing: Boolean,
    onToggleEditing: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val storedNickname = chat.nickname?.trim().orEmpty()
    val primaryName = storedNickname.ifEmpty { chat.displayName.trim() }
    val profileName =
        chat.profileName
            ?.trim()
            ?.takeIf { it.isNotEmpty() && !it.equals(primaryName, ignoreCase = true) }

    IrisSectionCard(modifier = modifier) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onToggleEditing)
                    .testTag("directChatNicknameRow"),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Nickname",
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
            )
            if (storedNickname.isNotEmpty()) {
                Text(
                    text = storedNickname,
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        profileName?.let { name ->
            IrisDivider()
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("directChatProfileNameRow"),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Profile name",
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = name,
                    style = MaterialTheme.typography.bodyLarge,
                    color = IrisTheme.palette.muted,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (editing) {
            IrisDivider()
            TextField(
                value = nicknameDraft,
                onValueChange = onNicknameChange,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("directChatNicknameField"),
                label = { Text("Nickname") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                colors = irisTextFieldColors(),
            )
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IrisInlineAction(
                    text = "Save",
                    onClick = onSave,
                    modifier = Modifier.testTag("directChatSaveNicknameButton"),
                ) {
                    Icon(imageVector = IrisIcons.Check, contentDescription = null)
                }
                if (storedNickname.isNotEmpty()) {
                    IrisInlineAction(
                        text = "Remove",
                        onClick = onRemove,
                        modifier = Modifier.testTag("directChatRemoveNicknameButton"),
                    ) {
                        Icon(imageVector = IrisIcons.Close, contentDescription = null)
                    }
                }
            }
        }
    }
}

@Composable
private fun CommonGroupsCard(
    appManager: AppManager,
    groups: List<ChatThreadSnapshot>,
    onBack: () -> Unit,
) {
    IrisSectionCard {
        Text(
            text = "Groups in common",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface,
        )
        groups.forEachIndexed { index, group ->
            CommonGroupRow(
                appManager = appManager,
                group = group,
                onBack = onBack,
            )
            if (index < groups.lastIndex) {
                IrisDivider(modifier = Modifier.padding(start = 50.dp))
            }
        }
    }
}

@Composable
private fun CommonGroupRow(
    appManager: AppManager,
    group: ChatThreadSnapshot,
    onBack: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable {
                    val groupId = groupIdFromChatId(group.chatId) ?: return@clickable
                    haptics.press()
                    onBack()
                    appManager.dispatch(AppAction.PushScreen(Screen.GroupDetails(groupId)))
                }
                .padding(vertical = 2.dp)
                .testTag("directChatCommonGroup-${group.chatId.take(12)}"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IrisAvatar(
            label = group.displayName,
            size = 38.dp,
            imageUrl = group.pictureUrl,
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            Text(
                text = group.displayName,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${group.memberCount} people",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Icon(
            imageVector = IrisIcons.ChevronRight,
            contentDescription = null,
            tint = IrisTheme.palette.muted,
            modifier = Modifier.size(22.dp),
        )
    }
}

private fun groupIdFromChatId(chatId: String): String? {
    val trimmed = chatId.trim()
    val prefix = "group:"
    if (!trimmed.startsWith(prefix, ignoreCase = true)) {
        return null
    }
    return trimmed.drop(prefix.length).trim().takeIf { it.isNotEmpty() }
}

@Composable
private fun DirectChatAdvancedCard(
    debug: PeerProfileDebugSnapshot?,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    IrisSectionCard(modifier = modifier) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(
                        interactionSource = interactionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onToggle()
                    },
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = IrisIcons.Devices,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                text = "Debug",
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Icon(
                imageVector = IrisIcons.ChevronRight,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.graphicsLayer { rotationZ = if (expanded) 90f else 0f },
            )
        }

        if (expanded) {
            if (debug == null) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    strokeWidth = 2.dp,
                    color = IrisTheme.palette.accent,
                )
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    ProfileDebugRow("Sessions", debug.sessionCount.toString())
                    ProfileDebugRow("Active sessions", debug.activeSessionCount.toString())
                    ProfileDebugRow("Receiving sessions", debug.receivingSessionCount.toString())
                    ProfileDebugRow("Known devices", debug.knownDeviceCount.toString())
                    ProfileDebugRow("Device roster", debug.rosterDeviceCount.toString())
                    ProfileDebugRow("Tracked senders", debug.trackedSenderCount.toString())
                    ProfileDebugRow("Recent handshakes", debug.recentHandshakeDeviceCount.toString())
                    ProfileDebugRow("Last handshake", lastHandshakeText(debug.lastHandshakeAtSecs))
                    ProfileDebugRow("Message tracking", if (debug.trackedForMessages) "On" else "Off")
                    ProfileDebugRow("User ID", debug.ownerNpub, monospaced = true)
                    ProfileDebugRow("Public key", debug.ownerPubkeyHex, monospaced = true)
                }
            }
        }
    }
}

@Composable
private fun ProfileDebugRow(
    label: String,
    value: String,
    monospaced: Boolean = false,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = label,
            modifier = Modifier.weight(0.9f),
            style = MaterialTheme.typography.bodyMedium,
            color = IrisTheme.palette.muted,
        )
        Text(
            text = value,
            modifier = Modifier.weight(1.1f),
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.SemiBold,
            fontFamily = if (monospaced) FontFamily.Monospace else null,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

private fun lastHandshakeText(seconds: ULong?): String {
    val value = seconds?.toLong() ?: return "Never"
    val ageSecs = ((System.currentTimeMillis() / 1_000L) - value).coerceAtLeast(0L)
    return when {
        ageSecs < 60L -> "Just now"
        ageSecs < 3_600L -> "${ageSecs / 60L}m ago"
        ageSecs < 86_400L -> "${ageSecs / 3_600L}h ago"
        else -> "${ageSecs / 86_400L}d ago"
    }
}
