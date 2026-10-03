package to.iris.chat

import android.graphics.Bitmap
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.core.AppManager
import to.iris.chat.core.MockRustAppClient
import to.iris.chat.core.RecordingSecureSecretStore
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.DeviceHistorySyncPhase
import to.iris.chat.rust.DeviceHistorySyncSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.rust.buildLargeTestAppState
import to.iris.chat.ui.screens.ChatListScreen
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class DeviceHistorySyncUiTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun initial_history_progress_keeps_chats_usable_and_hides_when_complete() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext.applicationContext
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val dataFile = context.preferencesDataStoreFile("history-progress-${UUID.randomUUID()}")
        val dataStore = PreferenceDataStoreFactory.create(scope = scope, produceFile = { dataFile })
        val initial = buildLargeTestAppState(2u, 1u, 0u).apply {
            preferences.nearbyShowInChatList = false
            deviceHistorySync = DeviceHistorySyncSnapshot(DeviceHistorySyncPhase.DISCOVERING, 0u, null)
        }
        val rust = MockRustAppClient(initial)
        val manager = AppManager(
            context = context,
            applicationScope = scope,
            secureSecretStore = RecordingSecureSecretStore(),
            dataStore = dataStore,
            rustFactory = { _, _ -> rust },
        )
        try {
            compose.setContent { IrisChatTheme { ChatListScreen(manager) } }
            for ((index, phase) in listOf(
                DeviceHistorySyncPhase.DISCOVERING,
                DeviceHistorySyncPhase.TRANSFERRING,
                DeviceHistorySyncPhase.WAITING,
                DeviceHistorySyncPhase.COMPLETE,
            ).withIndex()) {
                compose.runOnIdle {
                    rust.emit(AppUpdate.FullState(initial.copy(
                        rev = (index + 1).toULong(),
                        deviceHistorySync = DeviceHistorySyncSnapshot(
                            phase, if (phase == DeviceHistorySyncPhase.COMPLETE) 1_284u else 342u,
                            if (phase == DeviceHistorySyncPhase.DISCOVERING) null else 1_284u,
                        ),
                    )))
                }
                compose.waitUntil(5_000) { manager.deviceHistorySync.value?.phase == phase }
                compose.waitForIdle()
                if (phase == DeviceHistorySyncPhase.COMPLETE) {
                    compose.onAllNodesWithTag("deviceHistorySyncStatus").assertCountEquals(0)
                } else {
                    compose.onNodeWithTag("deviceHistorySyncStatus").assertIsDisplayed()
                    compose.onNodeWithText(if (phase == DeviceHistorySyncPhase.WAITING)
                        "Waiting for your other device…" else "Syncing messages…").assertIsDisplayed()
                    if (phase != DeviceHistorySyncPhase.DISCOVERING) {
                        compose.onNodeWithText("342 of 1,284").assertIsDisplayed()
                    } else {
                        compose.onAllNodesWithTag("deviceHistorySyncCount").assertCountEquals(0)
                    }
                }
                val screenshot = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
                File(context.getExternalFilesDir("screenshots"), "history-sync-${phase.name.lowercase()}.png")
                    .outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
                screenshot.recycle()
                if (phase == DeviceHistorySyncPhase.WAITING) {
                    compose.onNodeWithTag("chatListNewChatButton").performClick()
                    compose.runOnIdle { assertTrue(manager.router.value.screenStack.contains(Screen.NewChat)) }
                }
            }
        } finally {
            scope.cancel()
            dataFile.delete()
        }
    }
}
