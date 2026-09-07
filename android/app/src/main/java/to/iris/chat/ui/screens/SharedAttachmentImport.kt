package to.iris.chat.ui.screens

import android.content.ContentResolver
import android.content.Context
import android.net.Uri

/** Import an attachment supplied to the exported share activity by another app. */
internal fun copySharedAttachmentToCache(
    context: Context,
    uri: Uri,
): PickedAttachment? {
    // Never let another app use our permissions to copy a private file or one
    // of our own cached attachments into a new outgoing message.
    if (uri.scheme != ContentResolver.SCHEME_CONTENT) return null
    val authority = uri.authority?.substringAfterLast('@') ?: return null
    if (authority == "${context.packageName}.fileprovider") {
        // The image Share action deliberately stages a fresh copy here. Keep
        // choosing Iris in that chooser working without exposing other caches.
        val segments = uri.pathSegments
        if (segments.size != 3 || segments[0] != "attachment_cache" || segments[1] != "share") return null
        val filename = segments[2]
        if (filename.isEmpty() || filename == "." || filename == ".." || '/' in filename || '\\' in filename) return null
        return copyAttachmentToCache(context, uri)
    }
    val provider = context.packageManager.resolveContentProvider(authority, 0)
    if (provider?.applicationInfo?.uid == context.applicationInfo.uid) return null
    return copyAttachmentToCache(context, uri)
}
