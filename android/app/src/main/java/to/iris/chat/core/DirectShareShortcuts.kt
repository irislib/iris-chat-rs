package to.iris.chat.core

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.util.Log
import androidx.core.app.Person
import androidx.core.content.LocusIdCompat
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject
import to.iris.chat.MainActivity
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.DeviceAuthorizationState

internal data class ShareRecipient(val chatId: String, val name: String, val lastSent: Long?)

internal data class ShareShortcutState(val accountId: String?, val recipients: List<ShareRecipient>) {
    companion object {
        fun from(state: AppState): ShareShortcutState {
            val account = state.account?.takeIf { it.authorizationState == DeviceAuthorizationState.AUTHORIZED }
                ?: return ShareShortcutState(null, emptyList())
            val blocked = state.preferences.blockedOwnerPubkeys.toSet()
            return ShareShortcutState(account.publicKeyHex, state.chatList.filter {
                it.kind == ChatKind.DIRECT && !it.isRequest && it.chatId != account.publicKeyHex && it.chatId !in blocked
            }.map {
                ShareRecipient(it.chatId, it.displayName.trim().ifEmpty { "Chat" },
                    it.lastMessageAtSecs?.toLong()?.takeIf { _ -> it.lastMessageIsOutgoing == true })
            })
        }
    }
}

/** Only names and random local IDs leave app storage. No account/contact keys or message content. */
internal class DirectShareShortcuts(context: Context, preferencesName: String = "direct_share") {
    private val context = context.applicationContext
    private val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
    private var published: List<Pair<Record, String>>? = null

    private data class Record(val chatId: String, val shortcutId: String, val lastSent: Long)

    // Called by one background collector: ShortcutManager can block on some devices.
    fun update(state: ShareShortcutState, now: Long = System.currentTimeMillis() / 1000) {
        try {
            val previous = if (preferences.getString("account", null) == state.accountId) readRecords() else emptyList()
            val byChat = previous.associateBy { it.chatId }
            val records = if (state.accountId == null) emptyList() else state.recipients.mapNotNull { recipient ->
                val old = byChat[recipient.chatId]
                val sent = maxOf(old?.lastSent ?: 0, recipient.lastSent ?: 0)
                if (sent <= 0 || sent < now - RECENT_SECONDS) null
                else Record(recipient.chatId, old?.shortcutId ?: PREFIX + UUID.randomUUID(), sent)
            }.sortedWith(compareByDescending<Record> { it.lastSent }.thenBy { it.shortcutId })

            // Persist before publication so a cold-start tap can always resolve its opaque ID.
            if (records != previous || preferences.getString("account", null) != state.accountId) {
                val json = JSONArray()
                records.forEach { json.put(JSONObject().put("chat", it.chatId).put("id", it.shortcutId).put("sent", it.lastSent)) }
                if (!preferences.edit().putString("account", state.accountId).putString("recipients", json.toString()).commit()) return
            }

            val validIds = records.mapTo(mutableSetOf()) { it.shortcutId }
            val existing = ShortcutManagerCompat.getShortcuts(context,
                ShortcutManagerCompat.FLAG_MATCH_DYNAMIC or ShortcutManagerCompat.FLAG_MATCH_PINNED or ShortcutManagerCompat.FLAG_MATCH_CACHED)
            val stale = existing.map { it.id }.filter { it.startsWith(PREFIX) && it !in validIds }.distinct()
            if (stale.isNotEmpty()) {
                ShortcutManagerCompat.disableShortcuts(context, stale, "Chat unavailable")
                ShortcutManagerCompat.removeDynamicShortcuts(context, stale)
                ShortcutManagerCompat.removeLongLivedShortcuts(context, stale)
            }
            val names = state.recipients.associate { it.chatId to it.name }
            val active = records.take(minOf(ShortcutManagerCompat.getMaxShortcutCountPerActivity(context), 8))
                .map { it to names.getValue(it.chatId) }
            if (active == published) return
            val shortcuts = active.mapIndexed { rank, (record, name) -> shortcut(record, name, rank) }
            if (ShortcutManagerCompat.setDynamicShortcuts(context, shortcuts)) {
                active.filter { (record, _) -> record.lastSent > (byChat[record.chatId]?.lastSent ?: 0) }
                    .forEach { (record, _) -> ShortcutManagerCompat.reportShortcutUsed(context, record.shortcutId) }
                published = active
            }
        } catch (_: RuntimeException) {
            // A locked user or a launcher failure must never interrupt messaging.
            Log.w("DirectShare", "Could not update sharing suggestions")
        }
    }

    fun resolve(shortcutId: String, state: ShareShortcutState, now: Long = System.currentTimeMillis() / 1000): String? {
        if (state.accountId == null || preferences.getString("account", null) != state.accountId) return null
        val record = readRecords().firstOrNull { it.shortcutId == shortcutId && it.lastSent >= now - RECENT_SECONDS } ?: return null
        return record.chatId.takeIf { id -> state.recipients.any { it.chatId == id } }
    }

    private fun readRecords(): List<Record> = runCatching {
        val array = JSONArray(preferences.getString("recipients", "[]"))
        List(array.length()) { index ->
            val item = array.getJSONObject(index)
            Record(item.getString("chat"), item.getString("id"), item.getLong("sent"))
        }
    }.getOrDefault(emptyList())

    private fun shortcut(record: Record, name: String, rank: Int): ShortcutInfoCompat =
        ShortcutInfoCompat.Builder(context, record.shortcutId)
            .setActivity(ComponentName(context, MainActivity::class.java))
            .setIntent(Intent(context, MainActivity::class.java).setAction(ACTION_OPEN_CHAT)
                .putExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID, record.shortcutId))
            .setShortLabel(name)
            .setLongLabel(name)
            .setIcon(initialsIcon(name))
            .setPerson(Person.Builder().setName(name).setKey(record.shortcutId).build())
            .setCategories(setOf(CATEGORY))
            .setLocusId(LocusIdCompat(record.shortcutId))
            .setLongLived(true)
            .setRank(rank)
            .build()

    private fun initialsIcon(name: String): IconCompat {
        val bitmap = Bitmap.createBitmap(108, 108, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(Color.rgb(76, 83, 155))
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.WHITE
            textSize = 42f
            textAlign = Paint.Align.CENTER
            typeface = android.graphics.Typeface.DEFAULT_BOLD
        }
        val initials = name.split(Regex("\\s+")).take(2).joinToString("") {
            String(Character.toChars(it.codePointAt(0)))
        }.uppercase()
        canvas.drawText(initials, 54f, 54f - (paint.ascent() + paint.descent()) / 2, paint)
        return IconCompat.createWithAdaptiveBitmap(bitmap)
    }

    companion object {
        const val ACTION_OPEN_CHAT = "to.iris.chat.OPEN_SHARE_CHAT"
        const val CATEGORY = "to.iris.chat.SHARE_CHAT"
        private const val PREFIX = "share-"
        private const val RECENT_SECONDS = 30L * 24 * 60 * 60
    }
}
