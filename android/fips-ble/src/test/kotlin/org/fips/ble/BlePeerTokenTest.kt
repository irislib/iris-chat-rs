package org.fips.ble

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class BlePeerTokenTest {
    @Test
    fun appsOnOnePhoneRemainDistinctCandidatesWithTheSameRadioAddress() {
        val address = "AA:BB:CC:DD:EE:FF"
        val first = BlePeerToken.forCharacteristic(address, 17)
        val second = BlePeerToken.forCharacteristic(address, 23)
        assertNotEquals(first, second)
        assertEquals(address, BlePeerToken.deviceAddress(first))
        assertEquals(address, BlePeerToken.deviceAddress(second))
        // The Rust host's opaque-token contract excludes slashes and whitespace.
        assertTrue(first.matches(Regex("[a-zA-Z0-9_.:-]{1,128}")))
    }

    @Test
    fun incomingConnectionsStillResolveTheirPlainDeviceAddress() {
        val address = "AA:BB:CC:DD:EE:FF"
        assertEquals(address, BlePeerToken.deviceAddress(address))
    }
}
