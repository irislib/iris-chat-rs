package to.iris.chat.ui.screens

import android.content.Context
import android.view.inputmethod.InputContentInfo
import android.widget.Toast
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.TransferableContent
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One composer owns these copies until it sends them or discards its draft. */
@OptIn(ExperimentalFoundationApi::class)
internal class ChatAttachmentPaste(
    private val context: Context,
    private val scope: CoroutineScope,
) {
    private var generation = 0
    private var closed = false
    private var draftContext: Pair<Boolean, Boolean>? = null
    private val pending = mutableSetOf<Job>()
    private val staged = mutableSetOf<PickedAttachment>()

    @Composable
    fun receiverModifier(enabled: Boolean, sendDirectly: Boolean, onAttachments: (List<PickedAttachment>) -> Unit): Modifier {
        val callback by rememberUpdatedState(onAttachments)
        val expectedContext = enabled to sendDirectly
        SideEffect {
            if (draftContext != expectedContext) {
                invalidatePending()
                draftContext = expectedContext
            }
        }
        val listener = remember(this, expectedContext) {
            ReceiveContentListener { content ->
                receive(content, isCurrent = { enabled && draftContext == expectedContext }, onAttachments = callback,
                    onFailure = { Toast.makeText(context, "Couldn't paste attachment", Toast.LENGTH_SHORT).show() })
            }
        }
        return Modifier.contentReceiver(listener)
    }

    fun receive(
        content: TransferableContent,
        isCurrent: () -> Boolean,
        onAttachments: (List<PickedAttachment>) -> Unit,
        onFailure: () -> Unit,
    ): TransferableContent? {
        if (closed || !isCurrent()) return content
        val uris = droppedAttachmentUris(content.clipEntry.clipData)
        if (uris.isEmpty()) return content
        val receivingGeneration = generation
        val job = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            // Keep the payload (including the IME grant) alive throughout the copy.
            // Keep partial results outside withContext so cancellation can delete them.
            val copies = mutableListOf<PickedAttachment>()
            var delivered = false
            try {
                withContext(Dispatchers.IO) {
                    for (uri in uris) copySharedAttachmentToCache(context, uri)?.let(copies::add)
                }
                if (!closed && generation == receivingGeneration && isCurrent()) {
                    if (copies.size == uris.size) {
                        staged.addAll(copies)
                        onAttachments(copies)
                        delivered = true
                    } else onFailure()
                }
            } finally {
                withContext(NonCancellable + Dispatchers.IO) {
                    if (!delivered) copies.forEach { File(it.path).delete() }
                    releaseImeContent(content)
                }
            }
        }
        pending.add(job)
        job.invokeOnCompletion { pending.remove(job) }
        // A file clipboard may also contain labels or URLs. Keep the caption
        // untouched; only text-only payloads use normal text paste above.
        return null
    }

    fun invalidatePending() {
        generation++
        pending.toList().forEach { it.cancel() }
    }

    fun remove(attachment: PickedAttachment) {
        if (staged.remove(attachment)) File(attachment.path).delete()
    }

    fun sent(attachments: List<PickedAttachment>) {
        // The existing send pipeline now owns the files; never delete its sources.
        staged.removeAll(attachments.toSet())
        invalidatePending()
    }

    fun close() {
        closed = true
        invalidatePending()
        staged.toList().forEach(::remove)
    }
}

@Composable
internal fun rememberChatAttachmentPaste(chatId: String): ChatAttachmentPaste {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val paste = remember(context, chatId) { ChatAttachmentPaste(context, scope) }
    DisposableEffect(paste) { onDispose { paste.close() } }
    return paste
}

@OptIn(ExperimentalFoundationApi::class)
@Suppress("DEPRECATION")
private fun releaseImeContent(content: TransferableContent) {
    // Compose retains InputContentInfo in the platform extras after requesting
    // its temporary grant. Avoid depending on Compose's private bundle key.
    val extras = content.platformTransferableContent?.extras ?: return
    runCatching {
        extras.keySet().mapNotNull { extras.get(it) as? InputContentInfo }.distinct().forEach {
            runCatching { it.releasePermission() }
        }
    }
}
