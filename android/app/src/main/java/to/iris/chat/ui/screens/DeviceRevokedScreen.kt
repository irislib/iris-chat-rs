package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppState
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.theme.IrisTheme

@Composable
fun DeviceRevokedScreen(
    appManager: AppManager,
    appState: AppState,
) {
    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(16.dp)
                .testTag("deviceRevokedScreen"),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        IrisSectionCard {
            Text(
                text = "Device removed",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text = "This device no longer has access. Sign in again to keep using Iris Chat here.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
            IrisPrimaryButton(
                text = "Sign out",
                onClick = appManager::logout,
                modifier = Modifier.testTag("deviceRevokedLogoutButton"),
                icon = {
                    Icon(
                        imageVector = IrisIcons.Logout,
                        contentDescription = null,
                    )
                },
            )
        }
    }
}
