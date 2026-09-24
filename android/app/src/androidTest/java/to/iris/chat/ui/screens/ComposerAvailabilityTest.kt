package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.rust.DirectChatCapabilityState
import to.iris.chat.ui.theme.*

class ComposerAvailabilityTest {
    @get:Rule val compose = createComposeRule()

    @Test fun checkingKeepsDraftFocusAndPositionThenEnablesSend() {
        val state = mutableStateOf(DirectChatCapabilityState.CHECKING)
        val draft = mutableStateOf("")
        var sent = ""
        compose.mainClock.autoAdvance = false
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                Column {
                    androidx.compose.foundation.layout.Spacer(androidx.compose.ui.Modifier.weight(1f))
                    DirectChatComposer("test-chat", state.value, {}) {
                        ComposerBar(draft.value, emptyList(), false, false, null,
                            onDraftChange = { draft.value = it }, onAttach = {}, onRemoveAttachment = {},
                            onSend = { sent = draft.value }, sendAllowed = state.value == DirectChatCapabilityState.AVAILABLE)
                    }
                }
            }
        }
        compose.mainClock.advanceTimeByFrame()
        val field = compose.onNodeWithTag("chatMessageInput")
        val unfocusedTop = field.fetchSemanticsNode().boundsInRoot.top
        field.performClick().performTextInput("See you soon")
        compose.mainClock.advanceTimeBy(800)
        compose.waitUntil(5_000) { field.fetchSemanticsNode().boundsInRoot.top < unfocusedTop }
        var bounds = field.fetchSemanticsNode().boundsInRoot
        var lastChange = System.nanoTime()
        compose.waitUntil(5_000) {
            val next = field.fetchSemanticsNode().boundsInRoot
            if (bounds != next) { bounds = next; lastChange = System.nanoTime() }
            System.nanoTime() - lastChange > 200_000_000
        }
        compose.onNodeWithTag("directChatCapabilityBar").assertDoesNotExist()
        compose.onNodeWithTag("chatSendButton").assertIsNotEnabled()
        compose.mainClock.advanceTimeBy(2_100)
        compose.onNodeWithTag("directChatCapabilityBar").assertExists()
        field.assertTextContains("See you soon").assertIsFocused()
        assertEquals(bounds, field.fetchSemanticsNode().boundsInRoot)
        screenshot("checking-composer.png")
        compose.runOnUiThread { state.value = DirectChatCapabilityState.AVAILABLE }
        compose.mainClock.advanceTimeByFrame()
        compose.onNodeWithTag("directChatCapabilityBar").assertDoesNotExist()
        assertEquals(bounds, field.fetchSemanticsNode().boundsInRoot)
        compose.onNodeWithTag("chatSendButton").assertIsEnabled().performClick()
        assertEquals("See you soon", sent)
    }

    @Test fun quickCheckNeverShowsStatusAndSizeChoiceUpdatesComposer() {
        val state = mutableStateOf(DirectChatCapabilityState.CHECKING)
        val size = mutableStateOf(MessageFontSize.Normal)
        compose.mainClock.autoAdvance = false
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                CompositionLocalProvider(LocalMessageFontSize provides size.value) {
                    Column {
                        MessageFontSizeSetting(size.value) { size.value = it }
                        androidx.compose.foundation.layout.Spacer(androidx.compose.ui.Modifier.weight(1f))
                        DirectChatComposer("test-chat", state.value, {}) {
                            ComposerBar("A message that wraps naturally at larger sizes.", emptyList(), false, false, null,
                                onDraftChange = {}, onAttach = {}, onRemoveAttachment = {}, onSend = {})
                        }
                    }
                }
            }
        }
        compose.mainClock.advanceTimeBy(500)
        val field = compose.onNodeWithTag("chatMessageInput")
        val normalHeight = field.fetchSemanticsNode().boundsInRoot.height
        compose.runOnUiThread { state.value = DirectChatCapabilityState.AVAILABLE }
        compose.mainClock.advanceTimeBy(2_100)
        compose.onNodeWithTag("directChatCapabilityBar").assertDoesNotExist()
        compose.mainClock.autoAdvance = true
        compose.onNodeWithTag("messageFontSizeSetting").performClick()
        compose.onNodeWithTag("messageFontSizeExtraLarge").assertIsDisplayed()
        screenshot("message-font-choices.png")
        compose.onNodeWithTag("messageFontSizeExtraLarge").performClick()
        assertEquals(MessageFontSize.ExtraLarge, size.value)
        assertTrue(field.fetchSemanticsNode().boundsInRoot.height > normalHeight)
        screenshot("large-composer.png")
    }

    @Test fun fontSizePersistsAcrossPreferenceRecreation() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val file = File(context.cacheDir, "font-test-${System.nanoTime()}.preferences_pb")
        val firstScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val secondScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        try {
            val first = MessageFontSizePreference(PreferenceDataStoreFactory.create(scope = firstScope) { file }, firstScope)
            first.set(MessageFontSize.ExtraLarge).join()
            withTimeout(5_000) { first.size.first { it == MessageFontSize.ExtraLarge } }
            firstScope.coroutineContext[Job]!!.cancelAndJoin()
            val restored = MessageFontSizePreference(PreferenceDataStoreFactory.create(scope = secondScope) { file }, secondScope)
            assertEquals(MessageFontSize.ExtraLarge, withTimeout(5_000) { restored.size.first { it == MessageFontSize.ExtraLarge } })
        } finally {
            firstScope.coroutineContext[Job]!!.cancelAndJoin()
            secondScope.coroutineContext[Job]!!.cancelAndJoin()
            file.delete()
        }
    }

    private fun screenshot(name: String) {
        compose.waitForIdle()
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        File(context.getExternalFilesDir(null), name).outputStream().use {
            InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
        }
    }
}
