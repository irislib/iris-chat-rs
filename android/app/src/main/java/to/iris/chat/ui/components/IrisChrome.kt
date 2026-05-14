package to.iris.chat.ui.components

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.text.format.DateUtils
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.InsertDriveFile
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.rounded.Logout
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.AddComment
import androidx.compose.material.icons.rounded.AttachFile
import androidx.compose.material.icons.rounded.Audiotrack
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material.icons.rounded.DoneAll
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Group
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.IosShare
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.MarkEmailRead
import androidx.compose.material.icons.rounded.MarkEmailUnread
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.NotificationsOff
import androidx.compose.material.icons.rounded.PersonRemove
import androidx.compose.material.icons.rounded.PushPin
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Sensors
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import to.iris.chat.rust.DeliveryState
import to.iris.chat.ui.theme.IrisTheme
import to.iris.chat.ui.theme.Sky500
import java.net.URL
import java.text.SimpleDateFormat
import java.util.concurrent.ConcurrentHashMap
import java.util.Date
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt

private val CardShape = RoundedCornerShape(8.dp)
private val PillShape = RoundedCornerShape(100.dp)

@Immutable
data class IrisOfflineBannerState(
    val text: String,
)

val LocalIrisOfflineBannerState =
    staticCompositionLocalOf<IrisOfflineBannerState?> {
        null
    }

