package to.iris.chat.ui.components

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
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.ui.platform.testTag
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
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
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.NotificationsOff
import androidx.compose.material.icons.rounded.PersonRemove
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Sensors
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
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

private val CardShape = RoundedCornerShape(24.dp)
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
            tonalElevation = 0.dp,
            shadowElevation = 0.dp,
        ) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(start = 14.dp, end = 12.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                when {
                    onBack != null -> {
                        BadgedBox(
                            badge = {
                                if (backBadgeCount > 0uL) {
                                    Badge(
                                        containerColor = IrisTheme.palette.accent,
                                        contentColor = Color.White,
                                    ) {
                                        Text(
                                            if (backBadgeCount > 99uL) "99+" else backBadgeCount.toString(),
                                            color = Color.White,
                                        )
                                    }
                                }
                            },
                        ) {
                            IconButton(
                                onClick = onBack,
                                modifier = Modifier.size(40.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.AutoMirrored.Rounded.ArrowBack,
                                    contentDescription = "Back",
                                )
                            }
                        }
                    }

                    leading != null -> {
                        Row(
                            modifier = Modifier.padding(start = 2.dp),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            content = leading,
                        )
                    }

                    else -> {
                        Spacer(modifier = Modifier.size(40.dp))
                    }
                }

                Row(
                    modifier =
                        Modifier
                            .weight(1f)
                            .let { base ->
                                if (onTitleClick != null) {
                                    base
                                        .clickable(onClick = onTitleClick)
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
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.Bold,
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
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
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
                    maxLines = 1,
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
    val dataBitmap = remember(imageData) {
        imageData?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
    }
    val avatarBitmap =
        produceState(
            initialValue = imageUrl?.trim()?.let { IrisAvatarBitmapCache.get(it) },
            imageUrl,
        ) {
            val url = imageUrl?.trim().orEmpty()
            if (!url.startsWith("https://") && !url.startsWith("http://")) {
                value = null
                return@produceState
            }
            IrisAvatarBitmapCache.get(url)?.let { cached ->
                value = cached
                return@produceState
            }
            val bitmap =
                withContext(Dispatchers.IO) {
                    runCatching {
                        URL(url).openStream().use { BitmapFactory.decodeStream(it) }
                    }.getOrNull()
                }
            if (bitmap != null) {
                IrisAvatarBitmapCache.put(url, bitmap)
            }
            value = bitmap
        }
    Box(
        modifier =
            modifier
                .size(size)
                .clip(CircleShape)
                .background(if (emphasize) palette.accent else palette.panelAlt)
                .border(1.dp, palette.border, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        dataBitmap?.let { bitmap ->
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

private object IrisAvatarBitmapCache {
    private val bitmaps = ConcurrentHashMap<String, android.graphics.Bitmap>()

    fun get(key: String): android.graphics.Bitmap? = bitmaps[key]

    fun put(key: String, bitmap: android.graphics.Bitmap) {
        bitmaps[key] = bitmap
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
        border = BorderStroke(1.dp, palette.border),
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
fun IrisPrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: (@Composable () -> Unit)? = null,
) {
    Button(
        onClick = onClick,
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
    OutlinedButton(
        onClick = onClick,
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
    TextButton(onClick = onClick, modifier = modifier) {
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            icon?.invoke()
            Text(text)
        }
    }
}

@Composable
fun IrisChatListRow(
    title: String,
    isMuted: Boolean = false,
    preview: String?,
    timeLabel: String?,
    imageUrl: String? = null,
    imageData: ByteArray? = null,
    leadingContent: (@Composable () -> Unit)? = null,
    unreadCount: Long,
    lastMessageMine: Boolean,
    lastDelivery: DeliveryState?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val palette = IrisTheme.palette
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (leadingContent != null) {
            leadingContent()
        } else {
            IrisAvatar(label = title, size = 42.dp, imageUrl = imageUrl, imageData = imageData)
        }
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
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
                        style = MaterialTheme.typography.titleMedium,
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
                }
                if (timeLabel != null) {
                    Text(
                        text = timeLabel,
                        style = MaterialTheme.typography.labelMedium,
                        color = palette.muted,
                    )
                }
            }
            if (preview != null || (lastMessageMine && lastDelivery != null) || unreadCount > 0) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (preview != null) {
                        Text(
                            text = preview,
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodyMedium,
                            color = palette.muted,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    } else {
                        Spacer(modifier = Modifier.weight(1f))
                    }
                    if (lastMessageMine && lastDelivery != null) {
                        DeliveryGlyph(lastDelivery)
                    }
                    if (unreadCount > 0) {
                        BadgedBox(
                            badge = {
                                Badge(containerColor = palette.accent, contentColor = Color.White) {
                                    Text(
                                        if (unreadCount > 99) "99+" else unreadCount.toString(),
                                        color = Color.White,
                                    )
                                }
                            },
                        ) {
                            Spacer(modifier = Modifier.size(1.dp))
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
    val large = 22.dp
    val tail = 6.dp
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
