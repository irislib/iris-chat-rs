package to.iris.chat.ui.screens

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import android.util.Log
import android.util.LruCache
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Archive
import androidx.compose.material.icons.rounded.Audiotrack
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun SelectedAttachmentChip(
    attachment: PickedAttachment,
    enabled: Boolean,
    onRemove: () -> Unit,
) {
    val selectedAttachmentType = attachmentType(attachment)
    val haptics = rememberIrisHapticFeedback()

    if (selectedAttachmentType == ChatAttachmentType.IMAGE) {
        SelectedImageAttachmentChip(
            attachment = attachment,
            enabled = enabled,
            onRemove = {
                haptics.press()
                onRemove()
            },
        )
        return
    }

    Surface(
        color = IrisTheme.palette.panel,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.semantics {
            contentDescription = "${selectedAttachmentType.label}, ${attachment.filename}"
        },
    ) {
        Row(
            modifier = Modifier.padding(start = 10.dp, top = 7.dp, end = 4.dp, bottom = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = selectedAttachmentType.icon,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.size(18.dp),
            )
            Column(
                modifier = Modifier.widthIn(max = 200.dp),
                verticalArrangement = Arrangement.spacedBy(1.dp),
            ) {
                Text(
                    text = attachment.filename,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = selectedAttachmentType.label,
                    style = MaterialTheme.typography.labelSmall,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(
                onClick = {
                    haptics.press()
                    onRemove()
                },
                enabled = enabled,
                modifier =
                    Modifier
                        .size(28.dp)
                        .testTag("chatSelectedAttachmentRemove"),
            ) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Remove attachment",
                    tint = IrisTheme.palette.muted,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
    }
}

@Composable
private fun SelectedImageAttachmentChip(
    attachment: PickedAttachment,
    enabled: Boolean,
    onRemove: () -> Unit,
) {
    var bitmap by remember(attachment.path) {
        mutableStateOf(SelectedImageThumbnailCache.get(attachment.path))
    }
    LaunchedEffect(attachment.path) {
        if (bitmap == null) {
            val decoded = withContext(Dispatchers.IO) {
                decodeStagedImageThumbnail(attachment.path)
            }
            if (decoded != null) {
                SelectedImageThumbnailCache.put(attachment.path, decoded)
                bitmap = decoded
            }
        }
    }

    Box(
        modifier = Modifier
            .size(56.dp)
            .semantics { contentDescription = "Image, ${attachment.filename}" },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .clip(RoundedCornerShape(12.dp))
                .background(IrisTheme.palette.panel),
            contentAlignment = Alignment.Center,
        ) {
            val current = bitmap
            if (current != null) {
                Image(
                    bitmap = current.asImageBitmap(),
                    contentDescription = null,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop,
                )
            } else {
                Icon(
                    imageVector = Icons.Rounded.Image,
                    contentDescription = null,
                    tint = IrisTheme.palette.muted,
                    modifier = Modifier.size(22.dp),
                )
            }
        }
        Box(
            modifier = Modifier
                .align(Alignment.TopEnd)
                .padding(top = 3.dp, end = 3.dp)
                .size(20.dp)
                .clip(CircleShape)
                .background(Color.Black.copy(alpha = 0.6f))
                .clickable(enabled = enabled, onClick = onRemove)
                .testTag("chatSelectedAttachmentRemove"),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = IrisIcons.Close,
                contentDescription = "Remove attachment",
                tint = Color.White,
                modifier = Modifier.size(12.dp),
            )
        }
    }
}

private object SelectedImageThumbnailCache {
    private const val MaxCacheKb = 8 * 1024

    private val cache =
        object : LruCache<String, Bitmap>(MaxCacheKb) {
            override fun sizeOf(
                key: String,
                value: Bitmap,
            ): Int = (value.byteCount / 1024).coerceAtLeast(1)
        }

    fun get(key: String): Bitmap? = cache.get(key)

    fun put(
        key: String,
        bitmap: Bitmap,
    ) {
        cache.put(key, bitmap)
    }
}

private fun decodeStagedImageThumbnail(path: String): Bitmap? {
    val file = File(path)
    if (!file.exists()) {
        return null
    }
    val bounds =
        BitmapFactory.Options().apply {
            inJustDecodeBounds = true
        }
    BitmapFactory.decodeFile(path, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
        return null
    }
    val options =
        BitmapFactory.Options().apply {
            inSampleSize = chatAttachmentPreviewSampleSize(bounds.outWidth, bounds.outHeight)
        }
    return BitmapFactory.decodeFile(path, options)
}

internal data class PickedAttachment(
    val path: String,
    val filename: String,
)

internal data class PendingCameraImage(
    val uri: Uri,
    val attachment: PickedAttachment,
)

private enum class ChatAttachmentType(
    val label: String,
    val icon: ImageVector,
) {
    IMAGE("Image", Icons.Rounded.Image),
    VIDEO("Video", Icons.Rounded.Movie),
    AUDIO("Audio", Icons.Rounded.Audiotrack),
    ARCHIVE("Archive", Icons.Rounded.Archive),
    DOCUMENT("Document", Icons.Rounded.Description),
    FILE("File", IrisIcons.File),
}

private val chatImageExtensions = setOf(
    "gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif",
)
private val chatVideoExtensions = setOf("avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts")
private val chatAudioExtensions = setOf("aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma")
private val chatArchiveExtensions = setOf("7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip")
private val chatDocumentExtensions = setOf(
    "csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml",
)

private fun attachmentType(attachment: PickedAttachment): ChatAttachmentType =
    attachmentType(attachment.filename)

private fun attachmentType(attachment: MessageAttachmentSnapshot): ChatAttachmentType {
    if (attachment.isImage) {
        return ChatAttachmentType.IMAGE
    }
    if (attachment.isVideo) {
        return ChatAttachmentType.VIDEO
    }
    if (attachment.isAudio) {
        return ChatAttachmentType.AUDIO
    }
    return attachmentType(attachment.filename)
}

private fun attachmentType(filename: String): ChatAttachmentType {
    val extension = filename.substringAfterLast(".", "").trim().lowercase()
    if (extension.isEmpty()) {
        return ChatAttachmentType.FILE
    }
    if (chatImageExtensions.contains(extension)) {
        return ChatAttachmentType.IMAGE
    }
    if (chatVideoExtensions.contains(extension)) {
        return ChatAttachmentType.VIDEO
    }
    if (chatAudioExtensions.contains(extension)) {
        return ChatAttachmentType.AUDIO
    }
    if (chatArchiveExtensions.contains(extension)) {
        return ChatAttachmentType.ARCHIVE
    }
    if (chatDocumentExtensions.contains(extension)) {
        return ChatAttachmentType.DOCUMENT
    }
    return ChatAttachmentType.FILE
}

private const val ChatAttachmentsLogTag = "IrisChat"

internal fun copyAttachmentToCache(
    context: Context,
    uri: Uri,
): PickedAttachment? {
    val resolver = context.contentResolver
    val displayName = displayNameForUri(context, uri)
    val outputDir = File(context.cacheDir, "attachments/outgoing").apply { mkdirs() }
    val outputFile = File(outputDir, "${UUID.randomUUID()}-$displayName")

    return runCatching {
        resolver.openInputStream(uri)?.use { input ->
            outputFile.outputStream().use { output ->
                input.copyTo(output)
            }
        } ?: return null
        PickedAttachment(outputFile.absolutePath, displayName)
    }.onFailure { error ->
        Log.w(ChatAttachmentsLogTag, "failed to copy attachment", error)
    }.getOrNull()
}

internal fun createPendingCameraImage(context: Context): PendingCameraImage? {
    val outputDir = File(context.cacheDir, "attachments/outgoing").apply { mkdirs() }
    val displayName = "photo-${UUID.randomUUID()}.jpg"
    val outputFile = File(outputDir, displayName)
    return runCatching {
        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                outputFile,
            )
        PendingCameraImage(uri, PickedAttachment(outputFile.absolutePath, displayName))
    }.onFailure { error ->
        Log.w(ChatAttachmentsLogTag, "failed to create camera image", error)
    }.getOrNull()
}

