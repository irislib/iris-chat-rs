package to.iris.chat.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import to.iris.chat.rust.SearchResultSnapshot

/** Keeps database locks and local-history scans off the UI thread. */
internal class BackgroundSearch(
    private val dispatcher: CoroutineDispatcher,
    private val lookup: (String, String?, UInt) -> SearchResultSnapshot,
    private val onFailure: (String, String?, Throwable) -> Unit,
) {
    private val mutex = Mutex()

    suspend fun search(query: String, scopeChatId: String?, limit: UInt): SearchResultSnapshot =
        withContext(dispatcher) {
            // A cancelled blocking lookup must finish before the next query
            // updates the core's global search subscription. withContext also
            // prevents cancelled results from reaching the Compose caller.
            mutex.withLock {
                try {
                    lookup(query, scopeChatId, limit)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    onFailure(query, scopeChatId, error)
                    SearchResultSnapshot(
                        query = query,
                        scopeChatId = scopeChatId,
                        people = emptyList(),
                        contacts = emptyList(),
                        groups = emptyList(),
                        messages = emptyList(),
                        shortcut = null,
                    )
                }
            }
        }
}
