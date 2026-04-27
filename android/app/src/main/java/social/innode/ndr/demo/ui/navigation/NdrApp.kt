package social.innode.ndr.demo.ui.navigation

import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import social.innode.ndr.demo.account.AccountBootstrapState
import social.innode.ndr.demo.core.AppContainer
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.Screen
import social.innode.ndr.demo.ui.screens.ChatListScreen
import social.innode.ndr.demo.ui.screens.ChatScreen
import social.innode.ndr.demo.ui.screens.CreateAccountScreen
import social.innode.ndr.demo.ui.screens.CreateInviteScreen
import social.innode.ndr.demo.ui.screens.DeviceRevokedScreen
import social.innode.ndr.demo.ui.screens.DeviceRosterScreen
import social.innode.ndr.demo.ui.screens.GroupDetailsScreen
import social.innode.ndr.demo.ui.screens.JoinInviteScreen
import social.innode.ndr.demo.ui.screens.NewChatScreen
import social.innode.ndr.demo.ui.screens.NewGroupScreen
import social.innode.ndr.demo.ui.screens.MyProfileSheet
import social.innode.ndr.demo.ui.screens.RestoreAccountScreen
import social.innode.ndr.demo.ui.screens.SplashScreen
import social.innode.ndr.demo.ui.screens.SplashViewModel
import social.innode.ndr.demo.ui.screens.AwaitingDeviceApprovalScreen
import social.innode.ndr.demo.ui.screens.AddDeviceScreen
import social.innode.ndr.demo.ui.screens.WelcomeScreen

@Composable
fun NdrApp(container: AppContainer) {
    val appManager = container.appManager
    val splashViewModel = remember { SplashViewModel(appManager) }
    val bootstrapState by splashViewModel.bootstrapState.collectAsStateWithLifecycle()
    val appState by appManager.state.collectAsStateWithLifecycle()
    val context = LocalContext.current

    LaunchedEffect(appState.toast) {
        val message = appState.toast ?: return@LaunchedEffect
        Toast.makeText(context, message, Toast.LENGTH_LONG).show()
    }

    val router = appState.router
    val activeScreen = router.screenStack.lastOrNull() ?: router.defaultScreen

    BackHandler(enabled = bootstrapState != AccountBootstrapState.Loading && router.screenStack.isNotEmpty()) {
        appManager.dispatch(AppAction.UpdateScreenStack(router.screenStack.dropLast(1)))
    }

    Box {
        when (bootstrapState) {
            AccountBootstrapState.Loading -> {
                SplashScreen(
                    bootstrapState = bootstrapState,
                    onNeedsLogin = {},
                    onLoggedIn = {},
                )
            }

            AccountBootstrapState.NeedsLogin -> {
                when (activeScreen) {
                    Screen.Welcome -> WelcomeScreen(appManager = appManager)
                    Screen.CreateAccount -> CreateAccountScreen(appManager = appManager, appState = appState)
                    Screen.RestoreAccount -> RestoreAccountScreen(appManager = appManager, appState = appState)
                    Screen.AddDevice -> AddDeviceScreen(appManager = appManager, appState = appState, awaitingApproval = false)
                    else -> WelcomeScreen(appManager = appManager)
                }
            }

            is AccountBootstrapState.LoggedIn -> {
                when (val screen = activeScreen) {
                    Screen.Welcome -> {
                        WelcomeScreen(appManager = appManager)
                    }

                    Screen.CreateAccount -> {
                        CreateAccountScreen(appManager = appManager, appState = appState)
                    }

                    Screen.RestoreAccount -> {
                        RestoreAccountScreen(appManager = appManager, appState = appState)
                    }

                    Screen.AddDevice -> {
                        AddDeviceScreen(appManager = appManager, appState = appState, awaitingApproval = false)
                    }

                    Screen.ChatList -> {
                        ChatListScreen(appManager = appManager, appState = appState)
                    }

                    Screen.NewChat -> {
                        NewChatScreen(appManager = appManager, appState = appState)
                    }

                    Screen.NewGroup -> {
                        NewGroupScreen(appManager = appManager, appState = appState)
                    }

                    Screen.CreateInvite -> {
                        CreateInviteScreen(appManager = appManager, appState = appState)
                    }

                    Screen.JoinInvite -> {
                        JoinInviteScreen(appManager = appManager, appState = appState)
                    }

                    Screen.Settings -> {
                        val account = appState.account
                        if (account == null) {
                            ChatListScreen(appManager = appManager, appState = appState)
                        } else {
                            MyProfileSheet(
                                appManager = appManager,
                                npub = account.npub,
                                displayName = account.displayName,
                                pictureUrl = account.pictureUrl,
                                deviceNpub = account.deviceNpub,
                                canManageDevices = account.hasOwnerSigningAuthority,
                                sendTypingIndicators = appState.preferences.sendTypingIndicators,
                                sendReadReceipts = appState.preferences.sendReadReceipts,
                                desktopNotificationsEnabled = appState.preferences.desktopNotificationsEnabled,
                                imageProxyEnabled = appState.preferences.imageProxyEnabled,
                                imageProxyUrl = appState.preferences.imageProxyUrl,
                                imageProxyKeyHex = appState.preferences.imageProxyKeyHex,
                                imageProxySaltHex = appState.preferences.imageProxySaltHex,
                                preferences = appState.preferences,
                                networkStatus = appState.networkStatus,
                                onManageDevices = { appManager.pushScreen(Screen.DeviceRoster) },
                                onLogout = { appManager.logout() },
                                onDismiss = {
                                    appManager.dispatch(
                                        AppAction.UpdateScreenStack(router.screenStack.dropLast(1)),
                                    )
                                },
                            )
                        }
                    }

                    Screen.DeviceRoster -> {
                        DeviceRosterScreen(appManager = appManager, appState = appState)
                    }

                    Screen.AwaitingDeviceApproval -> {
                        AwaitingDeviceApprovalScreen(appManager = appManager, appState = appState)
                    }

                    Screen.DeviceRevoked -> {
                        DeviceRevokedScreen(appManager = appManager, appState = appState)
                    }

                    is Screen.Chat -> {
                        // ChatScreen takes only `(appManager, chatId)` and
                        // collects its own state slices internally. Passing
                        // `appState` here would invalidate ChatScreen's
                        // memoization on every relay event.
                        ChatScreen(
                            appManager = appManager,
                            chatId = screen.chatId,
                        )
                    }

                    is Screen.GroupDetails -> {
                        GroupDetailsScreen(
                            appManager = appManager,
                            appState = appState,
                            groupId = screen.groupId,
                        )
                    }
                }
            }
        }
    }
}
