package to.iris.chat.ui.screens

import androidx.compose.runtime.Composable
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppState

@Composable
fun AwaitingDeviceApprovalScreen(
    appManager: AppManager,
    appState: AppState,
) {
    AddDeviceScreen(
        appManager = appManager,
        appState = appState,
        awaitingApproval = true,
    )
}
