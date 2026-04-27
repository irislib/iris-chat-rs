package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import to.iris.chat.account.AccountBootstrapState

@Composable
fun SplashScreen(
    bootstrapState: AccountBootstrapState,
    onNeedsLogin: () -> Unit,
    onLoggedIn: () -> Unit,
) {
    LaunchedEffect(bootstrapState) {
        when (bootstrapState) {
            AccountBootstrapState.Loading -> Unit
            AccountBootstrapState.NeedsLogin -> onNeedsLogin()
            is AccountBootstrapState.LoggedIn -> onLoggedIn()
        }
    }

    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            CircularProgressIndicator()
            Text(
                text = "Loading device account…",
                style = MaterialTheme.typography.titleMedium,
            )
        }
    }
}
