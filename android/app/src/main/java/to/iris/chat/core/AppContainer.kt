package to.iris.chat.core

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import to.iris.chat.account.AndroidKeystoreSecretStore
import to.iris.chat.nearby.IrisNearbyService

class AppContainer(context: Context) {
    private val appContext = context.applicationContext
    private val applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    val secureSecretStore: AndroidKeystoreSecretStore = AndroidKeystoreSecretStore()
    val appManager: AppManager
    val nearbyIrisService: IrisNearbyService

    init {
        appManager =
            AppManager(
                context = appContext,
                applicationScope = applicationScope,
                secureSecretStore = secureSecretStore,
            )
        nearbyIrisService = IrisNearbyService(appContext)
        appManager.setFipsNearbyPeersPublisher(nearbyIrisService::applyFipsPeerSnapshot)
    }
}
