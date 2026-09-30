package to.iris.chat.ui.screens

import android.graphics.Bitmap
import android.util.LruCache
import to.iris.chat.core.AccountImageSession

internal object SelectedImageThumbnailCache {
    private const val MaxCacheKb = 8 * 1024

    private val cache =
        object : LruCache<String, Bitmap>(MaxCacheKb) {
            override fun sizeOf(
                key: String,
                value: Bitmap,
            ): Int = (value.byteCount / 1024).coerceAtLeast(1)
        }

    init { AccountImageSession.register { cache.evictAll() } }

    fun get(key: String): Bitmap? = cache.get(key)

    fun put(
        key: String,
        bitmap: Bitmap,
        generation: Long,
    ): Boolean = AccountImageSession.ifCurrent(generation) { cache.put(key, bitmap); true } ?: false
}

internal object ChatAttachmentPreviewBitmapCache {
    private const val MaxCacheKb = 48 * 1024

    private val cache =
        object : LruCache<String, Bitmap>(MaxCacheKb) {
            override fun sizeOf(
                key: String,
                value: Bitmap,
            ): Int = (value.byteCount / 1024).coerceAtLeast(1)
        }

    init { AccountImageSession.register { cache.evictAll() } }

    fun get(key: String): Bitmap? = cache.get(key)

    fun put(
        key: String,
        bitmap: Bitmap,
        generation: Long,
    ): Boolean = AccountImageSession.ifCurrent(generation) { cache.put(key, bitmap); true } ?: false
}
