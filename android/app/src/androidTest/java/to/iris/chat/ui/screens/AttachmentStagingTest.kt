package to.iris.chat.ui.screens

import android.net.Uri
import androidx.core.content.FileProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.R

@RunWith(AndroidJUnit4::class)
class AttachmentStagingTest {
    @Test
    fun external_share_cannot_stage_a_private_file_uri() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val privateFile = File.createTempFile("private-share-test", ".txt", context.filesDir)
        try {
            privateFile.writeText("private account data")
            assertNull(copySharedAttachmentToCache(context, Uri.fromFile(privateFile)))
        } finally {
            privateFile.delete()
        }
    }

    @Test
    fun external_share_cannot_stage_an_unguarded_own_provider_uri() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        for (cache in listOf("downloaded", "outgoing")) {
            val directory = File(context.cacheDir, "attachments/$cache").apply { mkdirs() }
            val privateFile = File.createTempFile("private-share-test", ".txt", directory)
            try {
                privateFile.writeText("private attachment")
                val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", privateFile)
                assertNull(copySharedAttachmentToCache(context, uri))
                val userQualifiedUri = uri.buildUpon().authority("0@${uri.authority}").build()
                assertNull(copySharedAttachmentToCache(context, userQualifiedUri))
                val traversalUri = uri.buildUpon()
                    .path("/attachment_cache/share")
                    .appendPath("../$cache/${privateFile.name}")
                    .build()
                assertNull(copySharedAttachmentToCache(context, traversalUri))
            } finally {
                privateFile.delete()
            }
        }
    }

    @Test
    fun choosing_iris_for_an_explicitly_shared_image_still_stages_it() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val directory = File(context.cacheDir, "attachments/share").apply { mkdirs() }
        val sharedFile = File.createTempFile("share-test", ".txt", directory)
        var stagedFile: File? = null
        try {
            sharedFile.writeText("deliberately shared attachment")
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", sharedFile)
            val staged = checkNotNull(copySharedAttachmentToCache(context, uri))
            stagedFile = File(staged.path)
            assertArrayEquals(sharedFile.readBytes(), stagedFile.readBytes())
        } finally {
            sharedFile.delete()
            stagedFile?.delete()
        }
    }

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
