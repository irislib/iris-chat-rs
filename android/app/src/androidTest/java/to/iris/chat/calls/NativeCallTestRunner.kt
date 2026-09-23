package to.iris.chat.calls

import android.app.Application
import android.content.Context
import androidx.test.runner.AndroidJUnitRunner

/** Opt-in runner: fresh FFI accounts only, never starts the installed account. */
class NativeCallTestRunner : AndroidJUnitRunner() {
    override fun newApplication(loader: ClassLoader, className: String, context: Context): Application =
        super.newApplication(loader, Application::class.java.name, context)
}
