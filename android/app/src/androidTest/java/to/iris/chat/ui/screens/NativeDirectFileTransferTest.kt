package to.iris.chat.ui.screens

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.runDirectFileTransferSmoke

/** Run with calls.NativeCallTestRunner to avoid starting an account runtime. */
@RunWith(AndroidJUnit4::class)
class NativeDirectFileTransferTest {
    @Test
    fun nativeFipsTransfersMultipleFilesOnlyAfterAcceptance() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val json = runDirectFileTransferSmoke(context.cacheDir.absolutePath)
        val output = File(context.getExternalFilesDir("screenshots"), "native-direct-file-transfer.json")
        output.parentFile?.mkdirs()
        output.writeText(json)
        val evidence = JSONObject(json)
        assertTrue(json, evidence.getBoolean("ok"))
        assertEquals("fips-tcp", evidence.getString("transport"))
        assertEquals(3, evidence.getJSONArray("files").length())
        assertEquals(0L, evidence.getLong("bytes_before_accept"))
    }
}
