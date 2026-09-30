package to.iris.chat.ui.components

import android.text.format.DateUtils
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.dp
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.abs

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
