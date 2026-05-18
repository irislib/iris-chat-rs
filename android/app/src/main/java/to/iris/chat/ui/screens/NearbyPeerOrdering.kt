package to.iris.chat.ui.screens

import android.os.SystemClock
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import kotlinx.coroutines.delay
import to.iris.chat.nearby.IrisNearbyService

private const val NearbyPeerReorderThrottleMillis = 5_000L

@Composable
internal fun rememberSortedNearbyPeers(
    peers: List<IrisNearbyService.Peer>,
    knownDirectChatIds: Set<String>,
    bluetoothPeerIds: Set<String>,
    localNetworkPeerIds: Set<String>,
): List<IrisNearbyService.Peer> {
    val sortedPeers =
        remember(peers, knownDirectChatIds, bluetoothPeerIds, localNetworkPeerIds) {
            sortNearbyPeers(
                peers = peers,
                knownDirectChatIds = knownDirectChatIds,
                bluetoothPeerIds = bluetoothPeerIds,
                localNetworkPeerIds = localNetworkPeerIds,
            )
        }
    val sortedOrder = remember(sortedPeers) { sortedPeers.map { it.id } }
    var displayedOrder by remember { mutableStateOf(sortedOrder) }
    var lastReorderAtMillis by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }

    LaunchedEffect(sortedOrder) {
        if (sortedOrder == displayedOrder) {
            return@LaunchedEffect
        }

        val now = SystemClock.elapsedRealtime()
        val hasPeerSetChanged = sortedOrder.toSet() != displayedOrder.toSet()
        if (!hasPeerSetChanged) {
            val remaining = NearbyPeerReorderThrottleMillis - (now - lastReorderAtMillis)
            if (remaining > 0L) {
                delay(remaining)
            }
        }

        displayedOrder = sortedOrder
        lastReorderAtMillis = SystemClock.elapsedRealtime()
    }

    val peersById = remember(peers) { peers.associateBy { it.id } }
    return remember(displayedOrder, sortedPeers, peersById) {
        if (displayedOrder.isEmpty()) {
            sortedPeers
        } else {
            val usedIds = mutableSetOf<String>()
            displayedOrder.mapNotNull { id ->
                peersById[id]?.also { usedIds.add(id) }
            } + sortedPeers.filter { usedIds.add(it.id) }
        }
    }
}

internal fun sortNearbyPeers(
    peers: List<IrisNearbyService.Peer>,
    knownDirectChatIds: Set<String>,
    bluetoothPeerIds: Set<String>,
    localNetworkPeerIds: Set<String>,
): List<IrisNearbyService.Peer> =
    peers.sortedWith { left, right ->
        compareNearbyPeers(
            left = left,
            right = right,
            knownDirectChatIds = knownDirectChatIds,
            bluetoothPeerIds = bluetoothPeerIds,
            localNetworkPeerIds = localNetworkPeerIds,
        )
    }

private fun compareNearbyPeers(
    left: IrisNearbyService.Peer,
    right: IrisNearbyService.Peer,
    knownDirectChatIds: Set<String>,
    bluetoothPeerIds: Set<String>,
    localNetworkPeerIds: Set<String>,
): Int =
    compareValues(
        if (left.ownerPubkeyHex.normalizedPeerKey() in knownDirectChatIds) 0 else 1,
        if (right.ownerPubkeyHex.normalizedPeerKey() in knownDirectChatIds) 0 else 1,
    ).takeIf { it != 0 }
        ?: compareValues(
            transportRank(left.id, bluetoothPeerIds, localNetworkPeerIds),
            transportRank(right.id, bluetoothPeerIds, localNetworkPeerIds),
        ).takeIf { it != 0 }
        ?: compareBluetoothRssi(left, right, bluetoothPeerIds).takeIf { it != 0 }
        ?: left.deterministicNearbyKey().compareTo(right.deterministicNearbyKey())
            .takeIf { it != 0 }
        ?: left.id.compareTo(right.id)

private fun compareBluetoothRssi(
    left: IrisNearbyService.Peer,
    right: IrisNearbyService.Peer,
    bluetoothPeerIds: Set<String>,
): Int {
    if (left.id !in bluetoothPeerIds || right.id !in bluetoothPeerIds) {
        return 0
    }
    return (right.bluetoothRssi ?: Int.MIN_VALUE)
        .compareTo(left.bluetoothRssi ?: Int.MIN_VALUE)
}

private fun transportRank(
    peerId: String,
    bluetoothPeerIds: Set<String>,
    localNetworkPeerIds: Set<String>,
): Int =
    when (peerId) {
        in bluetoothPeerIds -> 0
        in localNetworkPeerIds -> 1
        else -> 2
    }

private fun IrisNearbyService.Peer.deterministicNearbyKey(): String =
    ownerPubkeyHex.normalizedPeerKey().ifEmpty { "peer:${id.lowercase()}" }

private fun String?.normalizedPeerKey(): String =
    this?.trim()?.lowercase().orEmpty()
