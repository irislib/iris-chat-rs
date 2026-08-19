package to.iris.chat.ui.screens

import android.graphics.Bitmap
import android.graphics.Color
import androidx.exifinterface.media.ExifInterface
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

internal const val FIXTURE_RED = 0xFFE02020.toInt()
internal const val FIXTURE_GREEN = 0xFF20C040.toInt()
internal const val FIXTURE_BLUE = 0xFF2060E0.toInt()
internal const val FIXTURE_YELLOW = 0xFFF0D020.toInt()

internal fun expectedOrientationCorners(orientation: Int): IntArray =
    when (orientation) {
        1 -> intArrayOf(FIXTURE_RED, FIXTURE_GREEN, FIXTURE_BLUE, FIXTURE_YELLOW)
        2 -> intArrayOf(FIXTURE_GREEN, FIXTURE_RED, FIXTURE_YELLOW, FIXTURE_BLUE)
        3 -> intArrayOf(FIXTURE_YELLOW, FIXTURE_BLUE, FIXTURE_GREEN, FIXTURE_RED)
        4 -> intArrayOf(FIXTURE_BLUE, FIXTURE_YELLOW, FIXTURE_RED, FIXTURE_GREEN)
        5 -> intArrayOf(FIXTURE_RED, FIXTURE_BLUE, FIXTURE_GREEN, FIXTURE_YELLOW)
        6 -> intArrayOf(FIXTURE_BLUE, FIXTURE_RED, FIXTURE_YELLOW, FIXTURE_GREEN)
        7 -> intArrayOf(FIXTURE_YELLOW, FIXTURE_GREEN, FIXTURE_BLUE, FIXTURE_RED)
        8 -> intArrayOf(FIXTURE_GREEN, FIXTURE_YELLOW, FIXTURE_RED, FIXTURE_BLUE)
        else -> error("unsupported orientation $orientation")
    }

internal fun taggedOrientationFixture(orientation: Int): File {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val output =
        File(
            instrumentation.targetContext.cacheDir,
            "orientation-$orientation-${UUID.randomUUID()}.jpg",
        )
    instrumentation.context.assets.open("image-orientation/asymmetric.jpg").use { input ->
        output.outputStream().use(input::copyTo)
    }
    ExifInterface(output).apply {
        setAttribute(ExifInterface.TAG_ORIENTATION, orientation.toString())
        saveAttributes()
    }
    return output
}

@RunWith(AndroidJUnit4::class)
class ChatAttachmentImageDecoderTest {
    @Test
    fun byteArrayDecodeAppliesAllEightExifOrientations() {
        for (orientation in 1..8) {
            val fixture = taggedOrientationFixture(orientation)
            try {
                val bitmap =
                    checkNotNull(
                        decodeChatAttachmentImage(
                            fixture.readBytes(),
                            maxPixelSize = null,
                        ),
                    )
                val swapsDimensions = orientation >= 5
                assertEquals("orientation $orientation width", if (swapsDimensions) 200 else 320, bitmap.width)
                assertEquals("orientation $orientation height", if (swapsDimensions) 320 else 200, bitmap.height)
                assertBitmapCorners(bitmap, expectedOrientationCorners(orientation), "orientation $orientation")
                bitmap.recycle()
            } finally {
                fixture.delete()
            }
        }
    }

    @Test
    fun pathDecodeAppliesCombinedFlipAndRotation() {
        val fixture = taggedOrientationFixture(ExifInterface.ORIENTATION_TRANSVERSE)
        try {
            val bitmap = checkNotNull(decodeChatAttachmentImage(fixture.path, maxPixelSize = null))
            assertEquals(200, bitmap.width)
            assertEquals(320, bitmap.height)
            assertBitmapCorners(bitmap, expectedOrientationCorners(7), "orientation 7 path")
            bitmap.recycle()
        } finally {
            fixture.delete()
        }
    }

    @Test
    fun sampledDecodeKeepsExistingPowerOfTwoSizingBeforeOrientation() {
        val fixture = taggedOrientationFixture(ExifInterface.ORIENTATION_ROTATE_90)
        try {
            val bitmap =
                checkNotNull(
                    decodeChatAttachmentImage(
                        fixture.readBytes(),
                        maxPixelSize = 100,
                    ),
                )
            assertEquals(100, bitmap.width)
            assertEquals(160, bitmap.height)
            assertBitmapCorners(bitmap, expectedOrientationCorners(6), "sampled orientation 6")
            bitmap.recycle()
        } finally {
            fixture.delete()
        }
    }

    @Test
    fun missingOrientationFallsBackToIdentity() {
        val data =
            InstrumentationRegistry.getInstrumentation()
                .context
                .assets
                .open("image-orientation/asymmetric.jpg")
                .use { it.readBytes() }
        val bitmap = checkNotNull(decodeChatAttachmentImage(data, maxPixelSize = null))
        assertEquals(320, bitmap.width)
        assertEquals(200, bitmap.height)
        assertBitmapCorners(bitmap, expectedOrientationCorners(1), "missing orientation")
        bitmap.recycle()
    }

    private fun assertBitmapCorners(
        bitmap: Bitmap,
        expected: IntArray,
        label: String,
    ) {
        val actual =
            intArrayOf(
                bitmap.getPixel(bitmap.width / 4, bitmap.height / 4),
                bitmap.getPixel(bitmap.width * 3 / 4, bitmap.height / 4),
                bitmap.getPixel(bitmap.width / 4, bitmap.height * 3 / 4),
                bitmap.getPixel(bitmap.width * 3 / 4, bitmap.height * 3 / 4),
            )
        actual.zip(expected.toTypedArray()).forEachIndexed { index, (actualColor, expectedColor) ->
            assertColorNear(expectedColor, actualColor, "$label corner $index")
        }
    }

    private fun assertColorNear(
        expected: Int,
        actual: Int,
        label: String,
    ) {
        assertTrue("$label red", kotlin.math.abs(Color.red(expected) - Color.red(actual)) <= 35)
        assertTrue("$label green", kotlin.math.abs(Color.green(expected) - Color.green(actual)) <= 35)
        assertTrue("$label blue", kotlin.math.abs(Color.blue(expected) - Color.blue(actual)) <= 35)
    }
}
