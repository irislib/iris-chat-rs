package to.iris.chat.push

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Coalesce state updates while notification decryption runs off the UI thread. */
class ReadNotificationCleanup(
    private val scope: CoroutineScope,
    private val dispatcher: CoroutineDispatcher,
) {
    private var job: Job? = null
    private var pending = false

    @Synchronized
    fun schedule(dismiss: suspend () -> Unit) {
        pending = true
        if (job?.isActive == true) return
        job = scope.launch(dispatcher) {
            while (true) {
                val requested = synchronized(this@ReadNotificationCleanup) {
                    if (pending) {
                        pending = false
                        true
                    } else {
                        job = null
                        false
                    }
                }
                if (!requested) break
                runCatching { dismiss() }.onFailure {
                    android.util.Log.w("IrisPush", "Could not clear read notifications", it)
                }
            }
        }
    }
}
