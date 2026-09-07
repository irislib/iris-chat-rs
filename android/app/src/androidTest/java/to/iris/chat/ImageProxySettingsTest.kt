package to.iris.chat

import android.graphics.Bitmap
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ImageProxySettingsTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun fallback_is_off_by_default_and_requires_proxy_enabled() {
        val manager = (composeRule.activity.application as IrisChatApp).container.appManager
        manager.resetForUiTestsBlocking()
        waitForTag("welcomeCreateAction")
        composeRule.onNodeWithTag("welcomeCreateAction", useUnmergedTree = true).performClick()
        waitForTag("signupNameField")
        composeRule.onNodeWithTag("signupNameField", useUnmergedTree = true).performTextInput("Image settings tester")
        composeRule.onNodeWithTag("generateKeyButton", useUnmergedTree = true).performClick()
        waitForTag("chatListProfileButton")
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()
        waitForTag("settingsMediaRow")
        composeRule.onNodeWithTag("settingsMediaRow", useUnmergedTree = true).performScrollTo().performClick()
        waitForTag("myProfileImageProxyFallbackSwitch")
        val fallback = composeRule.onNodeWithTag("myProfileImageProxyFallbackSwitch", useUnmergedTree = true)
        fallback.performScrollTo().assertIsOff().assertIsEnabled()
        composeRule.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val screenshot = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
        File(instrumentation.targetContext.getExternalFilesDir("screenshots"), "image-proxy-fallback.png")
            .outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        screenshot.recycle()
        fallback.performClick()
        composeRule.waitUntil(10_000) { manager.state.value.preferences.imageProxyFallbackEnabled }
        fallback.assertIsOn()
        composeRule.onNodeWithTag("myProfileImageProxySwitch", useUnmergedTree = true).performScrollTo().performClick()
        composeRule.waitUntil(10_000) { !manager.state.value.preferences.imageProxyEnabled }
        fallback.assertIsNotEnabled().assertIsOn()
        composeRule.onNodeWithTag("myProfileImageProxySwitch", useUnmergedTree = true).performClick()
        composeRule.waitUntil(10_000) { manager.state.value.preferences.imageProxyEnabled }
        fallback.assertIsEnabled().assertIsOn().performClick()
        composeRule.waitUntil(10_000) { !manager.state.value.preferences.imageProxyFallbackEnabled }
        fallback.assertIsOff()
    }

    private fun waitForTag(tag: String) {
        composeRule.waitUntil(30_000) {
            composeRule.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
    }
}
