package to.iris.chat.core

import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.onNodeWithTag
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import to.iris.chat.IrisChatApp
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.rust.*
import to.iris.chat.ui.screens.ChatScreen
import to.iris.chat.ui.theme.IrisChatTheme

class DirectShareFlowTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var manager: AppManager
    private lateinit var rust: MockRustAppClient
    private lateinit var state: AppState
    private lateinit var context: Context
    private lateinit var root: File
    private var preferencesPrefix = "direct-share-flow-${UUID.randomUUID()}"

    @Before fun setUp() {
        val realManager = (compose.activity.application as IrisChatApp).container.appManager
        compose.waitUntil(10_000) { realManager.bootstrapState.value !is AccountBootstrapState.Loading }
        val base = compose.activity.applicationContext
        root = File(base.cacheDir, preferencesPrefix).apply { mkdirs() }
        context = object : ContextWrapper(base) {
            override fun getApplicationContext() = this
            override fun getFilesDir() = File(root, "files").apply { mkdirs() }
            override fun getNoBackupFilesDir() = File(root, "no-backup").apply { mkdirs() }
            override fun getCacheDir() = File(root, "cache").apply { mkdirs() }
            override fun getSharedPreferences(name: String, mode: Int) = base.getSharedPreferences("$preferencesPrefix-$name", mode)
        }
        rust = MockRustAppClient(AppManagerContractDefaults.initialState())
        compose.runOnUiThread {
            manager = AppManager(context, scope, secureSecretStore = RecordingSecureSecretStore(),
                rustFactory = { _, _ -> rust }, clearDeliveredNotifications = {})
        }
        compose.waitUntil(5_000) { manager.bootstrapState.value is AccountBootstrapState.NeedsLogin }
        state = buildLargeTestAppState(2u, 0u, 3u).apply {
            rev = 1u
            preferences.nearbyBluetoothEnabled = false
            preferences.nearbyLanEnabled = false
            preferences.desktopNotificationsEnabled = false
            chatList = chatList.mapIndexed { index, chat -> chat.copy(
                displayName = if (index == 0) "Alex" else "Maya",
                lastMessageAtSecs = (System.currentTimeMillis() / 1000 - index * 60).toULong(),
                lastMessageIsOutgoing = true, isRequest = false, draft = "Existing draft",
            ) }
            currentChat = currentChat!!.copy(chatId = chatList.first().chatId, displayName = "Alex", draft = "Existing draft",
                isRequest = false, directChatCapability = DirectChatCapabilityState.AVAILABLE)
            router = Router(Screen.ChatList, emptyList())
        }
        InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("cmd shortcut reset-throttling").close()
    }

    @After fun tearDown() {
        scope.cancel()
        DirectShareShortcuts(context).update(ShareShortcutState(null, emptyList()))
        context.getSharedPreferences("direct_share", Context.MODE_PRIVATE).edit().clear().commit()
        root.deleteRecursively()
    }

    @Test fun coldShareWaitsForAccountThenPrefillsComposerWithoutSending() {
        // Persisted OS shortcut, before the account has finished restoring.
        val shortcuts = DirectShareShortcuts(context)
        shortcuts.update(ShareShortcutState.from(state))
        val id = ShortcutManagerCompat.getDynamicShortcuts(context).single { it.shortLabel == "Alex" }.id
        compose.runOnUiThread {
            manager.receiveShare("Shared picnic plans", listOf(OutgoingAttachment("/cache/picnic.txt", "picnic.txt")), id)
        }
        assertNull(manager.pendingShare.value!!.targetChatId)
        emit(state)
        compose.waitUntil(5_000) { manager.pendingShare.value?.targetChatId == state.chatList.first().chatId }
        val restoration = StateRestorationTester(compose)
        restoration.setContent { IrisChatTheme(darkTheme = false) { ChatScreen(manager, state.chatList.first().chatId) } }
        compose.onNodeWithTag("chatMessageInput").assertTextContains("Existing draft\nShared picnic plans")
        compose.onNodeWithTag("chatSelectedAttachments").assertIsDisplayed()
        compose.onNodeWithTag("chatSendButton").assertIsDisplayed()
        assertNull(manager.pendingShare.value)
        assertTrue(rust.dispatchedActions.none { it is AppAction.SendMessage || it is AppAction.SendAttachments })
        screenshot("direct-share-composer")

        // Rust persists the draft; the UI must also retain the staged attachment on recreation.
        state = state.copy(rev = 2u, currentChat = state.currentChat!!.copy(draft = "Existing draft\nShared picnic plans"))
        emit(state)
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("chatMessageInput").assertTextContains("Existing draft\nShared picnic plans")
        compose.onNodeWithTag("chatSelectedAttachments").assertIsDisplayed()

        // A second share into an already open chat is consumed exactly once.
        compose.runOnUiThread { manager.receiveShare("Another link", emptyList(), id) }
        compose.onNodeWithTag("chatMessageInput").assertTextContains("Existing draft\nShared picnic plans\nAnother link")
        assertNull(manager.takePendingShareForChat(state.chatList.first().chatId))
    }

    @Test fun invalidAndBlockedTargetsFallBackToRecipientSelection() {
        emit(state)
        compose.runOnUiThread { manager.receiveShare("Review this", emptyList(), "forged-id") }
        assertNull(manager.pendingShare.value!!.targetChatId)
        assertNull(manager.pendingShare.value!!.shortcutId)
        compose.waitUntil(5_000) { ShortcutManagerCompat.getDynamicShortcuts(context).any { it.shortLabel == "Alex" } }
        val id = ShortcutManagerCompat.getDynamicShortcuts(context).single { it.shortLabel == "Alex" }.id
        state = state.copy(rev = 2u, preferences = state.preferences.copy(blockedOwnerPubkeys = listOf(state.chatList.first().chatId)))
        emit(state)
        compose.runOnUiThread { manager.receiveShare("Review this", emptyList(), id) }
        assertNull(manager.pendingShare.value!!.targetChatId)
        assertNull(manager.pendingShare.value!!.shortcutId)
        assertTrue(rust.dispatchedActions.none { it is AppAction.SendMessage })
    }

    @Test fun systemShareSheetOffersNamedRecipients() {
        emit(state)
        compose.waitUntil(5_000) { ShortcutManagerCompat.getDynamicShortcuts(context).any { it.shortLabel == "Alex" } }
        compose.runOnUiThread {
            compose.activity.startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain")
                .putExtra(Intent.EXTRA_TEXT, "Picnic on Sunday?"), null))
        }
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        compose.waitUntil(10_000) {
            automation.rootInActiveWindow?.let { root ->
                root.findAccessibilityNodeInfosByText("Alex").isNotEmpty() || root.findAccessibilityNodeInfosByText("Maya").isNotEmpty()
            } == true
        }
        screenshot("direct-share-system-sheet")
        automation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK)
    }

    private fun emit(snapshot: AppState) {
        compose.runOnUiThread {
            rust.currentState = snapshot
            rust.emit(AppUpdate.FullState(snapshot))
        }
        compose.waitUntil(5_000) { manager.state.value.rev == snapshot.rev }
    }

    private fun screenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.waitForIdle(500, 5_000)
        val bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
        File(instrumentation.targetContext.getExternalFilesDir("screenshots"), "$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }
}
