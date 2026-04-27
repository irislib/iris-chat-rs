package to.iris.chat

import android.app.Application
import to.iris.chat.core.AppContainer

class IrisChatApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        container = AppContainer(this)
    }
}