private fun displayNameForUri(
    context: Context,
    uri: Uri,
): String {
    val queried =
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
            }
    return safeAttachmentName(queried ?: uri.lastPathSegment ?: "attachment")
}

private fun safeAttachmentName(value: String): String {
    val basename = value.substringAfterLast('/').substringAfterLast('\\').trim()
    return basename.ifEmpty { "attachment" }
}

@Composable
internal fun ChatImageAlbumView(
    attachments: List<MessageAttachmentSnapshot>,
    isOutgoing: Boolean,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, MessageAttachmentSnapshot) -> Unit,
    onForward: (MessageAttachmentSnapshot) -> Unit,
) {
    val albumWidth = 232.dp
    val gap = 2.dp
    when (attachments.size) {
        0 -> Unit
        1 -> AlbumCell(
            attachment = attachments[0],
            isOutgoing = isOutgoing,
            width = 220.dp,
            height = 150.dp,
            downloadAttachment = downloadAttachment,
            onOpenImage = onOpenImage,
            onForward = onForward,
        )
        2 -> Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
            val cell = (albumWidth - gap) / 2
            AlbumCell(attachments[0], isOutgoing, cell, 150.dp, downloadAttachment, onOpenImage, onForward)
            AlbumCell(attachments[1], isOutgoing, cell, 150.dp, downloadAttachment, onOpenImage, onForward)
        }
        3 -> Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
            val left = albumWidth * 0.58f - gap / 2
            val right = albumWidth * 0.42f - gap / 2
            val tall = albumWidth * 0.86f
            val small = (tall - gap) / 2
            AlbumCell(attachments[0], isOutgoing, left, tall, downloadAttachment, onOpenImage, onForward)
            Column(verticalArrangement = Arrangement.spacedBy(gap)) {
                AlbumCell(attachments[1], isOutgoing, right, small, downloadAttachment, onOpenImage, onForward)
                AlbumCell(attachments[2], isOutgoing, right, small, downloadAttachment, onOpenImage, onForward)
            }
        }
        else -> {
            val cell = (albumWidth - gap) / 2
            Column(verticalArrangement = Arrangement.spacedBy(gap)) {
                Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
                    AlbumCell(attachments[0], isOutgoing, cell, cell, downloadAttachment, onOpenImage, onForward)
                    AlbumCell(attachments[1], isOutgoing, cell, cell, downloadAttachment, onOpenImage, onForward)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
                    AlbumCell(attachments[2], isOutgoing, cell, cell, downloadAttachment, onOpenImage, onForward)
                    Box {
                        AlbumCell(attachments[3], isOutgoing, cell, cell, downloadAttachment, onOpenImage, onForward)
                        if (attachments.size > 4) {
                            Box(
                                modifier = Modifier
                                    .size(cell)
                                    .clip(RoundedCornerShape(4.dp))
                                    .background(Color.Black.copy(alpha = 0.45f)),
                                contentAlignment = Alignment.Center,
                            ) {
                                Text(
                                    text = "+${attachments.size - 4}",
                                    style = MaterialTheme.typography.headlineSmall,
                                    color = Color.White,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun AlbumCell(
    attachment: MessageAttachmentSnapshot,
    isOutgoing: Boolean,
    width: androidx.compose.ui.unit.Dp,
    height: androidx.compose.ui.unit.Dp,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, MessageAttachmentSnapshot) -> Unit,
    onForward: (MessageAttachmentSnapshot) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val haptics = rememberIrisHapticFeedback()
    val clipboard = rememberIrisClipboard()
    var localImageData by remember(attachment.htreeUrl) { mutableStateOf<ByteArray?>(null) }
    var localPreviewBitmap by remember(attachment.htreeUrl) {
        mutableStateOf(ChatAttachmentPreviewBitmapCache.get(attachment.htreeUrl))
    }
    var imageLoadFailed by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var imageLoading by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var actionsOpen by remember(attachment.htreeUrl) { mutableStateOf(false) }
    val interactionSource = remember(attachment.htreeUrl) { MutableInteractionSource() }
    val foreground = if (isOutgoing) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface

    suspend fun loadImageIfNeeded(): ByteArray? {
        localImageData?.let { return it }
        if (imageLoading) return null
        imageLoading = true
        imageLoadFailed = false
        if (localPreviewBitmap == null) {
            localPreviewBitmap = ChatAttachmentPreviewBitmapCache.get(attachment.htreeUrl)
        }
        val data = downloadAttachment(attachment)
        imageLoading = false
        if (data == null) {
            imageLoadFailed = true
            return null
        }
        if (!isAnimatedImage(data, attachment.filename) && localPreviewBitmap == null) {
            val preview = decodeChatAttachmentPreviewBitmap(data)
            if (preview == null) {
                imageLoadFailed = true
                return null
            }
            ChatAttachmentPreviewBitmapCache.put(attachment.htreeUrl, preview)
            localPreviewBitmap = preview
        }
        localImageData = data
        return data
    }

    LaunchedEffect(attachment.htreeUrl) {
        loadImageIfNeeded()
    }

    if (actionsOpen) {
        AttachmentActionsSheet(
            attachment = attachment,
            onDismiss = { actionsOpen = false },
            onForward = {
                actionsOpen = false
                onForward(attachment)
            },
            onCopy = {
                actionsOpen = false
                clipboard.setText(attachment.filename, attachment.htreeUrl)
            },
        )
    }

    val isAnimated = remember(localImageData, attachment.filename) {
        localImageData?.let { isAnimatedImage(it, attachment.filename) } ?: isLikelyGif(attachment.filename)
    }

    Box(
        modifier = Modifier
            .size(width = width, height = height)
            .clip(RoundedCornerShape(4.dp))
            .background(foreground.copy(alpha = 0.12f))
            .combinedClickable(
                interactionSource = interactionSource,
                indication = null,
                onLongClick = {
                    haptics.press()
                    actionsOpen = true
                },
            ) {
                haptics.press()
                val data = localImageData
                if (data != null) {
                    onOpenImage(data, attachment)
                } else {
                    scope.launch {
                        loadImageIfNeeded()?.let { loadedData ->
                            onOpenImage(loadedData, attachment)
                        }
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        val preview = localPreviewBitmap
        when {
            preview != null -> Image(
                bitmap = preview.asImageBitmap(),
                contentDescription = attachment.filename,
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop,
            )
            isAnimated && localImageData != null -> AnimatedImageDataView(
                data = localImageData!!,
                modifier = Modifier.fillMaxSize(),
            )
            imageLoading -> CircularProgressIndicator(
                modifier = Modifier.size(22.dp),
                strokeWidth = 2.dp,
                color = foreground,
            )
            else -> Icon(
                imageVector = if (imageLoadFailed) Icons.Rounded.Warning else IrisIcons.Image,
                contentDescription = null,
                tint = foreground.copy(alpha = 0.72f),
                modifier = Modifier.size(26.dp),
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3Api::class)
@Composable
internal fun AttachmentChip(
    attachment: MessageAttachmentSnapshot,
    isOutgoing: Boolean,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, MessageAttachmentSnapshot) -> Unit,
    onForward: () -> Unit,
) {
    val context = LocalContext.current
    val clipboard = rememberIrisClipboard()
    val scope = rememberCoroutineScope()
    val haptics = rememberIrisHapticFeedback()
    val imageInteractionSource = remember(attachment.htreeUrl) { MutableInteractionSource() }
    val attachmentInteractionSource = remember(attachment.htreeUrl) { MutableInteractionSource() }
    var localImageData by remember(attachment.htreeUrl) { mutableStateOf<ByteArray?>(null) }
    var localPreviewBitmap by remember(attachment.htreeUrl) {
        mutableStateOf(ChatAttachmentPreviewBitmapCache.get(attachment.htreeUrl))
    }
    var imageLoadFailed by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var imageLoading by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var attachmentOpening by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var actionsOpen by remember(attachment.htreeUrl) { mutableStateOf(false) }
    val foreground =
        if (isOutgoing) {
            MaterialTheme.colorScheme.onPrimary
        } else {
            MaterialTheme.colorScheme.onSurface
        }
    val type = attachmentType(attachment)

    if (actionsOpen) {
        AttachmentActionsSheet(
            attachment = attachment,
            onDismiss = { actionsOpen = false },
            onForward = {
                actionsOpen = false
                onForward()
            },
            onCopy = {
                actionsOpen = false
                clipboard.setText(attachment.filename, attachment.htreeUrl)
            },
        )
    }

    suspend fun loadImageIfNeeded(): ByteArray? {
        localImageData?.let { return it }
        if (!attachment.isImage || imageLoading) {
            return null
        }
        imageLoading = true
        imageLoadFailed = false
        if (localPreviewBitmap == null) {
            localPreviewBitmap = ChatAttachmentPreviewBitmapCache.get(attachment.htreeUrl)
        }
        val data = downloadAttachment(attachment)
        imageLoading = false
        if (data == null) {
            imageLoadFailed = true
            return null
        }
        if (!isAnimatedImage(data, attachment.filename) && localPreviewBitmap == null) {
            val preview = decodeChatAttachmentPreviewBitmap(data)
            if (preview == null) {
                imageLoadFailed = true
                return null
            }
            ChatAttachmentPreviewBitmapCache.put(attachment.htreeUrl, preview)
            localPreviewBitmap = preview
        }
        localImageData = data
        return data
    }

    if (attachment.isImage) {
        LaunchedEffect(attachment.htreeUrl) {
            loadImageIfNeeded()
        }
        val isAnimated = remember(localImageData, attachment.filename) {
            localImageData?.let { data -> isAnimatedImage(data, attachment.filename) } ?: isLikelyGif(attachment.filename)
        }
        Box(
            modifier =
                Modifier
                    .size(width = 220.dp, height = 150.dp)
                    .clip(RoundedCornerShape(16.dp))
                    .background(foreground.copy(alpha = 0.12f))
                    .combinedClickable(
                        interactionSource = imageInteractionSource,
                        indication = null,
                        onLongClick = {
                            haptics.press()
                            actionsOpen = true
                        },
                    ) {
                        haptics.press()
                        val data = localImageData
                        if (data != null) {
                            onOpenImage(data, attachment)
                        } else {
                            scope.launch {
                                loadImageIfNeeded()?.let { loadedData ->
                                    onOpenImage(loadedData, attachment)
                                }
                            }
                        }
                    },
            contentAlignment = Alignment.Center,
        ) {
            if (localPreviewBitmap != null) {
                Image(
                    bitmap = localPreviewBitmap!!.asImageBitmap(),
                    contentDescription = attachment.filename,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop,
                )
            } else if (isAnimated && localImageData != null) {
                AnimatedImageDataView(
                    data = localImageData!!,
                    modifier = Modifier.fillMaxSize(),
                )
            } else if (imageLoading) {
                CircularProgressIndicator(
                    modifier = Modifier.size(22.dp),
                    strokeWidth = 2.dp,
                    color = foreground,
                )
            } else {
                Icon(
                    imageVector = if (imageLoadFailed) Icons.Rounded.Warning else IrisIcons.Image,
                    contentDescription = null,
                    tint = foreground.copy(alpha = 0.72f),
                    modifier = Modifier.size(30.dp),
                )
            }
        }
        return
    }

    Row(
        modifier =
            Modifier
                .semantics { contentDescription = "${type.label}, ${attachment.filename}" }
                .clip(RoundedCornerShape(12.dp))
                .background(foreground.copy(alpha = 0.12f))
                .combinedClickable(
                    interactionSource = attachmentInteractionSource,
                    indication = null,
                    onClick = {
                        if (attachmentOpening) {
                            return@combinedClickable
                        }
                        haptics.press()
                        scope.launch {
                            attachmentOpening = true
                            val data = downloadAttachment(attachment)
                            val opened = data?.let {
                                openDownloadedAttachment(context, attachment, it)
                            } ?: false
                            attachmentOpening = false
                            if (!opened) {
                                clipboard.setText(attachment.filename, attachment.htreeUrl)
                            }
                        }
                    },
                    onLongClick = {
                        haptics.press()
                        actionsOpen = true
                    },
                )
                .padding(horizontal = 10.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (attachmentOpening) {
            CircularProgressIndicator(
                modifier = Modifier.size(20.dp),
                strokeWidth = 2.dp,
                color = foreground,
            )
        } else {
            Icon(
                imageVector = type.icon,
                contentDescription = null,
                tint = foreground,
                modifier = Modifier.size(20.dp),
            )
        }
        Column(
            modifier = Modifier.widthIn(max = 220.dp),
            verticalArrangement = Arrangement.spacedBy(1.dp),
        ) {
            Text(
                text = attachment.filename,
                style = MaterialTheme.typography.labelLarge,
                color = foreground,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = type.label,
                style = MaterialTheme.typography.labelSmall,
                color = foreground.copy(alpha = 0.72f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AttachmentActionsSheet(
    attachment: MessageAttachmentSnapshot,
    onDismiss: () -> Unit,
    onForward: () -> Unit,
    onCopy: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = attachment.filename.ifBlank { "Attachment" },
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )
            AttachmentActionRow(
                icon = IrisIcons.Share,
                label = "Forward",
                onClick = onForward,
            )
            AttachmentActionRow(
                icon = IrisIcons.Copy,
                label = "Copy link",
                onClick = onCopy,
            )
        }
    }
}

@Composable
private fun AttachmentActionRow(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Row(
        modifier =
            Modifier
                .clip(RoundedCornerShape(12.dp))
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                ) {
                    haptics.press()
                    onClick()
                }
                .padding(horizontal = 12.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.size(20.dp),
        )
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

private object ChatAttachmentPreviewBitmapCache {
    private const val MaxCacheKb = 48 * 1024

    private val cache =
        object : LruCache<String, Bitmap>(MaxCacheKb) {
            override fun sizeOf(
                key: String,
                value: Bitmap,
            ): Int = (value.byteCount / 1024).coerceAtLeast(1)
        }

    fun get(key: String): Bitmap? = cache.get(key)

    fun put(
        key: String,
        bitmap: Bitmap,
    ) {
        cache.put(key, bitmap)
    }
}

private suspend fun decodeChatAttachmentPreviewBitmap(data: ByteArray): Bitmap? =
    withContext(Dispatchers.Default) {
        val bounds =
            BitmapFactory.Options().apply {
                inJustDecodeBounds = true
            }
        BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            return@withContext null
        }
        val options =
            BitmapFactory.Options().apply {
                inSampleSize = chatAttachmentPreviewSampleSize(bounds.outWidth, bounds.outHeight)
            }
        BitmapFactory.decodeByteArray(data, 0, data.size, options)
    }

private fun chatAttachmentPreviewSampleSize(
    width: Int,
    height: Int,
): Int {
    val maxPreviewPixels = 512
    var sampleSize = 1
    while (width / (sampleSize * 2) >= maxPreviewPixels || height / (sampleSize * 2) >= maxPreviewPixels) {
        sampleSize *= 2
    }
    return sampleSize
}

private fun openDownloadedAttachment(
    context: Context,
    attachment: MessageAttachmentSnapshot,
    data: ByteArray,
): Boolean =
    runCatching {
        val outputDir = File(context.cacheDir, "attachments/downloaded").apply { mkdirs() }
        val outputFile = File(outputDir, attachmentCacheName(attachment.nhash, attachment.filename))
        outputFile.writeBytes(data)
        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                outputFile,
            )
        val intent =
            Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, mimeTypeForFilename(attachment.filename))
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        context.startActivity(Intent.createChooser(intent, attachment.filename))
        true
    }.onFailure { error ->
        Log.w(ChatAttachmentsLogTag, "failed to open attachment", error)
    }.getOrDefault(false)

private fun attachmentCacheName(
    nhash: String,
    filename: String,
): String = "${safeAttachmentCacheComponent(nhash)}-${safeAttachmentCacheComponent(filename)}"

private fun safeAttachmentCacheComponent(value: String): String =
    value
        .split('/', '\\', ':')
        .joinToString("-")
        .trim()
        .ifEmpty { "attachment" }

private fun mimeTypeForFilename(filename: String): String {
    val extension = filename.substringAfterLast('.', "").lowercase()
    return extension
        .takeIf { it.isNotBlank() }
        ?.let { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) }
        ?: "application/octet-stream"
}

internal data class DownloadedImageAttachment(
    val data: ByteArray,
    val filename: String,
)

internal data class ImageViewerItem(
    val attachments: List<MessageAttachmentSnapshot>,
    val initialIndex: Int,
    val initialData: ByteArray,
    val senderName: String,
    val createdAtSecs: Long,
)

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun ImageViewerDialog(
    item: ImageViewerItem,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onForward: (MessageAttachmentSnapshot) -> Unit,
    onDismiss: () -> Unit,
) {
    val focusRequester = remember { FocusRequester() }
    val haptics = rememberIrisHapticFeedback()
    val scope = rememberCoroutineScope()
    val pagerState = rememberPagerState(
        initialPage = item.initialIndex.coerceIn(0, (item.attachments.size - 1).coerceAtLeast(0)),
    ) { item.attachments.size }

    val loadedData = remember { mutableStateMapOf<String, ByteArray>() }
    val decodedBitmaps = remember { mutableStateMapOf<String, Bitmap>() }
    val density = LocalDensity.current
    val configuration = LocalConfiguration.current
    val screenHeightPx = with(density) { configuration.screenHeightDp.dp.toPx() }
    val dismissThresholdPx = with(density) { 140.dp.toPx() }
    var dragOffsetY by remember { mutableFloatStateOf(0f) }

    LaunchedEffect(item.attachments) {
        item.attachments.getOrNull(item.initialIndex)?.let { initial ->
            loadedData[initial.htreeUrl] = item.initialData
            if (!isAnimatedImage(item.initialData, initial.filename)) {
                BitmapFactory.decodeByteArray(item.initialData, 0, item.initialData.size)?.let { bmp ->
                    decodedBitmaps[initial.htreeUrl] = bmp
                }
            }
        }
    }
    LaunchedEffect(pagerState.currentPage, item.attachments) {
        for (offset in listOf(0, -1, 1)) {
            val idx = pagerState.currentPage + offset
            val attachment = item.attachments.getOrNull(idx) ?: continue
            if (loadedData[attachment.htreeUrl] != null) continue
            val data = withContext(Dispatchers.IO) { downloadAttachment(attachment) } ?: continue
            loadedData[attachment.htreeUrl] = data
            if (!isAnimatedImage(data, attachment.filename) && decodedBitmaps[attachment.htreeUrl] == null) {
                withContext(Dispatchers.Default) {
                    BitmapFactory.decodeByteArray(data, 0, data.size)
                }?.let { bmp ->
                    decodedBitmaps[attachment.htreeUrl] = bmp
                }
            }
        }
    }
    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }

    val fadeFraction = (kotlin.math.abs(dragOffsetY) / screenHeightPx).coerceIn(0f, 1f)
    val backdropAlpha = 0.94f * (1f - fadeFraction * 0.55f)
    val chromeAlpha = (1f - fadeFraction * 2.2f).coerceIn(0f, 1f)
    val animatedOffset by animateFloatAsState(
        targetValue = dragOffsetY,
        animationSpec = if (dragOffsetY == 0f) tween(durationMillis = 220) else snap(),
        label = "viewerDragOffset",
    )

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = backdropAlpha))
                    .focusRequester(focusRequester)
                    .focusable()
                    .onPreviewKeyEvent { event ->
                        if (event.type != KeyEventType.KeyUp) {
                            return@onPreviewKeyEvent false
                        }
                        when (event.key) {
                            Key.Escape -> {
                                onDismiss()
                                true
                            }
                            Key.DirectionLeft -> {
                                if (pagerState.currentPage > 0) {
                                    scope.launch { pagerState.animateScrollToPage(pagerState.currentPage - 1) }
                                }
                                true
                            }
                            Key.DirectionRight -> {
                                if (pagerState.currentPage < item.attachments.lastIndex) {
                                    scope.launch { pagerState.animateScrollToPage(pagerState.currentPage + 1) }
                                }
                                true
                            }
                            else -> false
                        }
                    },
        ) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .offset { IntOffset(0, animatedOffset.toInt()) }
                    .pointerInput(Unit) {
                        detectVerticalDragGestures(
                            onDragEnd = {
                                if (kotlin.math.abs(dragOffsetY) > dismissThresholdPx) {
                                    onDismiss()
                                } else {
                                    dragOffsetY = 0f
                                }
                            },
                            onDragCancel = {
                                dragOffsetY = 0f
                            },
                            onVerticalDrag = { _, delta ->
                                dragOffsetY += delta
                            },
                        )
                    },
                contentAlignment = Alignment.Center,
            ) {
                HorizontalPager(
                    state = pagerState,
                    modifier = Modifier.fillMaxSize(),
                ) { page ->
                    val attachment = item.attachments[page]
                    val data = loadedData[attachment.htreeUrl]
                    val bitmap = decodedBitmaps[attachment.htreeUrl]
                    ImageViewerPage(
                        data = data,
                        bitmap = bitmap,
                        filename = attachment.filename,
                        onTap = {
                            haptics.press()
                            onDismiss()
                        },
                    )
                }
            }

            ImageViewerTopChrome(
                senderName = item.senderName,
                createdAtSecs = item.createdAtSecs,
                onClose = onDismiss,
                modifier = Modifier
                    .align(Alignment.TopCenter)
                    .alpha(chromeAlpha),
            )

            val shareContext = LocalContext.current
            ImageViewerBottomChrome(
                attachmentCount = item.attachments.size,
                currentIndex = pagerState.currentPage,
                onShare = {
                    val attachment = item.attachments.getOrNull(pagerState.currentPage) ?: return@ImageViewerBottomChrome
                    val data = loadedData[attachment.htreeUrl] ?: return@ImageViewerBottomChrome
                    shareImageAttachment(
                        shareContext,
                        DownloadedImageAttachment(data = data, filename = attachment.filename),
                    )
                },
                onForward = {
                    val attachment = item.attachments.getOrNull(pagerState.currentPage) ?: return@ImageViewerBottomChrome
                    onForward(attachment)
                },
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .alpha(chromeAlpha),
            )
        }
    }
}

@Composable
private fun ImageViewerTopChrome(
    senderName: String,
    createdAtSecs: Long,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .padding(horizontal = 12.dp, vertical = 6.dp),
    ) {
        GlassCircleIconButton(
            icon = IrisIcons.ChevronLeft,
            contentDescription = "Close image",
            onClick = onClose,
            modifier = Modifier.align(Alignment.CenterStart),
        )
        Column(
            modifier = Modifier.align(Alignment.Center),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = senderName,
                style = MaterialTheme.typography.labelLarge,
                color = Color.White,
                maxLines = 1,
            )
            Text(
                text = formatImageViewerDate(createdAtSecs),
                style = MaterialTheme.typography.labelSmall,
                color = Color.White.copy(alpha = 0.72f),
                maxLines = 1,
            )
        }
    }
}

@Composable
private fun ImageViewerBottomChrome(
    attachmentCount: Int,
    currentIndex: Int,
    onShare: () -> Unit,
    onForward: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .padding(horizontal = 20.dp, vertical = 14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (attachmentCount > 1) {
            PageIndicator(count = attachmentCount, current = currentIndex)
        }
        Row(modifier = Modifier.fillMaxWidth()) {
            GlassCircleIconButton(
                icon = IrisIcons.Share,
                contentDescription = "Share image",
                onClick = onShare,
            )
            Spacer(modifier = Modifier.weight(1f))
            GlassCircleIconButton(
                icon = IrisIcons.Forward,
                contentDescription = "Forward image",
                onClick = onForward,
            )
        }
    }
}

@Composable
private fun PageIndicator(count: Int, current: Int) {
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(percent = 50))
            .background(Color.Black.copy(alpha = 0.42f))
            .padding(horizontal = 10.dp, vertical = 5.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        repeat(count) { idx ->
            Box(
                modifier = Modifier
                    .size(6.dp)
                    .clip(androidx.compose.foundation.shape.CircleShape)
                    .background(
                        if (idx == current) Color.White.copy(alpha = 0.95f)
                        else Color.White.copy(alpha = 0.38f)
                    ),
            )
        }
    }
}

