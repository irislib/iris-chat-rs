package to.iris.chat.ui.screens

import android.graphics.Bitmap
import android.content.res.Configuration
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.ui.theme.IrisChatTheme
import to.iris.chat.IrisChatApp

class MessageRequestLayoutTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun actionsStayAboveNavigationBar() = checkActions(keyboard = false)

    @Test fun requestEntryDismissesKeyboardAndKeepsActionsReachableAfterScrolling() = checkActions(keyboard = true)

    @Test fun joinActionIsReachableWithLongInviteAndKeyboard() {
        val manager = (compose.activity.application as IrisChatApp).container.appManager
        compose.activityRule.scenario.onActivity { it.enableEdgeToEdge() }
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                JoinInviteScreen(manager, manager.state.value)
            }
        }
        compose.onNodeWithTag("joinInviteInput").performClick().performTextInput("https://example.com/invite/" + "a".repeat(600))
        compose.waitUntil(5_000) { bottomInset(WindowInsetsCompat.Type.ime()) > 0 }
        screenshot("join-keyboard")
        // In compact landscape the keyboard and toolbar leave less than a
        // button's height. Done dismisses the keyboard without submitting.
        if (compose.activity.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE) {
            compose.onNodeWithTag("joinInviteInput").performImeAction()
            compose.waitUntil(5_000) { bottomInset(WindowInsetsCompat.Type.ime()) == 0 }
        }
        val action = compose.onNodeWithTag("joinInviteAcceptButton").performScrollTo().assertIsDisplayed().assertIsEnabled()
        val safeBottom = compose.activity.window.decorView.height - bottomInset(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.ime())
        assertTrue("Join action must be above keyboard", with(compose.density) { action.getUnclippedBoundsInRoot().bottom.toPx() } <= safeBottom + 1)
        screenshot("join-keyboard-scrolled")
    }

    private fun checkActions(keyboard: Boolean) {
        var accepted = false
        val showRequest = mutableStateOf(!keyboard)
        compose.activityRule.scenario.onActivity { it.enableEdgeToEdge() }
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                // ChatScreen owns the top inset; each bottom bar owns its bottom insets.
                Scaffold(contentWindowInsets = WindowInsets(0, 0, 0, 0)) { padding ->
                    Column(Modifier.fillMaxSize().padding(padding).testTag("requestScreen")) {
                        var draft by remember { mutableStateOf("") }
                        if (keyboard) {
                            TextField(draft, { draft = it }, Modifier.statusBarsPadding().testTag("keyboardInput"))
                        }
                        LazyColumn(Modifier.weight(1f).testTag("history")) {
                            items(50) { Text("Earlier message $it", Modifier.padding(16.dp)) }
                        }
                        if (showRequest.value) {
                            MessageRequestBar("Alex", {}, {}, { accepted = true })
                        }
                    }
                }
            }
        }
        compose.onNodeWithTag("history").performScrollToIndex(49)
        if (keyboard) {
            compose.onNodeWithTag("keyboardInput").performClick().performTextInput("Draft")
            compose.waitUntil(5_000) { bottomInset(WindowInsetsCompat.Type.ime()) > 0 }
            compose.runOnIdle { showRequest.value = true }
            compose.waitUntil(5_000) { bottomInset(WindowInsetsCompat.Type.ime()) == 0 }
            compose.onNodeWithTag("keyboardInput").assertIsNotFocused()
        }
        compose.waitForIdle()
        screenshot("request-${if (keyboard) "keyboard" else "navigation"}")
        val root = compose.onNodeWithTag("requestScreen").fetchSemanticsNode().boundsInRoot
        val safeBottom = root.bottom - bottomInset(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.ime())
        for (tag in listOf("messageRequestBlockButton", "messageRequestBlockAndReportButton", "messageRequestAcceptButton")) {
            val button = compose.onNodeWithTag(tag).performScrollTo()
            compose.waitForIdle()
            button.assertIsDisplayed()
            val bounds = button.getUnclippedBoundsInRoot()
            with(compose.density) {
                assertTrue("$tag must fit above $safeBottom", bounds.bottom.toPx() <= safeBottom + 1)
                assertTrue("$tag inside screen", bounds.left.toPx() >= root.left && bounds.right.toPx() <= root.right && bounds.top.toPx() >= root.top)
            }
        }
        for (label in listOf("Block", "Block and report", "Accept")) {
            val layouts = mutableListOf<TextLayoutResult>()
            compose.onNodeWithText(label, useUnmergedTree = true).performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
            assertTrue("$label must expose its text layout", layouts.isNotEmpty())
            for (layout in layouts) {
                assertEquals("$label must not split across lines", 1, layout.lineCount)
                assertEquals("$label must include every character", label.length, layout.getLineEnd(0, visibleEnd = true))
                // Intrinsic text widths are fractional; Compose rounds the allocated width.
                assertTrue("$label must fit horizontally", layout.getLineRight(0) <= layout.size.width + 1)
                assertTrue("$label must fit vertically", layout.getLineBottom(0) <= layout.size.height + 1)
            }
        }
        screenshot("request-${if (keyboard) "keyboard" else "navigation"}-scrolled")
        compose.onNodeWithTag("messageRequestAcceptButton").performClick()
        assertTrue(accepted)
    }

    private fun bottomInset(type: Int): Int =
        ViewCompat.getRootWindowInsets(compose.activity.window.decorView)?.getInsets(type)?.bottom ?: 0

    private fun screenshot(name: String) {
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        File(instrumentation.targetContext.getExternalFilesDir("screenshots"), "$name.png")
            .outputStream().use {
                instrumentation.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, it)
            }
    }
}