@Composable
fun IrisTopBar(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    subtitleIcon: ImageVector? = null,
    onBack: (() -> Unit)? = null,
    backBadgeCount: ULong = 0uL,
    leading: (@Composable RowScope.() -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
    titleAccessoryLeading: (@Composable () -> Unit)? = null,
    onTitleClick: (() -> Unit)? = null,
) {
    val palette = IrisTheme.palette
    val offlineBanner = LocalIrisOfflineBannerState.current
    val haptics = rememberIrisHapticFeedback()
    val titleInteractionSource = remember { MutableInteractionSource() }
    Column(
        modifier =
            modifier
                .fillMaxWidth(),
    ) {
        Surface(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .statusBarsPadding(),
            color = palette.toolbar,
            contentColor = MaterialTheme.colorScheme.onSurface,
            tonalElevation = 0.dp,
            shadowElevation = 0.dp,
        ) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(64.dp)
                        .padding(
                            start = if (onBack != null) 4.dp else 16.dp,
                            end = 8.dp,
                        ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                when {
                    onBack != null -> {
                        BadgedBox(
                            badge = {
                                if (backBadgeCount > 0uL) {
                                    Badge(
                                        containerColor = IrisTheme.palette.accent,
                                        contentColor = MaterialTheme.colorScheme.onPrimary,
                                    ) {
                                        Text(
                                            if (backBadgeCount > 99uL) "99+" else backBadgeCount.toString(),
                                            color = MaterialTheme.colorScheme.onPrimary,
                                        )
                                    }
                                }
                            },
                        ) {
                            IconButton(
                                onClick = {
                                    haptics.press()
                                    onBack()
                                },
                                modifier = Modifier.size(48.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.AutoMirrored.Rounded.ArrowBack,
                                    contentDescription = "Back",
                                    modifier = Modifier.size(24.dp),
                                )
                            }
                        }
                    }

                    leading != null -> {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                            content = leading,
                        )
                    }

                    else -> {
                        Spacer(modifier = Modifier.size(48.dp))
                    }
                }

                Row(
                    modifier =
                        Modifier
                            .weight(1f)
                            .let { base ->
                                if (onTitleClick != null) {
                                    base
                                        .clickable(
                                            interactionSource = titleInteractionSource,
                                            indication = null,
                                            onClick = {
                                                haptics.press()
                                                onTitleClick()
                                            },
                                        )
                                        .testTag("chatHeaderTitleButton")
                                } else {
                                    base
                                }
                            },
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    titleAccessoryLeading?.invoke()
                    Column(
                        modifier = Modifier.weight(1f),
                        verticalArrangement = Arrangement.spacedBy(1.dp),
                    ) {
                        Text(
                            text = title,
                            style =
                                if (titleAccessoryLeading != null) {
                                    MaterialTheme.typography.titleMedium
                                } else {
                                    MaterialTheme.typography.titleLarge
                                },
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )

                        if (!subtitle.isNullOrBlank()) {
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                if (subtitleIcon != null) {
                                    Icon(
                                        imageVector = subtitleIcon,
                                        contentDescription = null,
                                        modifier = Modifier.size(12.dp),
                                        tint = palette.muted,
                                    )
                                }
                                Text(
                                    text = subtitle,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = palette.muted,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                    }
                }

                Row(
                    horizontalArrangement = Arrangement.spacedBy(0.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    content = actions,
                )
            }
        }

        AnimatedVisibility(
            visible = offlineBanner != null,
            enter = expandVertically(expandFrom = Alignment.Top) + fadeIn(),
            exit = shrinkVertically(shrinkTowards = Alignment.Top) + fadeOut(),
        ) {
            Surface(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("offlineStatusBanner"),
                color = palette.accentAlt,
                contentColor = Color.White,
                tonalElevation = 0.dp,
                shadowElevation = 0.dp,
            ) {
                Text(
                    text = offlineBanner?.text.orEmpty(),
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 6.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = Color.White,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = TextAlign.Center,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
fun IrisAvatar(
    label: String,
    modifier: Modifier = Modifier,
    size: Dp = 40.dp,
    emphasize: Boolean = false,
    imageUrl: String? = null,
    imageData: ByteArray? = null,
) {
    val palette = IrisTheme.palette
    val targetPx = with(LocalDensity.current) { (size.toPx() * 2f).roundToInt().coerceAtLeast(1) }
    val dataBitmap =
        produceState(
            initialValue =
                imageData?.let { data ->
                    IrisAvatarBitmapCache.get(IrisAvatarBitmapCache.dataKey(data, targetPx))
                },
            imageData,
            targetPx,
        ) {
            val data = imageData
            if (data == null || data.isEmpty()) {
                value = null
                return@produceState
            }
            val key = IrisAvatarBitmapCache.dataKey(data, targetPx)
            IrisAvatarBitmapCache.get(key)?.let { cached ->
                value = cached
                return@produceState
            }
            val bitmap = withContext(Dispatchers.Default) { decodeAvatarBitmap(data, targetPx) }
            if (bitmap != null) {
                IrisAvatarBitmapCache.put(key, bitmap)
            }
            value = bitmap
        }
    val avatarBitmap =
        produceState(
            initialValue =
                imageUrl
                    ?.trim()
                    ?.let { IrisAvatarBitmapCache.get(IrisAvatarBitmapCache.urlKey(it, targetPx)) },
            imageUrl,
            targetPx,
        ) {
            val url = imageUrl?.trim().orEmpty()
            if (!url.startsWith("https://") && !url.startsWith("http://")) {
                value = null
                return@produceState
            }
            val key = IrisAvatarBitmapCache.urlKey(url, targetPx)
            IrisAvatarBitmapCache.get(key)?.let { cached ->
                value = cached
                return@produceState
            }
            val bitmap =
                withContext(Dispatchers.IO) {
                    runCatching {
                        URL(url).openStream().use { decodeAvatarBitmap(it.readBytes(), targetPx) }
                    }.getOrNull()
                }
            if (bitmap != null) {
                IrisAvatarBitmapCache.put(key, bitmap)
            }
            value = bitmap
        }
    Box(
        modifier =
            modifier
                .size(size)
                .clip(CircleShape)
                .background(if (emphasize) palette.accent else palette.panelAlt),
        contentAlignment = Alignment.Center,
    ) {
        dataBitmap.value?.let { bitmap ->
            Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.size(size),
            )
        } ?: avatarBitmap.value?.let { bitmap ->
            Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.size(size),
            )
        } ?: Text(
            text = label.take(1).uppercase(),
            style = MaterialTheme.typography.titleSmall,
            color = if (emphasize) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
            fontWeight = FontWeight.Bold,
        )
    }
}

private fun decodeAvatarBitmap(data: ByteArray, targetSizePx: Int): Bitmap? {
    val bounds =
        BitmapFactory.Options().apply {
            inJustDecodeBounds = true
        }
    BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
        return null
    }
    val options =
        BitmapFactory.Options().apply {
            inPreferredConfig = Bitmap.Config.ARGB_8888
            inSampleSize = avatarSampleSize(bounds.outWidth, bounds.outHeight, targetSizePx)
        }
    return BitmapFactory.decodeByteArray(data, 0, data.size, options)
}

private fun avatarSampleSize(
    width: Int,
    height: Int,
    targetSizePx: Int,
): Int {
    var sampleSize = 1
    while (width / (sampleSize * 2) >= targetSizePx && height / (sampleSize * 2) >= targetSizePx) {
        sampleSize *= 2
    }
    return sampleSize
}

private object IrisAvatarBitmapCache {
    private const val MaxEntries = 160
    private val bitmaps = ConcurrentHashMap<String, android.graphics.Bitmap>()

    fun get(key: String): android.graphics.Bitmap? = bitmaps[key]

    fun dataKey(data: ByteArray, targetSizePx: Int): String =
        "data:${System.identityHashCode(data)}:${data.size}:$targetSizePx"

    fun urlKey(url: String, targetSizePx: Int): String = "url:$targetSizePx:$url"

    fun put(key: String, bitmap: android.graphics.Bitmap) {
        bitmaps[key] = bitmap
        if (bitmaps.size > MaxEntries) {
            bitmaps.keys.firstOrNull()?.let { bitmaps.remove(it) }
        }
    }
}

@Composable
fun IrisSectionCard(
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(18.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    val palette = IrisTheme.palette
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = palette.panel,
        shape = CardShape,
        shadowElevation = 0.dp,
        tonalElevation = 0.dp,
    ) {
        Column(
            modifier = Modifier.padding(contentPadding),
            verticalArrangement = Arrangement.spacedBy(14.dp),
            content = content,
        )
    }
}

@Composable
fun IrisListSection(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = IrisTheme.palette.panel,
        shape = RectangleShape,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Column(content = content)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun irisTextFieldColors(containerColor: Color = IrisTheme.palette.panelAlt) =
    TextFieldDefaults.colors(
        focusedTextColor = MaterialTheme.colorScheme.onSurface,
        unfocusedTextColor = MaterialTheme.colorScheme.onSurface,
        disabledTextColor = IrisTheme.palette.muted,
        focusedContainerColor = containerColor,
        unfocusedContainerColor = containerColor,
        disabledContainerColor = containerColor,
        cursorColor = MaterialTheme.colorScheme.onSurface,
        focusedIndicatorColor = Color.Transparent,
        unfocusedIndicatorColor = Color.Transparent,
        disabledIndicatorColor = Color.Transparent,
        focusedLabelColor = IrisTheme.palette.muted,
        unfocusedLabelColor = IrisTheme.palette.muted,
        disabledLabelColor = IrisTheme.palette.muted.copy(alpha = 0.54f),
        focusedPlaceholderColor = IrisTheme.palette.muted,
        unfocusedPlaceholderColor = IrisTheme.palette.muted,
        disabledPlaceholderColor = IrisTheme.palette.muted.copy(alpha = 0.54f),
    )

@Composable
fun IrisMenuRow(
    title: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    icon: ImageVector? = null,
    leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable RowScope.() -> Unit)? = null,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                    onClick = {
                        haptics.press()
                        onClick()
                    },
                )
                .heightIn(min = 56.dp)
                .padding(horizontal = 16.dp, vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(24.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when {
            leading != null -> leading()
            icon != null -> {
                Icon(
                    imageVector = icon,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(24.dp),
                )
            }
        }
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (!subtitle.isNullOrBlank()) {
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (trailing != null) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
                content = trailing,
            )
        } else {
            Icon(
                imageVector = IrisIcons.ChevronRight,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.size(24.dp),
            )
        }
    }
}

@Composable
fun IrisToggleRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
) {
    val haptics = rememberIrisHapticFeedback()
    IrisMenuRow(
        title = title,
        subtitle = subtitle,
        onClick = { onCheckedChange(!checked) },
        modifier =
            modifier.semantics {
                toggleableState = if (checked) ToggleableState.On else ToggleableState.Off
            },
        trailing = {
            Switch(
                checked = checked,
                onCheckedChange = { value ->
                    haptics.press()
                    onCheckedChange(value)
                },
                colors =
                    SwitchDefaults.colors(
                        checkedThumbColor = MaterialTheme.colorScheme.onPrimary,
                        checkedTrackColor = MaterialTheme.colorScheme.primary,
                        checkedBorderColor = MaterialTheme.colorScheme.primary,
                        uncheckedThumbColor = IrisTheme.palette.muted,
                        uncheckedTrackColor = IrisTheme.palette.panelAlt,
                        uncheckedBorderColor = IrisTheme.palette.border,
                    ),
            )
        },
    )
}

@Composable
fun IrisPrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: (@Composable () -> Unit)? = null,
) {
    val haptics = rememberIrisHapticFeedback()
    Button(
        onClick = {
            haptics.confirm()
            onClick()
        },
        enabled = enabled,
        modifier = modifier,
        shape = PillShape,
        contentPadding = PaddingValues(horizontal = 18.dp, vertical = 14.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = IrisTheme.palette.accent,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            icon?.invoke()
            Text(text)
        }
    }
}

@Composable
fun IrisSecondaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: (@Composable () -> Unit)? = null,
) {
    val haptics = rememberIrisHapticFeedback()
    OutlinedButton(
        onClick = {
            haptics.press()
            onClick()
        },
        enabled = enabled,
        modifier = modifier,
        shape = PillShape,
        border = BorderStroke(1.dp, IrisTheme.palette.border),
        contentPadding = PaddingValues(horizontal = 18.dp, vertical = 14.dp),
        colors =
            ButtonDefaults.outlinedButtonColors(
                containerColor = IrisTheme.palette.panel,
                contentColor = MaterialTheme.colorScheme.onSurface,
            ),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            icon?.invoke()
            Text(text)
        }
    }
}

