package social.innode.ndr.demo.ui.screens

import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import android.util.Log
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Archive
import androidx.compose.material.icons.rounded.Audiotrack
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import java.io.File
import java.util.UUID
import kotlinx.coroutines.launch
import social.innode.ndr.demo.rust.MessageAttachmentSnapshot
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

@Composable
internal fun SelectedAttachmentChip(
    attachment: PickedAttachment,
    enabled: Boolean,
    onRemove: () -> Unit,
) {
    val selectedAttachmentType = attachmentType(attachment)

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
                onClick = onRemove,
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

internal data class PickedAttachment(
    val path: String,
    val filename: String,
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

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun AttachmentChip(
    attachment: MessageAttachmentSnapshot,
    isOutgoing: Boolean,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, String) -> Unit,
) {
    val context = LocalContext.current
    val clipboard = rememberIrisClipboard()
    val scope = rememberCoroutineScope()
    var localImageData by remember(attachment.htreeUrl) { mutableStateOf<ByteArray?>(null) }
    var imageLoadFailed by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var imageLoading by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var attachmentOpening by remember(attachment.htreeUrl) { mutableStateOf(false) }
    val foreground =
        if (isOutgoing) {
            MaterialTheme.colorScheme.onPrimary
        } else {
            MaterialTheme.colorScheme.onSurface
        }
    val type = attachmentType(attachment)

    suspend fun loadImageIfNeeded(): ByteArray? {
        localImageData?.let { return it }
        if (!attachment.isImage || imageLoading) {
            return null
        }
        imageLoading = true
        imageLoadFailed = false
        val data = downloadAttachment(attachment)
        imageLoading = false
        if (data == null) {
            imageLoadFailed = true
            return null
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
        val bitmap = remember(localImageData) {
            localImageData
                ?.takeUnless { data -> isAnimatedImage(data, attachment.filename) }
                ?.let { data -> BitmapFactory.decodeByteArray(data, 0, data.size) }
        }
        Box(
            modifier =
                Modifier
                    .size(width = 220.dp, height = 150.dp)
                    .clip(RoundedCornerShape(16.dp))
                    .background(foreground.copy(alpha = 0.12f))
                    .clickable {
                        val data = localImageData
                        if (data != null) {
                            onOpenImage(data, attachment.filename)
                        } else {
                            scope.launch {
                                loadImageIfNeeded()?.let { loadedData ->
                                    onOpenImage(loadedData, attachment.filename)
                                }
                            }
                        }
                    },
            contentAlignment = Alignment.Center,
        ) {
            if (bitmap != null) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
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
                    onClick = {
                        if (attachmentOpening) {
                            return@combinedClickable
                        }
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
                        clipboard.setText(attachment.filename, attachment.htreeUrl)
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

@Composable
internal fun ImageViewerDialog(
    item: DownloadedImageAttachment,
    onDismiss: () -> Unit,
) {
    val focusRequester = remember { FocusRequester() }
    val bitmap = remember(item.data) {
        item.data
            .takeUnless { data -> isAnimatedImage(data, item.filename) }
            ?.let { data -> BitmapFactory.decodeByteArray(data, 0, data.size) }
    }
    val isAnimated = remember(item.data, item.filename) {
        isAnimatedImage(item.data, item.filename)
    }
    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.92f))
                    .clickable(onClick = onDismiss)
                    .focusRequester(focusRequester)
                    .focusable()
                    .onPreviewKeyEvent { event ->
                        if (event.key == Key.Escape && event.type == KeyEventType.KeyUp) {
                            onDismiss()
                            true
                        } else {
                            false
                        }
                    },
            contentAlignment = Alignment.Center,
        ) {
            if (isAnimated) {
                AnimatedImageDataView(
                    data = item.data,
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(18.dp),
                )
            } else if (bitmap != null) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = item.filename,
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(18.dp),
                    contentScale = ContentScale.Fit,
                )
            } else {
                CircularProgressIndicator(color = Color.White)
            }
            val shareContext = LocalContext.current
            IconButton(
                onClick = { shareImageAttachment(shareContext, item) },
                modifier = Modifier.align(Alignment.TopStart),
            ) {
                Icon(
                    imageVector = Icons.Rounded.Share,
                    contentDescription = "Share image",
                    tint = Color.White,
                )
            }
            IconButton(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.TopEnd),
            ) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Close image",
                    tint = Color.White,
                )
            }
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
