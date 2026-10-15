package to.iris.chat.ui.screens

import android.graphics.Bitmap
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File

@Composable
internal fun ComposerTestFrame(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.BottomCenter) { content() }
}

internal fun captureComposerTest(name: String) {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
        File(instrumentation.targetContext.cacheDir, "$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }
}
