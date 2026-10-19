package to.iris.chat.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.runtime.produceState
import java.util.concurrent.ConcurrentHashMap
import to.iris.chat.core.AccountImageSession
import to.iris.chat.core.AppManager

@Composable
internal fun rememberNhashImageData(
    appManager: AppManager,
    pictureUrl: String?,
) = produceState(
    initialValue = parseNhashUri(pictureUrl)?.let { NhashImageDataCache.get(it) },
    pictureUrl,
) {
    val nhash = parseNhashUri(pictureUrl)
    if (nhash == null) {
        value = null
        return@produceState
    }
    NhashImageDataCache.get(nhash)?.let {
        value = it
        return@produceState
    }
    val generation = AccountImageSession.current()
    val loaded = appManager.resolveHashtreePictureBytes(nhash)
    if (AccountImageSession.isCurrent(generation)) {
        loaded?.let { NhashImageDataCache.put(nhash, it, generation) }
        value = loaded
    }
}

internal object NhashImageDataCache {
    private val images = ConcurrentHashMap<String, ByteArray>()
    init { AccountImageSession.register { images.clear() } }

    fun get(nhash: String): ByteArray? = images[nhash]

    fun put(nhash: String, data: ByteArray, generation: Long) {
        AccountImageSession.ifCurrent(generation) { images[nhash] = data }
    }
}

internal fun parseNhashUri(value: String?): String? {
    val trimmed = value?.trim().orEmpty()
    val prefix = when {
        trimmed.startsWith("htree://") -> "htree://"
        trimmed.startsWith("nhash://") -> "nhash://"
        else -> return null
    }
    return trimmed
        .removePrefix(prefix)
        .substringBefore("/")
        .takeIf(String::isNotBlank)
}

