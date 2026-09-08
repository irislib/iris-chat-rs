package to.iris.chat.ui.navigation

import to.iris.chat.rust.NetworkStatusSnapshot

internal fun offlineStatusBannerText(
    networkStatus: NetworkStatusSnapshot?,
    isLoggedIn: Boolean,
    foregroundedAtSecs: Long,
    nowSecs: Long,
): String? {
    val deadline = offlineBannerDeadlineSecs(networkStatus, isLoggedIn, foregroundedAtSecs)
    return if (deadline != null && nowSecs >= deadline) "Can’t reach message servers" else null
}

internal fun offlineBannerDeadlineSecs(
    networkStatus: NetworkStatusSnapshot?,
    isLoggedIn: Boolean,
    foregroundedAtSecs: Long,
): Long? {
    if (!isLoggedIn || networkStatus == null || networkStatus.relayUrls.isEmpty() ||
        networkStatus.connectedRelayCount != 0uL
    ) {
        return null
    }
    val offlineSinceSecs = networkStatus.allRelaysOfflineSinceSecs?.toLong() ?: return null
    val connections = networkStatus.relayConnections.filter { it.url in networkStatus.relayUrls }
    if (connections.size != networkStatus.relayUrls.size ||
        connections.any { it.status != "offline" && it.status != "blocked" }
    ) {
        return null
    }
    return maxOf(offlineSinceSecs, foregroundedAtSecs) + OFFLINE_BANNER_GRACE_SECS
}

private const val OFFLINE_BANNER_GRACE_SECS = 30L
