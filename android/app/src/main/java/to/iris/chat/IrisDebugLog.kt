package to.iris.chat

import android.util.Log

object IrisDebugLog {
    @Volatile
    var enabled: Boolean = false

    private fun shouldLog(): Boolean = BuildConfig.DEBUG || enabled

    fun d(tag: String, message: String) {
        if (shouldLog()) {
            Log.d(tag, message)
        }
    }

    fun d(tag: String, message: String, error: Throwable) {
        if (shouldLog()) {
            Log.d(tag, message, error)
        }
    }
}
