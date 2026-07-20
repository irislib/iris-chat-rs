package to.iris.chat.nearby

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.fips.ble.AndroidFipsBlePlatform
import org.fips.ble.FipsBleCommandRunner
import org.fips.ble.HostBleCommand
import org.fips.ble.HostBleEvent
import org.fips.ble.HostBleEventSink
import to.iris.chat.IrisDebugLog
import to.iris.chat.rust.FfiApp
import to.iris.chat.rust.FfiFipsBle
import to.iris.chat.rust.FipsBleCommand as RustCommand
import to.iris.chat.rust.FipsBleEvent as RustEvent

/** Thin Iris mapper between generated UniFFI types and the reusable FIPS Android adapter. */
class IrisFipsBleRuntime(
    context: Context,
    app: FfiApp,
) : AutoCloseable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val bridge = FfiFipsBle(app = app)
    private val runner =
        FipsBleCommandRunner(
            AndroidFipsBlePlatform(context),
            HostBleEventSink { event ->
                val accepted = bridge.emit(event.toRust())
                IrisDebugLog.d(TAG, "${event.debugSummary()} accepted=$accepted")
            },
        )
    private val pump: Job =
        scope.launch {
            while (isActive) {
                val command = bridge.nextCommand(timeoutMs = 1_000u)
                if (command == null) {
                    // A timeout is normally one second. If Rust has closed the
                    // channel, keep this command pump from becoming a busy loop.
                    delay(50)
                    continue
                }
                val platformCommand = command.toPlatform()
                IrisDebugLog.d(TAG, "command ${platformCommand.debugSummary()}")
                runner.submit(platformCommand)
            }
        }

    override fun close() {
        pump.cancel()
        runner.close()
        bridge.detach()
        scope.cancel()
    }

    private companion object {
        const val TAG = "IrisFipsBle"
    }
}

private fun HostBleEvent.debugSummary(): String =
    when (this) {
        is HostBleEvent.Listening -> "listening psm=$psm"
        is HostBleEvent.AdvertisingStarted -> "advertising started"
        is HostBleEvent.AdvertisingStopped -> "advertising stopped"
        is HostBleEvent.ScanningStarted -> "scanning started"
        is HostBleEvent.PeerDiscovered -> "peer discovered"
        is HostBleEvent.Connected -> "connected send_mtu=$sendSegmentMtu receive_mtu=$receiveSegmentMtu"
        is HostBleEvent.IncomingConnection ->
            "incoming connected send_mtu=$sendSegmentMtu receive_mtu=$receiveSegmentMtu"
        is HostBleEvent.BytesReceived -> "received bytes=${bytes.size}"
        is HostBleEvent.WriteCompleted -> "write completed"
        is HostBleEvent.Disconnected -> "disconnected reason=${reason ?: "none"}"
        is HostBleEvent.Failed -> "failed message=$message"
    }

private fun HostBleCommand.debugSummary(): String =
    when (this) {
        is HostBleCommand.Listen -> "listen"
        HostBleCommand.StopListening -> "stop listening"
        is HostBleCommand.StartAdvertising -> "start advertising bootstrap_bytes=${bootstrap.size}"
        is HostBleCommand.StopAdvertising -> "stop advertising"
        is HostBleCommand.StartScanning -> "start scanning"
        HostBleCommand.StopScanning -> "stop scanning"
        is HostBleCommand.Connect -> "connect"
        is HostBleCommand.Write -> "write connection=$connectionId bytes=${bytes.size}"
        is HostBleCommand.Close -> "close connection=$connectionId"
    }

private fun RustCommand.toPlatform(): HostBleCommand =
    when (this) {
        is RustCommand.Listen -> HostBleCommand.Listen(requestId.toLong(), preferredPsm.toInt())
        RustCommand.StopListening -> HostBleCommand.StopListening
        is RustCommand.StartAdvertising -> HostBleCommand.StartAdvertising(requestId.toLong(), bootstrap)
        is RustCommand.StopAdvertising -> HostBleCommand.StopAdvertising(requestId.toLong())
        is RustCommand.StartScanning -> HostBleCommand.StartScanning(requestId.toLong())
        RustCommand.StopScanning -> HostBleCommand.StopScanning
        is RustCommand.Connect -> HostBleCommand.Connect(requestId.toLong(), peerToken, psm.toInt())
        is RustCommand.Write -> HostBleCommand.Write(requestId.toLong(), connectionId.toLong(), bytes)
        is RustCommand.Close -> HostBleCommand.Close(connectionId.toLong())
    }

private fun HostBleEvent.toRust(): RustEvent =
    when (this) {
        is HostBleEvent.Listening -> RustEvent.Listening(requestId.toULong(), psm.toUShort())
        is HostBleEvent.AdvertisingStarted -> RustEvent.AdvertisingStarted(requestId.toULong())
        is HostBleEvent.AdvertisingStopped -> RustEvent.AdvertisingStopped(requestId.toULong())
        is HostBleEvent.ScanningStarted -> RustEvent.ScanningStarted(requestId.toULong())
        is HostBleEvent.PeerDiscovered -> RustEvent.PeerDiscovered(peerToken, bootstrap)
        is HostBleEvent.Connected ->
            RustEvent.Connected(
                requestId.toULong(),
                connectionId.toULong(),
                peerToken,
                sendSegmentMtu.toUShort(),
                receiveSegmentMtu.toUShort(),
            )
        is HostBleEvent.IncomingConnection ->
            RustEvent.IncomingConnection(
                connectionId.toULong(),
                peerToken,
                sendSegmentMtu.toUShort(),
                receiveSegmentMtu.toUShort(),
            )
        is HostBleEvent.BytesReceived -> RustEvent.BytesReceived(connectionId.toULong(), bytes)
        is HostBleEvent.WriteCompleted -> RustEvent.WriteCompleted(requestId.toULong())
        is HostBleEvent.Disconnected -> RustEvent.Disconnected(connectionId.toULong(), reason)
        is HostBleEvent.Failed -> RustEvent.Failed(requestId.toULong(), message)
    }
