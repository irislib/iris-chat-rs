package to.iris.chat.push

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

class CallPushDeliveryTest {
    @Test
    fun unavailableBootstrapStillReachesCoreWithoutShowingUnauthenticatedCall() = runBlocking {
        val actions = mutableListOf<String>()
        deliverCallPush<String>(
            resolve = {
                actions += "preview-timeout"
                null
            },
            present = { actions += "show-call" },
            ingest = { actions += "ingest-encrypted-payload" },
        )
        assertEquals(listOf("preview-timeout", "ingest-encrypted-payload"), actions)
    }

    @Test
    fun authenticatedPreviewStartsCallBeforeReleasingWakeLease() = runBlocking {
        val actions = mutableListOf<String>()
        deliverCallPush(
            resolve = {
                actions += "resolve"
                "authenticated-call"
            },
            present = { actions += "show-$it" },
            ingest = { actions += "ingest-encrypted-payload" },
        )
        assertEquals(listOf("resolve", "show-authenticated-call", "ingest-encrypted-payload"), actions)
    }

    @Test
    fun previewFailureStillHandsCiphertextToRecovery() = runBlocking {
        val failure = IllegalStateException("preview unavailable")
        var ingestions = 0
        val result = runCatching {
            deliverCallPush<String>(
                resolve = { throw failure },
                present = { error("Unauthenticated call must not appear") },
                ingest = { ingestions++ },
            )
        }
        assertSame(failure, result.exceptionOrNull())
        assertEquals(1, ingestions)
    }
}
