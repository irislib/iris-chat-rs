package to.iris.chat.ui.screens

import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.R

@RunWith(AndroidJUnit4::class)
class AttachmentStagingTest {
    @Test
    fun bundled_iris_logo_is_copied_to_the_outgoing_attachment_cache() {
        val context =
            InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val logoUri = Uri.parse("android.resource://${context.packageName}/drawable/iris_logo")
        val expected = context.resources.openRawResource(R.drawable.iris_logo).use { it.readBytes() }
        val staged = checkNotNull(copyAttachmentToCache(context, logoUri))
        val stagedFile = File(staged.path)

        try {
            assertTrue(stagedFile.path.contains("/attachments/outgoing/"))
            assertArrayEquals(expected, stagedFile.readBytes())
        } finally {
            stagedFile.delete()
        }
    }
}
