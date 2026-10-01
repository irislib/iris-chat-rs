package to.iris.chat.ui.screens

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.distinctUntilChanged

@Composable
internal fun ObserveChatTimelineViewport(
    listState: LazyListState,
    messageCount: Int,
    preserveAnchor: Boolean,
    onNearBottom: (Boolean) -> Unit,
) {
    val canPreserveAnchor by rememberUpdatedState(preserveAnchor)
    val updateNearBottom by rememberUpdatedState(onNearBottom)
    LaunchedEffect(listState, messageCount) {
        var previousViewport: ChatTimelineViewport? = null
        snapshotFlow {
            val layout = listState.layoutInfo
            ChatTimelineViewport(
                height = layout.viewportEndOffset - layout.viewportStartOffset,
                firstIndex = listState.firstVisibleItemIndex,
                firstOffset = listState.firstVisibleItemScrollOffset,
                nearBottom = messageCount == 0 || (layout.visibleItemsInfo.lastOrNull()?.index ?: -1) >= messageCount - 2,
            )
        }.distinctUntilChanged().collect { viewport ->
            val previous = previousViewport
            previousViewport = viewport
            if (
                canPreserveAnchor && previous != null && previous.height > 0 &&
                viewport.height > 0 && previous.height != viewport.height
            ) {
                // Initial navigation must finish before this observer can scroll.
                // Use the pre-resize anchor before geometry can disarm following
                // or clamp the old offset. Older messages move with the keyboard
                // too, without jumping to the latest message.
                if (previous.nearBottom && messageCount > 0) {
                    listState.scrollToItem(messageCount - 1)
                } else {
                    listState.scrollToItem(previous.firstIndex, previous.offsetAfterResize(viewport.height))
                }
            } else {
                updateNearBottom(viewport.nearBottom)
            }
        }
    }
}

internal data class ChatTimelineViewport(
    val height: Int,
    val firstIndex: Int,
    val firstOffset: Int,
    val nearBottom: Boolean,
) {
    // A negative offset is valid: LazyListState resolves it to earlier rows
    // when the keyboard closes and more of the conversation becomes visible.
    fun offsetAfterResize(newHeight: Int): Int = firstOffset + height - newHeight
}
