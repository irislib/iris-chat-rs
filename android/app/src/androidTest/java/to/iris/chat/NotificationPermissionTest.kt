package to.iris.chat

import android.Manifest
import android.accessibilityservice.AccessibilityServiceInfo
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.os.Build
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.rust.AppAction

@RunWith(AndroidJUnit4::class)
class NotificationPermissionTest {
    @Test
    fun asksAfterAccountSetupAndRespectsDenialAcrossRestarts() {
        assumeTrue(Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU)
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.serviceInfo = instrumentation.uiAutomation.serviceInfo.apply {
            flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
        }
        val context = instrumentation.targetContext
        // Run on a fresh test install without granting permissions at installation.
        assumeTrue(context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
        val manager = (context.applicationContext as IrisChatApp).container.appManager
        manager.resetForUiTestsBlocking()
        val preferences = context.getSharedPreferences("iris_runtime_permissions", Context.MODE_PRIVATE)
        preferences.edit().remove("notifications").commit()

        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            waitUntil { manager.bootstrapState.value == AccountBootstrapState.NeedsLogin }
            instrumentation.waitForIdleSync()
            assertFalse("Do not ask on the welcome screen", notificationDialogVisible())
            assertFalse(preferences.getBoolean("notifications", false))
            screenshot("notification-welcome")

            scenario.onActivity { manager.dispatch(AppAction.CreateAccount("Notification test")) }
            waitUntil { manager.bootstrapState.value is AccountBootstrapState.LoggedIn }
            waitUntil { notificationDialogVisible() }
            assertTrue(preferences.getBoolean("notifications", false))
            screenshot("notification-after-account-setup")
            assertTrue(checkNotNull(permissionButton("permission_deny_button"))
                .performAction(AccessibilityNodeInfo.ACTION_CLICK))
            waitUntil { !notificationDialogVisible() }
            assertTrue(context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)

            scenario.moveToState(Lifecycle.State.CREATED)
            scenario.moveToState(Lifecycle.State.RESUMED)
            scenario.recreate()
            instrumentation.waitForIdleSync()
            assertFalse("Do not repeat the prompt after denial", notificationDialogVisible())
        }

        // A new activity must also honor the persisted decision.
        ActivityScenario.launch(MainActivity::class.java).use {
            instrumentation.waitForIdleSync()
            assertFalse(notificationDialogVisible())
            screenshot("notification-denial-respected")
        }

        // Older versions asked without storing a flag. Honor the OS denial too.
        preferences.edit().remove("notifications").commit()
        ActivityScenario.launch(MainActivity::class.java).use {
            waitUntil { preferences.getBoolean("notifications", false) }
            assertFalse("Do not ask again after an older version's denial", notificationDialogVisible())
        }
    }

    private fun notificationDialogVisible() = permissionButton("permission_deny_button") != null

    private fun permissionButton(id: String): AccessibilityNodeInfo? {
        val root = InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: return null
        val nodes = ArrayDeque<AccessibilityNodeInfo>()
        nodes.add(root)
        while (nodes.isNotEmpty()) {
            val node = nodes.removeFirst()
            if (node.viewIdResourceName?.endsWith(":id/$id") == true) return node
            for (index in 0 until node.childCount) node.getChild(index)?.let(nodes::add)
        }
        return null
    }

    private fun waitUntil(condition: () -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + 20_000
        while (!condition() && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50)
        assertTrue("Timed out waiting for account or notification permission state", condition())
    }

    private fun screenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
        try {
            File(instrumentation.targetContext.getExternalFilesDir("screenshots"), "$name.png")
                .outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        } finally {
            bitmap.recycle()
        }
    }
}
