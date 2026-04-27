package to.iris.chat.ui.screens

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.StateFlow
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.core.AppManager

class SplashViewModel(
    appManager: AppManager,
) : ViewModel() {
    val bootstrapState: StateFlow<AccountBootstrapState> = appManager.bootstrapState
}
