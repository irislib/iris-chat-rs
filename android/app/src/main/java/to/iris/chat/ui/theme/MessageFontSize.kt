package to.iris.chat.ui.theme

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.intPreferencesKey
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

// Device-local, like Signal. sp still respects Android's accessibility scale.
enum class MessageFontSize(val label: String, val body: Int, val quote: Int) {
    Small("Small", 14, 12),
    Normal("Normal", 16, 14),
    Large("Large", 22, 18),
    ExtraLarge("Extra large", 28, 22),
}

val LocalMessageFontSize = staticCompositionLocalOf { MessageFontSize.Normal }

class MessageFontSizePreference(
    private val store: DataStore<Preferences>,
    private val scope: CoroutineScope,
) {
    private val key = intPreferencesKey("message_font_size")
    val size = store.data
        .catch { if (it is IOException) emit(emptyPreferences()) else throw it }
        .map { values -> MessageFontSize.entries.firstOrNull { it.body == values[key] } ?: MessageFontSize.Normal }
        .stateIn(scope, SharingStarted.Eagerly, MessageFontSize.Normal)

    fun set(size: MessageFontSize) = scope.launch {
        store.edit { it[key] = size.body }
    }
}
