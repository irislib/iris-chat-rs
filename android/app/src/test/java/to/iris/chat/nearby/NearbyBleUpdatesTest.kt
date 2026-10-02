package to.iris.chat.nearby

import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NearbyBleUpdatesTest {
    @Test
    fun blockingNativeWorkRunsOffCallerAndConflatesPendingSettings() {
        val caller = Thread.currentThread()
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val latestApplied = CountDownLatch(1)
        val applied = Collections.synchronizedList(mutableListOf<Pair<Boolean, Boolean>>())
        val failures = Collections.synchronizedList(mutableListOf<Exception>())
        val updates = NearbyBleUpdates({ enabled, lan ->
            assertFalse("native work must leave the callback thread", Thread.currentThread() === caller)
            applied += enabled to lan
            if (applied.size == 1) {
                started.countDown()
                check(release.await(5, TimeUnit.SECONDS))
            } else {
                latestApplied.countDown()
            }
        }, failures::add)
        try {
            updates.update(enabled = true, nearbyLanEnabled = false)
            assertTrue(started.await(5, TimeUnit.SECONDS))
            updates.update(enabled = true, nearbyLanEnabled = true)
            updates.update(enabled = false, nearbyLanEnabled = true)
            updates.update(enabled = false, nearbyLanEnabled = false)
            release.countDown()
            assertTrue(latestApplied.await(5, TimeUnit.SECONDS))
        } finally {
            release.countDown()
            updates.close()
        }
        updates.update(enabled = true, nearbyLanEnabled = true)
        assertEquals(listOf(true to false, false to false), applied)
        assertTrue(failures.isEmpty())
    }

    @Test
    fun shutdownJoinsInFlightWorkAndDiscardsPendingOrLateUpdates() {
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val closeEntered = CountDownLatch(1)
        val closeReturned = CountDownLatch(1)
        val applied = Collections.synchronizedList(mutableListOf<Pair<Boolean, Boolean>>())
        val failures = Collections.synchronizedList(mutableListOf<Exception>())
        val updates = NearbyBleUpdates({ enabled, lan ->
            applied += enabled to lan
            started.countDown()
            check(release.await(5, TimeUnit.SECONDS))
        }, failures::add)
        val worker = Executors.newSingleThreadExecutor()
        try {
            updates.update(enabled = true, nearbyLanEnabled = false)
            assertTrue(started.await(5, TimeUnit.SECONDS))
            updates.update(enabled = true, nearbyLanEnabled = true)
            val closing = worker.submit {
                closeEntered.countDown()
                updates.close()
                closeReturned.countDown()
            }
            assertTrue(closeEntered.await(5, TimeUnit.SECONDS))
            assertFalse("shutdown waits for native work", closeReturned.await(200, TimeUnit.MILLISECONDS))
            release.countDown()
            closing.get(5, TimeUnit.SECONDS)
            updates.update(enabled = true, nearbyLanEnabled = true)
            assertEquals(listOf(true to false), applied)
            assertTrue(failures.isEmpty())
        } finally {
            release.countDown()
            worker.shutdownNow()
            assertTrue(worker.awaitTermination(5, TimeUnit.SECONDS))
            updates.close()
        }
    }
}
