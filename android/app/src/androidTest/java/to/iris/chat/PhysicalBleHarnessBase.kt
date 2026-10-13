package to.iris.chat

import android.Manifest
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.DeliveryState

/** Shared physical Bluetooth setup and application-level receipt checks. */
abstract class PhysicalBleHarnessBase : RealRelayHarnessBase() {
    protected fun acceptAndMarkIncomingMessageSeen(expectedMessage: String, timeoutMs: Long): String {
        // Fresh accounts keep read receipts private by default. This physical
        // test explicitly verifies the return receipt, so opt its account in.
        appManager().dispatch(AppAction.SetReadReceiptsEnabled(true))
        waitForState("physical peer read receipts enabled", timeoutMs = 30_000) {
            true.takeIf { appManager().state.value.preferences.sendReadReceipts }
        }

        val chatId =
            waitForState("incoming message", timeoutMs = timeoutMs) {
                val state = appManager().state.value
                state.currentChat
                    ?.takeIf { chat ->
                        chat.messages.any { message ->
                            !message.isOutgoing && message.body == expectedMessage
                        }
                    }
                    ?.chatId
                    ?: state.chatList
                        .firstOrNull { thread ->
                            thread.lastMessageIsOutgoing == false &&
                                thread.lastMessagePreview == expectedMessage
                        }
                        ?.chatId
            }

        appManager().openChat(chatId)
        val messageIds =
            waitForState("incoming message in open chat", timeoutMs = 30_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                    ?.messages
                    ?.filter { message ->
                        !message.isOutgoing && message.body == expectedMessage
                    }
                    ?.map { message -> message.id }
                    ?.takeIf { ids -> ids.isNotEmpty() }
            }

        appManager().dispatch(AppAction.SetMessageRequestAccepted(chatId))
        waitForState("message request accepted", timeoutMs = 30_000) {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                ?.let { chat -> true.takeIf { !chat.isRequest } }
        }
        appManager().dispatch(AppAction.MarkMessagesSeen(chatId, messageIds))
        waitForState("incoming message marked seen", timeoutMs = 30_000) {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                ?.messages
                ?.filter { message -> message.id in messageIds }
                ?.takeIf { messages ->
                    messages.isNotEmpty() &&
                        messages.all { message -> message.delivery == DeliveryState.SEEN }
                }
        }
        return chatId
    }

    protected fun enableFipsBleForPhysicalPeer() {
        val permissions =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                listOf(
                    Manifest.permission.BLUETOOTH_SCAN,
                    Manifest.permission.BLUETOOTH_CONNECT,
                    Manifest.permission.BLUETOOTH_ADVERTISE,
                )
            } else {
                listOf(Manifest.permission.ACCESS_FINE_LOCATION)
            }
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        permissions.forEach { permission ->
            runCatching {
                instrumentation.uiAutomation.grantRuntimePermission(appPackageName(), permission)
            }
        }
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(false))
        waitForState("FIPS Bluetooth reset", timeoutMs = 30_000) {
            appManager().state.value.preferences.takeIf { preferences ->
                !preferences.nearbyBluetoothEnabled
            }
        }
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(true))
        waitForState("FIPS Bluetooth enabled", timeoutMs = 30_000) {
            appManager().state.value.preferences.takeIf { preferences ->
                preferences.nearbyEnabled && preferences.nearbyBluetoothEnabled
            }
        }
    }

}
