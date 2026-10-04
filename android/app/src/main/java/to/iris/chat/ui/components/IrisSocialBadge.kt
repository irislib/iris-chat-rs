package to.iris.chat.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Star
import androidx.compose.foundation.border
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import to.iris.chat.rust.SocialBadge
import to.iris.chat.rust.SocialConnectionSnapshot
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun IrisSocialBadge(connection: SocialConnectionSnapshot, modifier: Modifier = Modifier) {
    val badge = connection.badge ?: return
    val color = when (badge) {
        SocialBadge.WARNING -> IrisTheme.palette.accentAlt
        SocialBadge.FOLLOWING -> Color(0xFF0A84FF)
        SocialBadge.FRIEND -> Color(0xFF8E8E93)
        SocialBadge.TRUSTED -> Color(0xFFD4A017)
        SocialBadge.MUTED -> MaterialTheme.colorScheme.error
    }
    Icon(
        if (badge == SocialBadge.WARNING) Icons.Default.PriorityHigh else if (badge == SocialBadge.MUTED) Icons.AutoMirrored.Filled.VolumeOff else Icons.Default.Check,
        contentDescription = connection.description,
        tint = Color.White,
        modifier = modifier.size(16.dp).background(color, CircleShape).padding(2.dp),
    )
}

@Composable
fun IrisSocialConnectionLabel(connection: SocialConnectionSnapshot) {
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        IrisSocialBadge(connection)
        Text(connection.description, style = MaterialTheme.typography.bodySmall, color = IrisTheme.palette.muted)
    }
}

@Composable
fun IrisFavoriteBadge(size: Dp, modifier: Modifier = Modifier) {
    Icon(
        Icons.Default.Star,
        contentDescription = "Favorite",
        tint = Color(0xFF352900),
        modifier = modifier.size(size)
            .background(Color(0xFFFBBF24), CircleShape)
            .border(1.dp, IrisTheme.palette.panel, CircleShape)
            .padding(size * 0.19f)
            .testTag("favoriteAvatarBadge"),
    )
}
