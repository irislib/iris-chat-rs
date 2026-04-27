package to.iris.chat.ui.components

import android.content.ClipData
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.Clipboard
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

@Composable
fun rememberIrisClipboard(): IrisClipboard {
    val clipboard = LocalClipboard.current
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    return remember(clipboard, context, coroutineScope) {
        IrisClipboard(
            clipboard = clipboard,
            context = context,
            coroutineScope = coroutineScope,
        )
    }
}

class IrisClipboard internal constructor(
    private val clipboard: Clipboard,
    private val context: Context,
    private val coroutineScope: CoroutineScope,
) {
    fun setText(
        label: String,
        text: String,
    ) {
        coroutineScope.launch {
            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(label, text)))
        }
    }

    fun getText(onText: (String) -> Unit) {
        coroutineScope.launch {
            val text =
                clipboard
                    .getClipEntry()
                    ?.clipData
                    ?.takeIf { it.itemCount > 0 }
                    ?.getItemAt(0)
                    ?.coerceToText(context)
                    ?.toString()
                    .orEmpty()
            onText(text)
        }
    }
}
