package to.iris.chat.nearby

import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.assertThrows
import org.junit.Test

class NearbyBleSessionTest {
    @Test
    fun changingLanRestartsButRepeatedSettingsKeepTheAttachedBridge() {
        val events = mutableListOf<String>()
        var nextId = 0
        val session = NearbyBleSession {
            val id = ++nextId
            events += "attach $id"
            AutoCloseable { events += "detach $id" }
        }

        session.update(enabled = false, nearbyLanEnabled = false)
        session.update(enabled = true, nearbyLanEnabled = false)
        session.update(enabled = true, nearbyLanEnabled = false)
        session.update(enabled = true, nearbyLanEnabled = true)
        session.update(enabled = true, nearbyLanEnabled = false)
        session.update(enabled = false, nearbyLanEnabled = false)
        session.update(enabled = false, nearbyLanEnabled = true)

        assertEquals(
            listOf("attach 1", "detach 1", "attach 2", "detach 2", "attach 3", "detach 3"),
            events,
        )
        session.update(enabled = true, nearbyLanEnabled = true)
        session.close()
        session.close()
        session.update(enabled = true, nearbyLanEnabled = false)
        assertEquals(listOf("attach 4", "detach 4"), events.takeLast(2))
        assertEquals(4, nextId)
    }

    @Test
    fun failedAttachmentCanRetryTheSameSettings() {
        val failure = IllegalStateException("attachment unavailable")
        var attempts = 0
        var closes = 0
        val session = NearbyBleSession {
            if (++attempts == 1) throw failure
            AutoCloseable { closes++ }
        }

        assertSame(failure, assertThrows(IllegalStateException::class.java) {
            session.update(enabled = true, nearbyLanEnabled = true)
        })
        session.update(enabled = true, nearbyLanEnabled = true)
        session.update(enabled = true, nearbyLanEnabled = true)
        session.close()

        assertEquals(2, attempts)
        assertEquals(1, closes)
    }

    @Test
    fun failedDetachCannotAttachAReplacement() {
        var attachments = 0
        var closes = 0
        val failure = IllegalStateException("detach failed")
        val session = NearbyBleSession {
            attachments++
            AutoCloseable { if (++closes == 1) throw failure }
        }
        session.update(enabled = true, nearbyLanEnabled = false)

        assertSame(failure, assertThrows(IllegalStateException::class.java) {
            session.update(enabled = true, nearbyLanEnabled = true)
        })
        assertEquals(1, attachments)
        // Even returning to the old setting must finish the failed teardown.
        session.update(enabled = true, nearbyLanEnabled = false)
        assertEquals(2, attachments)
        assertEquals(2, closes)
        session.close()
    }

    @Test
    fun terminalCleanupReleasesFailedDetachWithoutAllowingAnotherAttachment() {
        val failure = IllegalStateException("detach failed")
        val runtime = AutoCloseable { throw failure }
        var attachments = 0
        var platformStops = 0
        val session = NearbyBleSession { attachments++; runtime }
        session.update(enabled = true, nearbyLanEnabled = false)

        assertSame(failure, assertThrows(IllegalStateException::class.java) { session.close() })
        session.update(enabled = true, nearbyLanEnabled = true)
        val coreFailure = IllegalStateException("core termination unconfirmed")
        assertSame(coreFailure, assertThrows(IllegalStateException::class.java) {
            session.shutdownCoreAndCleanup({ throw coreFailure }) { platformStops++ }
        })
        assertEquals(0, platformStops)
        session.shutdownCoreAndCleanup({}) {
            assertSame(runtime, it)
            platformStops++
        }
        session.shutdownCoreAndCleanup({}) { platformStops++ }
        session.close()

        assertEquals(1, attachments)
        assertEquals(1, platformStops)
    }

    @Test
    fun failedDetachPlatformRemainsLiveUntilCoreJoinReturns() {
        val joining = CountDownLatch(1)
        val coreExited = CountDownLatch(1)
        val platformStopped = CountDownLatch(1)
        val session = NearbyBleSession { AutoCloseable { error("detach failed") } }
        session.update(enabled = true, nearbyLanEnabled = false)
        assertThrows(IllegalStateException::class.java) { session.close() }
        val worker = Executors.newSingleThreadExecutor()
        try {
            val shutdown = worker.submit {
                session.shutdownCoreAndCleanup({
                    joining.countDown()
                    check(coreExited.await(5, TimeUnit.SECONDS))
                }) { platformStopped.countDown() }
            }
            assertTrue(joining.await(5, TimeUnit.SECONDS))
            assertFalse("platform survives until core exit", platformStopped.await(200, TimeUnit.MILLISECONDS))
            coreExited.countDown()
            shutdown.get(5, TimeUnit.SECONDS)
            assertEquals(0L, platformStopped.count)
        } finally {
            coreExited.countDown()
            worker.shutdownNow()
            assertTrue(worker.awaitTermination(5, TimeUnit.SECONDS))
        }
    }

    @Test
    fun concurrentUpdatesWaitForDetachAndDoNotCreateDuplicateBridges() {
        val detachEntered = CountDownLatch(1)
        val releaseDetach = CountDownLatch(1)
        val secondUpdateEntered = CountDownLatch(1)
        val secondUpdateReturned = CountDownLatch(1)
        var attachments = 0
        var firstDetached = false
        val session = NearbyBleSession {
            val id = ++attachments
            if (id > 1) assertTrue("old detach completes before replacement", firstDetached)
            AutoCloseable {
                if (id == 1) {
                    detachEntered.countDown()
                    check(releaseDetach.await(5, TimeUnit.SECONDS))
                    firstDetached = true
                }
            }
        }
        session.update(enabled = true, nearbyLanEnabled = false)
        val workers = Executors.newFixedThreadPool(2)
        try {
            val first = workers.submit {
                session.update(enabled = true, nearbyLanEnabled = true)
            }
            assertTrue(detachEntered.await(5, TimeUnit.SECONDS))
            val second = workers.submit {
                secondUpdateEntered.countDown()
                session.update(enabled = true, nearbyLanEnabled = true)
                secondUpdateReturned.countDown()
            }
            assertTrue(secondUpdateEntered.await(5, TimeUnit.SECONDS))
            assertFalse("second update waits for teardown", secondUpdateReturned.await(200, TimeUnit.MILLISECONDS))
            releaseDetach.countDown()
            first.get(5, TimeUnit.SECONDS)
            second.get(5, TimeUnit.SECONDS)
            assertEquals(2, attachments)
        } finally {
            releaseDetach.countDown()
            workers.shutdownNow()
            assertTrue(workers.awaitTermination(5, TimeUnit.SECONDS))
            session.close()
        }
    }
}
