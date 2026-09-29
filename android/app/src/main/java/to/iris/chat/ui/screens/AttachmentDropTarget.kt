package to.iris.chat.ui.screens

import android.app.Activity
import android.content.ClipData
import android.content.Context
import android.content.ContextWrapper
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun droppedAttachmentUris(clip: ClipData) = (0 until clip.itemCount)
    .mapNotNull { clip.getItemAt(it).uri?.takeIf { uri -> uri.scheme == "content" } }.distinct()

private tailrec fun Context.dropActivity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.dropActivity()
    else -> null
}

/** Copy foreign content while its temporary drag grant is held; only stage a preview. */
@Composable
internal fun attachmentDropTarget(enabled: Boolean, onAttachments: (List<PickedAttachment>) -> Unit): Modifier {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val currentCallback = rememberUpdatedState(onAttachments)
    val currentEnabled = rememberUpdatedState(enabled)
    val target = remember(context) { object : DragAndDropTarget {
        override fun onDrop(event: DragAndDropEvent): Boolean {
            if (!currentEnabled.value) return false
            val androidEvent = event.toAndroidDragEvent()
            val uris = androidEvent.clipData?.let(::droppedAttachmentUris).orEmpty()
            if (uris.isEmpty()) return false
            val permission = context.dropActivity()?.requestDragAndDropPermissions(androidEvent)
            val callback = currentCallback.value
            scope.launch(start = CoroutineStart.UNDISPATCHED) {
                try {
                    val attachments = withContext(Dispatchers.IO) {
                        uris.mapNotNull { copySharedAttachmentToCache(context, it) }
                    }
                    if (currentEnabled.value && attachments.isNotEmpty()) callback(attachments)
                } finally { permission?.release() }
            }
            return true
        }
    } }
    return Modifier.dragAndDropTarget(shouldStartDragAndDrop = { currentEnabled.value }, target = target)
}
