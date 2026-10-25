package to.iris.chat.core

import android.content.Context
import android.content.Intent
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class DirectShareShortcutsTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val preferencesName = "direct-share-test-${UUID.randomUUID()}"
    private val now = System.currentTimeMillis() / 1000
    private lateinit var shortcuts: DirectShareShortcuts
    private val state get() = ShareShortcutState("private-account-key", listOf(
        ShareRecipient("private-alex-key", "Alex", now - 60),
        ShareRecipient("private-maya-key", "Maya", now - 10),
        ShareRecipient("never-sent", "Stranger", null),
        ShareRecipient("old-chat", "Old chat", now - 31 * 86400),
    ))

    @Before fun setUp() {
        InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("cmd shortcut reset-throttling").close()
        shortcuts = DirectShareShortcuts(context, preferencesName)
    }

    @After fun tearDown() {
        shortcuts.update(ShareShortcutState(null, emptyList()), now)
        context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE).edit().clear().commit()
    }

    @Test fun publishesRecentOutgoingRecipientsWithOpaqueIdsAndMatchingShareCategory() {
        shortcuts.update(state, now)
        val published = ShortcutManagerCompat.getDynamicShortcuts(context).sortedBy { it.rank }
        assertEquals(listOf("Maya", "Alex"), published.map { it.shortLabel.toString() })
        for (shortcut in published) {
            assertEquals(setOf(DirectShareShortcuts.CATEGORY), shortcut.categories)
            assertEquals(DirectShareShortcuts.ACTION_OPEN_CHAT, shortcut.intent.action)
            assertEquals(shortcut.id, shortcut.intent.getStringExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID))
            assertEquals(setOf(ShortcutManagerCompat.EXTRA_SHORTCUT_ID), shortcut.intent.extras!!.keySet())
            assertFalse(shortcut.id.contains("private"))
            assertNull(shortcut.intent.data)
            assertNotNull(shortcuts.resolve(shortcut.id, state, now))
        }
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER).setPackage(context.packageName)
        val activity = context.packageManager.resolveActivity(launcher, android.content.pm.PackageManager.GET_META_DATA)!!.activityInfo
        assertTrue(activity.metaData.getInt("android.app.shortcuts") != 0)
    }

    @Test fun replyAndRestartKeepTheSameRecipientWhileNewSendChangesRank() {
        shortcuts.update(state, now)
        val alexId = ShortcutManagerCompat.getDynamicShortcuts(context).single { it.shortLabel == "Alex" }.id
        val afterReply = state.copy(recipients = state.recipients.map { it.copy(lastSent = null) })
        val restarted = DirectShareShortcuts(context, preferencesName)
        restarted.update(afterReply, now)
        assertEquals("private-alex-key", restarted.resolve(alexId, afterReply, now))
        restarted.update(afterReply.copy(recipients = afterReply.recipients.map {
            if (it.name == "Alex") it.copy(name = "Alex Updated", lastSent = now) else it
        }), now)
        val first = ShortcutManagerCompat.getDynamicShortcuts(context).minBy { it.rank }
        assertEquals(alexId, first.id)
        assertEquals("Alex Updated", first.shortLabel)
    }

    @Test fun removedExpiredAndOtherAccountTargetsCannotBeUsed() {
        shortcuts.update(state, now)
        val id = ShortcutManagerCompat.getDynamicShortcuts(context).single { it.shortLabel == "Alex" }.id
        assertNull(shortcuts.resolve(id, state.copy(accountId = "another-account"), now))
        assertNull(shortcuts.resolve("forged-id", state, now))
        assertNull(shortcuts.resolve(id, state, now + 31 * 86400))
        val removed = state.copy(recipients = state.recipients.filter { it.name != "Alex" })
        assertNull(shortcuts.resolve(id, removed, now))
        shortcuts.update(removed, now)
        assertFalse(ShortcutManagerCompat.getDynamicShortcuts(context).any { it.id == id })
        shortcuts.update(ShareShortcutState(null, emptyList()), now)
        assertTrue(ShortcutManagerCompat.getDynamicShortcuts(context).isEmpty())
        assertNull(shortcuts.resolve(id, state, now))
    }
}
