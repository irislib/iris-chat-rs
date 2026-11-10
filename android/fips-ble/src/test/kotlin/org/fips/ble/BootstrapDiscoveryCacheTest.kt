package org.fips.ble

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BootstrapDiscoveryCacheTest {
    @Test
    fun fullQueueDefersRetriesWithoutSpinningAndResumesWhenAReadFinishes() {
        var now = 0L
        val cache = BootstrapDiscoveryCache(capacity = 2, clock = { now })
        assertTrue(cache.begin("failed"))
        cache.fail("failed")
        assertTrue(cache.begin("a"))
        assertTrue(cache.begin("b"))
        now = 5_000
        assertTrue(cache.dueRetries().isEmpty())
        assertEquals(15_000L, cache.nextWakeupAt())
        assertTrue(cache.complete("a"))
        assertEquals(listOf("failed"), cache.dueRetries())
        assertEquals(5_000L, cache.nextWakeupAt())
    }

    @Test
    fun rememberedFailuresAreBounded() {
        var now = 0L
        val cache = BootstrapDiscoveryCache(capacity = 2, clock = { now })
        for (peer in listOf("a", "b", "c")) {
            assertTrue(cache.begin(peer))
            cache.fail(peer)
        }
        now = 5_000
        assertEquals(listOf("b", "c"), cache.dueRetries())
    }

    @Test
    fun missingCallbackExpiresAndRetriesWithoutAnotherAdvertisement() {
        var now = 1_000L
        val cache = BootstrapDiscoveryCache(clock = { now })
        assertTrue(cache.begin("iphone"))
        assertEquals(16_000L, cache.nextWakeupAt())
        now = 15_999
        assertTrue(cache.expirePending().isEmpty())
        now = 16_000
        assertEquals(listOf("iphone"), cache.expirePending())
        assertFalse(cache.complete("iphone"))
        assertFalse(cache.begin("iphone"))
        assertEquals(21_000L, cache.nextWakeupAt())
        now = 21_000
        assertEquals(listOf("iphone"), cache.dueRetries())
        assertTrue(cache.begin("iphone"))
        assertTrue(cache.complete("iphone"))
        assertNull(cache.nextWakeupAt())
    }

    @Test
    fun repeatedFailuresBackOffButSuccessClearsTheDelay() {
        var now = 0L
        val cache = BootstrapDiscoveryCache(clock = { now })
        for (delay in listOf(5_000L, 10_000L, 20_000L, 40_000L, 60_000L, 60_000L)) {
            assertTrue(cache.begin("iphone"))
            cache.fail("iphone")
            assertEquals(now + delay, cache.nextWakeupAt())
            cache.fail("iphone") // A late disconnect cannot extend the delay.
            cache.invalidate("iphone") // A stale L2CAP PSM cannot bypass it either.
            assertFalse(cache.begin("iphone"))
            now += delay
        }
        assertTrue(cache.begin("iphone"))
        assertTrue(cache.complete("iphone"))
        cache.invalidate("iphone")
        assertTrue(cache.begin("iphone"))
        cache.fail("iphone")
        assertEquals(now + 5_000, cache.nextWakeupAt())
    }

    @Test
    fun fullPendingQueueStillExpiresAndMakesRoomForDiscovery() {
        var now = 0L
        val cache = BootstrapDiscoveryCache(capacity = 2, clock = { now })
        assertTrue(cache.begin("a"))
        assertTrue(cache.begin("b"))
        assertFalse(cache.begin("c"))
        assertEquals(15_000L, cache.nextWakeupAt())
        now = 15_000
        assertEquals(setOf("a", "b"), cache.expirePending().toSet())
        assertTrue(cache.begin("c"))
    }

    @Test
    fun stoppedScanDropsDeadlinesRetriesAndLateCompletions() {
        val cache = BootstrapDiscoveryCache(clock = { 0L })
        assertTrue(cache.begin("a"))
        assertTrue(cache.begin("b"))
        cache.fail("a")
        cache.clear()
        assertFalse(cache.complete("b"))
        cache.fail("b")
        assertNull(cache.nextWakeupAt())
        assertTrue(cache.dueRetries().isEmpty())
        assertTrue(cache.begin("a"))
    }

    @Test
    fun refreshingDoesNotCancelAnInFlightRead() {
        val cache = BootstrapDiscoveryCache()
        assertTrue(cache.begin("iphone"))
        cache.invalidate("iphone")
        assertFalse(cache.begin("iphone"))
        assertTrue(cache.complete("iphone"))
    }

    @Test
    fun connectionFailureMakesACompletedBootstrapEligibleForRefresh() {
        val cache = BootstrapDiscoveryCache()
        val peer = "peer"

        assertTrue(cache.begin(peer))
        assertFalse(cache.begin(peer))
        cache.complete(peer)
        assertFalse(cache.begin(peer))

        cache.invalidate(peer)

        assertTrue(cache.begin(peer))
    }

    @Test
    fun completedBootstrapCacheIsBounded() {
        val cache = BootstrapDiscoveryCache(capacity = 2)

        assertTrue(cache.begin("a"))
        cache.complete("a")
        assertTrue(cache.begin("b"))
        cache.complete("b")
        assertTrue(cache.begin("c"))
        cache.complete("c")
        assertTrue(cache.begin("a"))
    }

    @Test
    fun pendingBootstrapReadsAreBoundedWithoutEviction() {
        val cache = BootstrapDiscoveryCache(capacity = 2)

        assertTrue(cache.begin("a"))
        assertTrue(cache.begin("b"))
        assertFalse(cache.begin("c"))
        assertFalse(cache.begin("a"))

        cache.fail("a")

        assertTrue(cache.begin("c"))
    }
}
