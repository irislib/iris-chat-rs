package org.fips.ble

/** FIPS needs a distinct candidate for each app sharing a phone's GATT server. */
internal object BlePeerToken {
    fun forCharacteristic(address: String, instanceId: Int): String = "$address.$instanceId"

    fun deviceAddress(token: String): String = token.substringBefore('.')
}
