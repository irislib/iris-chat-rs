package org.fips.ble

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeNotNull
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Opt-in radio probe paired with AppleBlePhysicalDiscoveryTests, without an Iris account. */
class PhysicalBleDiscoveryTest {
    @Test
    fun testDiscoversAppleAndExchangesBytes() {
        val token = InstrumentationRegistry.getArguments().getString("probeToken")
        assumeNotNull(token)
        requireNotNull(token)
        require(token.matches(Regex("[a-f0-9]{32}")))
        val prefix = "iris-ble-probe:$token:".toByteArray()
        val hello = prefix + "android".toByteArray()
        val expected = prefix + "apple".toByteArray()
        val discovered = CountDownLatch(1)
        val received = CountDownLatch(1)
        val remoteReady = CountDownLatch(1)
        val errors = mutableListOf<String>()
        val bootstrapSizes = mutableListOf<Int>()
        val buffers = mutableMapOf<Long, ByteArray>()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val platform = AndroidFipsBlePlatform(context)
        // A second app can expose the same UUID on the same radio. Publish its
        // unrelated bootstrap first so first-service-only discovery fails.
        val otherApp = AndroidFipsBlePlatform(context)
        val otherAppReady = CountDownLatch(1)
        otherApp.setEventSink { event ->
            when (event) {
                is HostBleEvent.Listening -> otherApp.submit(HostBleCommand.StartAdvertising(2, "ignored!".toByteArray()))
                is HostBleEvent.AdvertisingStarted -> otherAppReady.countDown()
                else -> Unit
            }
        }
        var request = 10L
        var connecting = false
        platform.setEventSink { event ->
            when (event) {
                is HostBleEvent.Listening -> platform.submit(
                    HostBleCommand.StartAdvertising(2, prefix + byteArrayOf('A'.code.toByte(),
                        (event.psm and 255).toByte(), (event.psm shr 8).toByte())),
                )
                is HostBleEvent.PeerDiscovered -> {
                    val bytes = event.bootstrap
                    synchronized(bootstrapSizes) { bootstrapSizes.add(bytes.size) }
                    if (bytes.size == prefix.size + 3 && bytes.take(prefix.size).toByteArray().contentEquals(prefix)
                        && bytes[prefix.size] == 'I'.code.toByte()) {
                        discovered.countDown()
                        if (!connecting) {
                            connecting = true
                            val psm = (bytes[prefix.size + 1].toInt() and 255) or
                                ((bytes[prefix.size + 2].toInt() and 255) shl 8)
                            platform.submit(HostBleCommand.Connect(4, event.peerToken, psm))
                        }
                    }
                }
                is HostBleEvent.Connected -> platform.submit(HostBleCommand.Write(request++, event.connectionId, hello))
                is HostBleEvent.IncomingConnection -> platform.submit(HostBleCommand.Write(request++, event.connectionId, hello))
                is HostBleEvent.BytesReceived -> {
                    val bytes = (buffers[event.connectionId] ?: byteArrayOf()) + event.bytes
                    if (bytes.size <= 1_024) buffers[event.connectionId] = bytes
                    val text = bytes.toString(Charsets.UTF_8)
                    if (text.startsWith(expected.toString(Charsets.UTF_8))) received.countDown()
                    if (text.contains((prefix + "apple-ready".toByteArray()).toString(Charsets.UTF_8))) remoteReady.countDown()
                }
                is HostBleEvent.Failed -> synchronized(errors) { errors.add("${event.requestId}: ${event.message}") }
                else -> Unit
            }
        }
        try {
            otherApp.submit(HostBleCommand.Listen(1, 0))
            assertTrue("Companion service did not start", otherAppReady.await(10, TimeUnit.SECONDS))
            platform.submit(HostBleCommand.Listen(1, 0))
            platform.submit(HostBleCommand.StartScanning(3))
            val found = discovered.await(90, TimeUnit.SECONDS)
            assertTrue("Apple discovery failed: ${synchronized(errors) { errors.toList() }}; " +
                "bootstrap sizes=${synchronized(bootstrapSizes) { bootstrapSizes.toList() }}", found)
            val exchanged = received.await(20, TimeUnit.SECONDS)
            assertTrue("Apple data missing: ${synchronized(errors) { errors.toList() }}", exchanged)
            assertTrue("Apple did not confirm its own discovery", remoteReady.await(40, TimeUnit.SECONDS))
            // Keep the peer alive long enough for the other test to finish its read.
            Thread.sleep(5_000)
        } finally {
            platform.close()
            otherApp.close()
        }
    }
}