@Composable
fun IrisInlineAction(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: (@Composable () -> Unit)? = null,
) {
    IrisTextButton(onClick = onClick, modifier = modifier) {
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            icon?.invoke()
            Text(text)
        }
    }
}

@Composable
fun IrisTextButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    destructive: Boolean = false,
    confirm: Boolean = false,
    content: @Composable RowScope.() -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val contentColor =
        if (destructive) {
            MaterialTheme.colorScheme.error
        } else {
            MaterialTheme.colorScheme.onSurface
        }
    TextButton(
        onClick = {
            if (confirm || destructive) {
                haptics.confirm()
            } else {
                haptics.press()
            }
            onClick()
        },
        modifier = modifier,
        enabled = enabled,
        colors =
            ButtonDefaults.textButtonColors(
                contentColor = contentColor,
                disabledContentColor = contentColor.copy(alpha = 0.38f),
            ),
        content = content,
    )
}

@Composable
fun IrisChatListRow(
    title: String,
    modifier: Modifier = Modifier,
    isMuted: Boolean = false,
    isPinned: Boolean = false,
    preview: String?,
    timeLabel: String?,
    imageUrl: String? = null,
    imageData: ByteArray? = null,
    leadingContent: (@Composable () -> Unit)? = null,
    previewLeading: (@Composable () -> Unit)? = null,
    unreadCount: Long,
    lastMessageMine: Boolean,
    lastDelivery: DeliveryState?,
    onClick: () -> Unit,
) {
    val palette = IrisTheme.palette
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    // Signal-Android spec: 48dp avatar, BodyLarge title in onSurface,
    // BodyMedium preview in secondary tint, BodyMedium time in
    // tertiary tint, 16dp side gutter, 10dp top/bottom padding so the
    // row sits at the 84dp min-height when the preview is one line.
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = 84.dp)
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                    onClick = {
                        haptics.press()
                        onClick()
                    },
                )
                .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        // 20dp matches Signal-Android's conversation_list_item_view
        // (16dp marginStart + 4dp implicit on the avatar's right
        // edge), so the row title lines up where Signal puts it.
        horizontalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        if (leadingContent != null) {
            leadingContent()
        } else {
            IrisAvatar(label = title, size = 48.dp, imageUrl = imageUrl, imageData = imageData)
        }
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(5.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = title,
                        modifier = Modifier.weight(1f, fill = false),
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (isMuted) {
                        Icon(
                            imageVector = IrisIcons.NotificationsOff,
                            contentDescription = "muted",
                            modifier = Modifier.size(14.dp),
                            tint = palette.muted,
                        )
                    }
                    if (isPinned) {
                        Icon(
                            imageVector = IrisIcons.Pin,
                            contentDescription = "pinned",
                            modifier = Modifier.size(14.dp),
                            tint = palette.muted,
                        )
                    }
                }
                if (timeLabel != null) {
                    Text(
                        text = timeLabel,
                        style = MaterialTheme.typography.bodyMedium,
                        color = palette.muted,
                    )
                }
            }
            if (preview != null || previewLeading != null || (lastMessageMine && lastDelivery != null) || unreadCount > 0) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (previewLeading != null) {
                        previewLeading()
                    }
                    if (preview != null) {
                        Text(
                            text = preview,
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodyMedium,
                            color = palette.muted,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    } else {
                        Spacer(modifier = Modifier.weight(1f))
                    }
                    if (lastMessageMine && lastDelivery != null) {
                        DeliveryGlyph(lastDelivery)
                    }
                    if (unreadCount > 0) {
                        Surface(
                            color = palette.accent,
                            contentColor = MaterialTheme.colorScheme.onPrimary,
                            shape = PillShape,
                        ) {
                            Text(
                                text = if (unreadCount > 99) "99+" else unreadCount.toString(),
                                modifier =
                                    Modifier
                                        .heightIn(min = 18.dp)
                                        .widthIn(min = 18.dp)
                                        .padding(horizontal = 6.dp, vertical = 1.dp),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onPrimary,
                                textAlign = TextAlign.Center,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun DeliveryGlyph(
    delivery: DeliveryState,
    isOutgoing: Boolean = false,
) {
    // Match iOS: delivered uses double-check in the bubble's text colour;
    // seen escalates to the accent (sky/blue). Single-check stays muted while
    // a message is queued/pending/sent.
    val onBubble =
        if (isOutgoing) {
            MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.72f)
        } else {
            IrisTheme.palette.muted
        }
    val tint =
        when (delivery) {
            DeliveryState.QUEUED -> onBubble
            DeliveryState.PENDING -> onBubble
            DeliveryState.SENT -> onBubble
            DeliveryState.RECEIVED -> onBubble
            DeliveryState.SEEN -> Sky500
            DeliveryState.FAILED -> MaterialTheme.colorScheme.error
        }
    val imageVector =
        when (delivery) {
            DeliveryState.QUEUED -> Icons.Rounded.Schedule
            DeliveryState.PENDING -> Icons.Rounded.Schedule
            DeliveryState.SENT -> Icons.Rounded.Check
            DeliveryState.RECEIVED -> Icons.Rounded.DoneAll
            DeliveryState.SEEN -> Icons.Rounded.DoneAll
            DeliveryState.FAILED -> Icons.Rounded.MoreHoriz
        }
    Icon(
        imageVector = imageVector,
        contentDescription = delivery.name,
        tint = tint,
        modifier = Modifier.size(14.dp),
    )
}

fun formatRelativeTime(
    lastMessageAtSecs: Long?,
    nowMillis: Long = System.currentTimeMillis(),
): String? {
    val seconds = lastMessageAtSecs ?: return null
    val timeMillis = seconds * 1000
    val elapsedMillis = abs(nowMillis - timeMillis)
    if (elapsedMillis < DateUtils.MINUTE_IN_MILLIS) {
        return "now"
    }
    if (elapsedMillis < DateUtils.HOUR_IN_MILLIS) {
        return "${elapsedMillis / DateUtils.MINUTE_IN_MILLIS}m"
    }
    if (elapsedMillis < DateUtils.DAY_IN_MILLIS) {
        return "${elapsedMillis / DateUtils.HOUR_IN_MILLIS}h"
    }
    return "${elapsedMillis / DateUtils.DAY_IN_MILLIS}d"
}

fun formatMessageClock(createdAtSecs: Long): String =
    SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(createdAtSecs * 1000))

fun formatTimelineDay(createdAtSecs: Long): String {
    val timeMillis = createdAtSecs * 1000
    return when {
        DateUtils.isToday(timeMillis) -> "Today"
        DateUtils.isToday(timeMillis + DateUtils.DAY_IN_MILLIS) -> "Yesterday"
        else -> SimpleDateFormat("EEE, d MMM", Locale.getDefault()).format(Date(timeMillis))
    }
}

fun isSameTimelineDay(first: Long, second: Long): Boolean {
    val fmt = SimpleDateFormat("yyyy-MM-dd", Locale.US)
    return fmt.format(Date(first * 1000)) == fmt.format(Date(second * 1000))
}

fun messageBubbleShape(
    isOutgoing: Boolean,
    isFirstInCluster: Boolean,
    isLastInCluster: Boolean,
): Shape {
    val large = 18.dp
    val tail = 4.dp
    return when {
        isFirstInCluster && isLastInCluster -> RoundedCornerShape(large)
        isOutgoing && isFirstInCluster ->
            RoundedCornerShape(topStart = large, topEnd = large, bottomStart = large, bottomEnd = tail)
        isOutgoing && isLastInCluster ->
            RoundedCornerShape(topStart = large, topEnd = tail, bottomStart = large, bottomEnd = large)
        isOutgoing ->
            RoundedCornerShape(topStart = large, topEnd = tail, bottomStart = large, bottomEnd = tail)
        !isOutgoing && isFirstInCluster ->
            RoundedCornerShape(topStart = large, topEnd = large, bottomStart = tail, bottomEnd = large)
        !isOutgoing && isLastInCluster ->
            RoundedCornerShape(topStart = tail, topEnd = large, bottomStart = large, bottomEnd = large)
        else ->
            RoundedCornerShape(topStart = tail, topEnd = large, bottomStart = tail, bottomEnd = large)
    }
}

@Composable
fun IrisDivider(modifier: Modifier = Modifier) {
    Box(
        modifier =
            modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(IrisTheme.palette.border),
    )
}

object IrisIcons {
    val NewChat = Icons.Rounded.AddComment
    val NewGroup = Icons.Rounded.Group
    val ScanQr = Icons.Rounded.QrCodeScanner
    val Send = Icons.AutoMirrored.Rounded.Send
    val Attach = Icons.Rounded.AttachFile
    val Copy = Icons.Rounded.ContentCopy
    val DeleteForever = Icons.Rounded.DeleteForever
    val MarkRead = Icons.Rounded.MarkEmailRead
    val MarkUnread = Icons.Rounded.MarkEmailUnread
    val Pin = Icons.Rounded.PushPin
    val Devices = Icons.Rounded.Devices
    val Edit = Icons.Rounded.Edit
    val File = Icons.AutoMirrored.Rounded.InsertDriveFile
    val Image = Icons.Rounded.Image
    val Key = Icons.Rounded.Key
    val Movie = Icons.Rounded.Movie
    val Audio = Icons.Rounded.Audiotrack
    val Notifications = Icons.Rounded.Notifications
    val NotificationsOff = Icons.Rounded.NotificationsOff
    val Close = Icons.Rounded.Close
    val RemoveMember = Icons.Rounded.PersonRemove
    val Logout = Icons.AutoMirrored.Rounded.Logout
    val Refresh = Icons.Rounded.Refresh
    val Share = Icons.Rounded.IosShare
    val ChevronRight = Icons.AutoMirrored.Rounded.KeyboardArrowRight
    val Nearby = Icons.Rounded.Sensors
}
