package to.iris.chat.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.ui.theme.IrisTheme

val LocalNearbyAvatarOwners = compositionLocalOf<Set<String>> { emptySet() }

fun nearbyAvatarOwners(snapshot: IrisNearbyService.Snapshot, enabled: Boolean, localOwner: String?): Set<String> {
    if (!enabled || localOwner.isNullOrBlank() || (!snapshot.visible && !snapshot.localNetworkVisible)) return emptySet()
    return snapshot.peers.mapNotNull { it.ownerPubkeyHex?.takeIf { owner -> owner.isNotBlank() && owner != localOwner } }.toSet()
}

@Composable
fun IrisNearbyBadge(size: Dp, modifier: Modifier = Modifier) {
    Icon(
        IrisIcons.Nearby,
        contentDescription = "Nearby",
        tint = Color.White,
        modifier = modifier.size(size)
            .background(IrisTheme.palette.accent, CircleShape)
            .border(1.5.dp, IrisTheme.palette.panel, CircleShape)
            .padding(size * 0.18f)
            .testTag("nearbyAvatarBadge"),
    )
}