@Composable
private fun GlassCircleIconButton(
    icon: ImageVector,
    contentDescription: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .size(44.dp)
            .clip(androidx.compose.foundation.shape.CircleShape)
            .background(Color.White.copy(alpha = 0.18f))
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = contentDescription,
            tint = Color.White,
            modifier = Modifier.size(22.dp),
        )
    }
}

private val imageViewerDateFormatter = java.text.SimpleDateFormat("MMM d, h:mm a", java.util.Locale.getDefault())

private fun formatImageViewerDate(secs: Long): String =
    imageViewerDateFormatter.format(java.util.Date(secs * 1000L))

@Composable
private fun ImageViewerPage(
    data: ByteArray?,
    bitmap: Bitmap?,
    filename: String,
    onTap: () -> Unit,
) {
    val interactionSource = remember(filename) { MutableInteractionSource() }
    val isAnimated = remember(data, filename) {
        data?.let { isAnimatedImage(it, filename) } ?: false
    }
    Box(
        modifier = Modifier
            .fillMaxSize()
            .clickable(
                interactionSource = interactionSource,
                indication = null,
                onClick = onTap,
            ),
        contentAlignment = Alignment.Center,
    ) {
        when {
            data == null -> CircularProgressIndicator(color = Color.White)
            isAnimated -> AnimatedImageDataView(
                data = data,
                modifier = Modifier
                    .fillMaxSize()
                    .padding(18.dp),
            )
            bitmap != null -> Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = filename,
                modifier = Modifier
                    .fillMaxSize()
                    .padding(18.dp),
                contentScale = ContentScale.Fit,
            )
            else -> CircularProgressIndicator(color = Color.White)
        }
    }
}

