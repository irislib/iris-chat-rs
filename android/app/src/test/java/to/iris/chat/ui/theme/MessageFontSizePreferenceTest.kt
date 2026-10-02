package to.iris.chat.ui.theme

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.intPreferencesKey
import java.io.IOException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class MessageFontSizePreferenceTest {
    @Test fun immediateSetWaitsForTheExistingCollectorsRealInitialRead() = runTest {
        val store = PausedInitialReadStore()
        val preference = MessageFontSizePreference(store, backgroundScope)
        runCurrent()
        assertTrue(store.readStarted.isCompleted)

        val write = preference.set(MessageFontSize.ExtraLarge)
        runCurrent()
        assertEquals(MessageFontSize.Normal, preference.size.value)
        assertEquals(0, store.writes)
        assertFalse(write.isCompleted)

        store.releaseRead.complete(Unit)
        runCurrent()
        write.join()
        assertEquals(1, store.collectors)
        assertEquals(1, store.writes)
        assertEquals(28, store.values.value[intPreferencesKey("message_font_size")])
        assertEquals(MessageFontSize.ExtraLarge, preference.size.value)
    }

    @Test fun failedInitialReadFailsWaitingSetWithoutWritingOrHanging() = runTest {
        val failure = IOException("initial preference read failed")
        val uncaught = mutableListOf<Throwable>()
        val scope = CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler) +
            CoroutineExceptionHandler { _, error -> uncaught += error })
        try {
            val store = PausedInitialReadStore()
            val preference = MessageFontSizePreference(store, scope)
            val write = preference.set(MessageFontSize.ExtraLarge)
            var writeFailure: Throwable? = null
            write.invokeOnCompletion { writeFailure = it }
            runCurrent()
            assertFalse(write.isCompleted)

            store.releaseRead.completeExceptionally(failure)
            runCurrent()
            assertTrue(write.isCompleted)
            assertTrue(writeFailure is IOException)
            assertEquals(failure.message, writeFailure?.message)
            assertEquals(1, uncaught.size)
            assertTrue(uncaught.single() is IOException)
            assertEquals(failure.message, uncaught.single().message)
            assertEquals(0, store.writes)
            assertEquals(1, store.collectors)
            assertEquals(MessageFontSize.Normal, preference.size.value)
        } finally {
            scope.cancel()
        }
    }

    private class PausedInitialReadStore : DataStore<Preferences> {
        val readStarted = CompletableDeferred<Unit>()
        val releaseRead = CompletableDeferred<Unit>()
        val values = MutableStateFlow(emptyPreferences())
        var collectors = 0
        var writes = 0

        override val data = flow {
            collectors++
            readStarted.complete(Unit)
            releaseRead.await()
            emitAll(values)
        }

        override suspend fun updateData(transform: suspend (Preferences) -> Preferences): Preferences {
            writes++
            return transform(values.value).also { values.value = it }
        }
    }
}
