package to.iris.chat.ui.screens

import android.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class ChatAttachmentOrientationComposeTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun imageAttachmentPreviewDisplaysOrientedBitmap() {
        val fixture = taggedOrientationFixture(2)
        try {
            val data = fixture.readBytes()
            val filename = "orientation-2.jpg"
            val attachment =
                MessageAttachmentSnapshot(
                    nhash = "nhash1orientation2",
                    filename = filename,
                    filenameEncoded = filename,
                    htreeUrl = "htree://nhash1orientation2/$filename",
                    isImage = true,
                    isVideo = false,
                    isAudio = false,
                )

            composeRule.setContent {
                IrisChatTheme(darkTheme = false) {
                    AttachmentChip(
                        attachment = attachment,
                        isOutgoing = false,
                        downloadAttachment = { data },
                        onOpenImage = { _, _ -> },
                        onForward = {},
                    )
                }
            }

            composeRule.waitUntil(5_000) {
                runCatching {
                    composeRule
                        .onNodeWithContentDescription(filename, useUnmergedTree = true)
                        .fetchSemanticsNode()
                    true
                }.getOrDefault(false)
            }
            val image =
                composeRule
                    .onNodeWithContentDescription(filename, useUnmergedTree = true)
                    .captureToImage()
            val pixels = image.toPixelMap()
            val expected = expectedOrientationCorners(2)
            val actual =
                intArrayOf(
                    pixels[pixels.width / 4, pixels.height / 4].toArgb(),
                    pixels[pixels.width * 3 / 4, pixels.height / 4].toArgb(),
                    pixels[pixels.width / 4, pixels.height * 3 / 4].toArgb(),
                    pixels[pixels.width * 3 / 4, pixels.height * 3 / 4].toArgb(),
                )
            actual.zip(expected.toTypedArray()).forEachIndexed { index, (actualColor, expectedColor) ->
                assertColorNear(expectedColor, actualColor, "preview corner $index")
            }
        } finally {
            fixture.delete()
        }
    }

    private fun assertColorNear(
        expected: Int,
        actual: Int,
        label: String,
    ) {
        assertTrue("$label red", kotlin.math.abs(Color.red(expected) - Color.red(actual)) <= 45)
        assertTrue("$label green", kotlin.math.abs(Color.green(expected) - Color.green(actual)) <= 45)
        assertTrue("$label blue", kotlin.math.abs(Color.blue(expected) - Color.blue(actual)) <= 45)
    }
}
