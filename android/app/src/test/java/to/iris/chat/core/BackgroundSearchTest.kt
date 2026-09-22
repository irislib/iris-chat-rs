package to.iris.chat.core

import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.chat.rust.SearchResultSnapshot

class BackgroundSearchTest {
    @Test
    fun blockingLookupRunsOnWorkerAndPreservesRequest() = runBlocking {
        Executors.newSingleThreadExecutor().asCoroutineDispatcher().use { worker ->
            val caller = Thread.currentThread()
            var lookupThread: Thread? = null
            var requestedLimit: UInt? = null
            val search = BackgroundSearch(
                worker,
                lookup = { query, scope, limit ->
                    lookupThread = Thread.currentThread()
                    requestedLimit = limit
                    result(query, scope)
                },
                onFailure = { _, _, error -> throw AssertionError(error) },
            )

            val actual = search.search("needle", "chat-1", 20u)

            assertEquals("needle", actual.query)
            assertEquals("chat-1", actual.scopeChatId)
            assertEquals(20u, requestedLimit)
            assertTrue(lookupThread != null)
            assertFalse(lookupThread === caller)
        }
    }

    @Test
    fun cancelledLookupFinishesBeforeClearAndCannotDeliverStaleResults() = runBlocking {
        Executors.newFixedThreadPool(2).asCoroutineDispatcher().use { worker ->
            val started = CountDownLatch(1)
            val release = CountDownLatch(1)
            val clearStarted = CountDownLatch(1)
            val searched = mutableListOf<String>()
            val delivered = mutableListOf<String>()
            val search = BackgroundSearch(
                worker,
                lookup = { query, scope, _ ->
                    if (query == "old") {
                        started.countDown()
                        check(release.await(5, TimeUnit.SECONDS))
                    } else {
                        clearStarted.countDown()
                    }
                    // Record completion, not entry: an unlocked clear could
                    // otherwise be overwritten by the blocked old request.
                    searched += query
                    result(query, scope)
                },
                onFailure = { _, _, error -> throw AssertionError(error) },
            )
            val oldSearch = launch {
                delivered += search.search("old", null, 50u).query
            }
            try {
                // Let the caller dispatch the blocking lookup before waiting.
                kotlinx.coroutines.yield()
                assertTrue("old lookup starts", started.await(5, TimeUnit.SECONDS))
                oldSearch.cancel()
                val clearSearch = launch {
                    delivered += search.search("", null, 0u).query
                }
                kotlinx.coroutines.yield()
                assertFalse("clear waits for old lookup", clearStarted.await(200, TimeUnit.MILLISECONDS))
                release.countDown()
                oldSearch.join()
                clearSearch.join()

                assertEquals(listOf("old", ""), searched)
                assertEquals(listOf(""), delivered)
            } finally {
                release.countDown()
                oldSearch.cancelAndJoin()
            }
        }
    }

    @Test
    fun failedLookupReturnsEmptyMatchingSnapshotAndReportsFailure() = runBlocking {
        Executors.newSingleThreadExecutor().asCoroutineDispatcher().use { worker ->
            val failure = IllegalStateException("database unavailable")
            var reported: Throwable? = null
            val search = BackgroundSearch(
                worker,
                lookup = { _, _, _ -> throw failure },
                onFailure = { _, _, error -> reported = error },
            )

            val actual = search.search("needle", "chat-1", 20u)

            assertEquals(result("needle", "chat-1"), actual)
            assertTrue(reported === failure)
        }
    }

    private fun result(query: String, scope: String?) = SearchResultSnapshot(
        query = query,
        scopeChatId = scope,
        people = emptyList(),
        contacts = emptyList(),
        groups = emptyList(),
        messages = emptyList(),
        shortcut = null,
    )
}
