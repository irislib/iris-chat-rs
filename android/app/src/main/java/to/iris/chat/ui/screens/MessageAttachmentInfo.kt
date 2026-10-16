package to.iris.chat.ui.screens

import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import java.net.URI
import to.iris.chat.rust.MessageAttachmentSnapshot

@Composable
internal fun MessageInfoAttachmentRows(attachments: List<MessageAttachmentSnapshot>) {
    attachments.forEach { attachment ->
        MessageInfoValueRow(
            label = if (attachment.filename.isBlank()) "File" else attachment.filename,
            value = attachment.htreeUrl,
            monospaced = true,
            copyValue = attachment.htreeUrl,
        )
        attachment.uploadedUrls.filter(::isAttachmentUploadUrl).forEachIndexed { index, url ->
            key(url) {
                MessageInfoValueRow(
                    label = if (index == 0) "Encrypted uploads" else "",
                    value = url,
                    copyValue = url,
                    link = true,
                )
            }
        }
    }
}

internal fun attachmentInfoTextLines(attachments: List<MessageAttachmentSnapshot>): List<String> =
    buildList {
        attachments.forEach { attachment ->
            add("- ${if (attachment.filename.isBlank()) "File" else attachment.filename} ${attachment.htreeUrl}")
            val uploads = attachment.uploadedUrls.filter(::isAttachmentUploadUrl)
            if (uploads.isNotEmpty()) {
                add("  Encrypted uploads: ${uploads.joinToString(", ")}")
            }
        }
    }

internal fun isAttachmentUploadUrl(value: String): Boolean = runCatching {
    val uri = URI(value)
    uri.scheme?.lowercase() in listOf("http", "https") && !uri.host.isNullOrBlank() &&
        uri.rawUserInfo == null && uri.rawQuery == null && uri.rawFragment == null
}.getOrDefault(false)