private fun shareImageAttachment(
    context: Context,
    item: DownloadedImageAttachment,
) {
    runCatching {
        val outputDir = File(context.cacheDir, "attachments/share").apply { mkdirs() }
        val safeName = safeAttachmentCacheComponent(item.filename.ifBlank { "image" })
        val outputFile = File(outputDir, "${UUID.randomUUID()}-$safeName")
        outputFile.writeBytes(item.data)
        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                outputFile,
            )
        val intent =
            Intent(Intent.ACTION_SEND).apply {
                type = mimeTypeForFilename(item.filename)
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        context.startActivity(Intent.createChooser(intent, item.filename))
    }.onFailure { error ->
        Log.w(ChatAttachmentsLogTag, "failed to share image", error)
    }
}

@Composable
private fun AnimatedImageDataView(
    data: ByteArray,
    modifier: Modifier = Modifier,
) {
    val html = remember(data) { animatedImageHtml(data) }
    AndroidView(
        modifier = modifier,
        factory = { context ->
            WebView(context).apply {
                setBackgroundColor(android.graphics.Color.TRANSPARENT)
                settings.javaScriptEnabled = false
                isVerticalScrollBarEnabled = false
                isHorizontalScrollBarEnabled = false
                loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
            }
        },
        update = { webView ->
            webView.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
        },
    )
}

private fun animatedImageHtml(data: ByteArray): String {
    val encoded = Base64.encodeToString(data, Base64.NO_WRAP)
    return """
        <!doctype html>
        <html>
        <head>
        <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1">
        <style>
        html, body {
          margin: 0;
          width: 100%;
          height: 100%;
          overflow: hidden;
          background: transparent;
        }
        body {
          display: flex;
          align-items: center;
          justify-content: center;
        }
        img {
          width: 100%;
          height: 100%;
          object-fit: contain;
        }
        </style>
        </head>
        <body><img src="data:image/gif;base64,$encoded" alt=""></body>
        </html>
    """.trimIndent()
}

private fun isLikelyGif(filename: String): Boolean =
    filename.endsWith(".gif", ignoreCase = true)

private fun isAnimatedImage(
    data: ByteArray,
    filename: String,
): Boolean =
    isLikelyGif(filename) ||
        data.take(6).toByteArray().contentEquals("GIF87a".toByteArray()) ||
        data.take(6).toByteArray().contentEquals("GIF89a".toByteArray())
