package to.iris.chat.nearby

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/** Keep blocking native attach/detach off state callbacks; retain the latest request. */
internal class NearbyBleUpdates(
    apply: (enabled: Boolean, nearbyLanEnabled: Boolean) -> Unit,
    onFailure: (Exception) -> Unit,
) : AutoCloseable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val requests = Channel<Pair<Boolean, Boolean>>(Channel.CONFLATED)
    private val worker = scope.launch {
        for ((enabled, nearbyLanEnabled) in requests) {
            ensureActive()
            try {
                apply(enabled, nearbyLanEnabled)
            } catch (error: Exception) {
                onFailure(error)
            }
        }
    }

    fun update(enabled: Boolean, nearbyLanEnabled: Boolean) {
        requests.trySend(enabled to nearbyLanEnabled)
    }

    override fun close() {
        requests.close()
        try {
            runBlocking { worker.cancelAndJoin() }
        } finally {
            scope.cancel()
        }
    }
}
