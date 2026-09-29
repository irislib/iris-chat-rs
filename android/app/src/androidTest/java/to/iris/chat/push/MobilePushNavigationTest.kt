package to.iris.chat.push

import android.app.PendingIntent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MobilePushNavigationTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun twoChatsKeepDistinctPendingIntentsAndTheirOriginalDestinations() {
        val first = MobilePushNotifier.launchIntent(context, "{\"chat_id\":\"notification-test-a\"}")
        val second = MobilePushNotifier.launchIntent(context, "{\"chat_id\":\"notification-test-b\"}")
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val a = PendingIntent.getActivity(context, 0, first, flags)
        val b = PendingIntent.getActivity(context, 0, second, flags)
        try {
            assertNotEquals(a, b)
            assertFalse(first.filterEquals(second))
            assertEquals("notification-test-a", first.getStringExtra(MobilePushNotifier.CHAT_ID_EXTRA))
            assertEquals("notification-test-b", second.getStringExtra(MobilePushNotifier.CHAT_ID_EXTRA))
            assertEquals(MobilePushNotifier.ACTION_OPEN_CHAT, first.action)
        } finally { a.cancel(); b.cancel() }
    }

    @Test
    fun groupDestinationTakesPrecedenceOverTheMessageAuthor() {
        val intent = MobilePushNotifier.launchIntent(context, "{\"sender_pubkey\":\"sender\",\"group_id\":\"team\"}")
        assertEquals("group:team", intent.getStringExtra(MobilePushNotifier.CHAT_ID_EXTRA))
        val canonical = MobilePushNotifier.launchIntent(context, "{\"chat_id\":\"group:canonical\",\"group_id\":\"team\"}")
        assertEquals("group:canonical", canonical.getStringExtra(MobilePushNotifier.CHAT_ID_EXTRA))
    }

    @Test
    fun genericAlertHasNoInventedChatDestination() {
        val intent = MobilePushNotifier.launchIntent(context, "{}")
        assertEquals("to.iris.chat.OPEN_CHAT_LIST", intent.action)
        assertFalse(intent.hasExtra(MobilePushNotifier.CHAT_ID_EXTRA))
    }
}
