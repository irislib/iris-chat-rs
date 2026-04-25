package social.innode.ndr.demo.ui.screens

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
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisPrimaryButton
import social.innode.ndr.demo.ui.components.IrisSectionCard
import social.innode.ndr.demo.ui.theme.IrisTheme

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
                text = "This device is no longer in the owner-signed roster. Messaging is blocked until you log out and re-link it from a primary device.",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
            )
            IrisPrimaryButton(
                text = "Logout",
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
