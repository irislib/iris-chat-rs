package to.iris.chat.calls

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CallFramePacerTest {
    @Test fun fractionalCameraRatesKeepTheRequestedLowBandwidthCadence() {
        for (camera in listOf(24, 25, 30, 60)) {
            val pacer = CallFramePacer()
            val count = (0 until camera * 10).count { pacer.accept(1_000_000_000L + it * 1_000_000_000L / camera, 10) }
            assertTrue("$camera fps camera should produce 100 frames, got $count", count in 99..100)
        }
    }
    @Test fun tierChangesPausesAndCameraClockResetsDoNotStallOrBurst() {
        val pacer = CallFramePacer()
        assertTrue(pacer.accept(1_000_000_000, 30))
        assertFalse(pacer.accept(1_010_000_000, 30))
        assertTrue(pacer.accept(1_020_000_000, 10))
        assertFalse(pacer.accept(1_080_000_000, 10))
        assertTrue(pacer.accept(5_000_000_000, 10))
        assertFalse(pacer.accept(5_010_000_000, 10))
        assertTrue(pacer.accept(100, 10))
        assertFalse(pacer.accept(200, 10))
    }
}
