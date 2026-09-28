package to.iris.chat.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import to.iris.chat.audio.AudioWaveform
import to.iris.chat.audio.VoiceMessagePlayback
import to.iris.chat.rust.MessageAttachmentSnapshot
import java.util.Locale

@Composable
internal fun ChatAudioMessage(
    attachment: MessageAttachmentSnapshot,
    color: Color,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onLongClick: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val currentDownload by rememberUpdatedState(downloadAttachment)
    val player = remember(attachment.htreeUrl) { VoiceMessagePlayback(context, scope) { currentDownload(attachment) } }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(player, lifecycle) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) player.pause() }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); player.close() }
    }
    Row(
        Modifier.width(244.dp).padding(vertical = 4.dp).combinedClickable(onClick = {}, onLongClick = onLongClick),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        val action = when { player.loading -> "Cancel loading"; player.error != null -> "Retry audio"; player.playing -> "Pause audio"; else -> "Play audio" }
        IconButton(onClick = player::toggle, modifier = Modifier.size(44.dp).background(color.copy(alpha = 0.12f), CircleShape).testTag("chatAudioPlayButton")) {
            Icon(when { player.loading -> Icons.Rounded.Close; player.error != null -> Icons.Rounded.Refresh; player.playing -> Icons.Rounded.Pause; else -> Icons.Rounded.PlayArrow }, action, tint = color)
        }
        Column(Modifier.weight(1f)) {
            AudioWaveformControl(player, color)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(player.error ?: if (player.duration > 0) "${audioTime(player.elapsed)} / ${audioTime(player.duration)}" else "Audio",
                    Modifier.weight(1f).testTag("chatAudioDuration"), color = color.copy(alpha = 0.7f), style = MaterialTheme.typography.labelSmall, maxLines = 2)
                TextButton(onClick = player::cycleRate, contentPadding = PaddingValues(horizontal = 6.dp),
                    modifier = Modifier.height(32.dp).widthIn(min = 40.dp).testTag("chatAudioSpeedButton").semantics {
                        contentDescription = "Playback speed"; stateDescription = "${player.rate}×"
                    }) {
                    Text("${player.rate.toString().removeSuffix(".0")}×", color = color,
                        modifier = Modifier.background(color.copy(alpha = 0.12f), RoundedCornerShape(12.dp)).padding(horizontal = 6.dp, vertical = 2.dp),
                        style = MaterialTheme.typography.labelSmall)
                }
            }
        }
    }
}

@Composable
private fun AudioWaveformControl(player: VoiceMessagePlayback, color: Color) {
    val enabled = player.duration > 0 && !player.loading && player.error == null
    Canvas(Modifier.fillMaxWidth().height(32.dp).testTag("chatAudioProgress")
        .semantics(mergeDescendants = true) {
            contentDescription = "Audio position"
            stateDescription = "${audioTime(player.elapsed)} of ${audioTime(player.duration)}"
            progressBarRangeInfo = ProgressBarRangeInfo(player.elapsed, 0f..player.duration.coerceAtLeast(1f))
            if (!enabled) disabled()
            setProgress { if (enabled) player.seek(it); enabled }
        }
        .onKeyEvent { event ->
            if (!enabled || event.type != KeyEventType.KeyDown) false else {
                val step = if (event.isShiftPressed) 5f else 1f
                when (event.key) {
                    Key.DirectionLeft, Key.DirectionDown -> { player.seek(player.elapsed - step); true }
                    Key.DirectionRight, Key.DirectionUp -> { player.seek(player.elapsed + step); true }
                    Key.MoveHome -> { player.seek(0f); true }
                    Key.MoveEnd -> { player.seek(player.duration); true }
                    else -> false
                }
            }
        }.focusable(enabled)
        .pointerInput(enabled, player.duration) {
            if (enabled) detectTapGestures { player.seek(((it.x - 4.dp.toPx()) / (size.width - 8.dp.toPx())).coerceIn(0f, 1f) * player.duration) }
        }
        .pointerInput(enabled, player.duration) {
            if (enabled) detectDragGestures(onDragStart = { player.seek(((it.x - 4.dp.toPx()) / (size.width - 8.dp.toPx())).coerceIn(0f, 1f) * player.duration) }) { change, _ ->
                change.consume()
                player.seek(((change.position.x - 4.dp.toPx()) / (size.width - 8.dp.toPx())).coerceIn(0f, 1f) * player.duration)
            }
        }) {
        val width = size.width - 8.dp.toPx()
        val step = width / AudioWaveform.BAR_COUNT
        val fraction = if (player.duration > 0) player.elapsed / player.duration else 0f
        repeat(AudioWaveform.BAR_COUNT) { i ->
            val height = (3 + (player.peaks.getOrNull(i) ?: 0f) * 21).dp.toPx()
            drawRoundRect(color.copy(alpha = if ((i + 0.5f) / AudioWaveform.BAR_COUNT <= fraction) 1f else 0.35f),
                Offset(4.dp.toPx() + i * step, (size.height - height) / 2), Size((step - 1.5.dp.toPx()).coerceAtLeast(1f), height), CornerRadius(1.dp.toPx()))
        }
        if (enabled) drawCircle(color, 3.dp.toPx(), Offset(4.dp.toPx() + fraction * width, size.height / 2))
    }
}

private fun audioTime(seconds: Float): String = "%d:%02d".format(Locale.ROOT, seconds.toInt() / 60, seconds.toInt() % 60)
