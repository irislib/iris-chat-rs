package to.iris.chat

import android.view.KeyEvent as AndroidKeyEvent
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsNotFocused
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyPress
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.test.espresso.Espresso.pressBack
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertFalse
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.qr.DeviceApprovalQr
import to.iris.chat.ui.screens.QrScannerTestOverrides

@RunWith(AndroidJUnit4::class)
class PikaLikeUiTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Before
    fun resetAppState() {
        QrScannerTestOverrides.nextScannedValue = null
        (composeRule.activity.application as IrisChatApp)
            .container
            .appManager
            .resetForUiTestsBlocking()
        composeRule.waitUntil(20_000) {
            runCatching {
                composeRule
                    .onAllNodesWithTag("welcomeCreateAction", useUnmergedTree = true)
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }.getOrDefault(false)
        }
    }

    @Test
    fun generate_account_and_open_profile_sheet() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("myProfileSheet")
        composeRule.onNodeWithTag("myProfileQrCode", useUnmergedTree = true).assertIsDisplayed()
        composeRule.onNodeWithTag("myProfileManageDevicesButton", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun profile_sheet_opens_manage_devices() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("myProfileManageDevicesButton")
        composeRule.onNodeWithTag("myProfileManageDevicesButton", useUnmergedTree = true)
            .performClick()

        composeRule.waitForTag("deviceRosterOwnerNpub")
        composeRule.onNodeWithTag("deviceRosterCurrentDeviceNpub", useUnmergedTree = true)
            .assertIsDisplayed()
        composeRule.onNodeWithTag("deviceRosterAddInput", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun manage_devices_valid_input_enables_authorize_action() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("myProfileManageDevicesButton")
        composeRule.onNodeWithTag("myProfileManageDevicesButton", useUnmergedTree = true)
            .performClick()

        composeRule.waitForTag("deviceRosterAddInput")
        composeRule.onNodeWithTag("deviceRosterAddInput", useUnmergedTree = true)
            .performTextInput(SECONDARY_DEVICE_NPUB)
        composeRule.onNodeWithTag("deviceRosterAddButton", useUnmergedTree = true)
            .assertIsEnabled()
    }

    @Test
    fun scan_device_approval_qr_authorizes_device() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("myProfileManageDevicesButton")
        composeRule.onNodeWithTag("myProfileManageDevicesButton", useUnmergedTree = true)
            .performClick()

        composeRule.waitForTag("deviceRosterOwnerNpub")
        val ownerNpub =
            (composeRule.activity.application as IrisChatApp)
                .container
                .appManager
                .state
                .value
                .deviceRoster
                ?.ownerNpub
                .orEmpty()

        composeRule.runOnUiThread {
            QrScannerTestOverrides.nextScannedValue =
                DeviceApprovalQr.encode(
                    ownerInput = ownerNpub,
                    deviceInput = SECONDARY_DEVICE_NPUB,
                )
        }
        composeRule.onNodeWithTag("deviceRosterScanButton", useUnmergedTree = true).performClick()
        composeRule.waitUntil(10_000) {
            runCatching {
                composeRule
                    .onNodeWithTag("deviceRosterAddButton", useUnmergedTree = true)
                    .assertIsEnabled()
                true
            }.getOrDefault(false)
        }
        composeRule.onNodeWithTag("deviceRosterAddButton", useUnmergedTree = true).performClick()
        composeRule.waitUntil(20_000) {
            val roster =
                (composeRule.activity.application as IrisChatApp)
                    .container
                    .appManager
                    .state
                    .value
                    .deviceRoster
            roster?.devices?.any { it.deviceNpub == SECONDARY_DEVICE_NPUB && it.isAuthorized } == true
        }
    }

    @Test
    fun create_chat_and_send_message_locally() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatPeerInput")
        composeRule.onNodeWithTag("newChatScanQrButton", useUnmergedTree = true).assertIsDisplayed()
        composeRule.onNodeWithTag("newChatPeerInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)

        composeRule.waitForTag("chatMessageInput")
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
            .performTextInput("hello from test")
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).performClick()

        composeRule.waitForText("hello from test")
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).assertIsNotEnabled()
    }

    @Test
    fun tapping_message_on_mobile_reveals_actions() {
        assumeTrue(composeRule.activity.resources.configuration.screenWidthDp < 600)

        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatPeerInput")
        composeRule.onNodeWithTag("newChatPeerInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)

        composeRule.waitForTag("chatMessageInput")
        val message = "tap actions ${System.nanoTime()}"
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
            .performTextInput(message)
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).performClick()
        composeRule.waitForText(message)

        val messageId =
            (composeRule.activity.application as IrisChatApp)
                .container
                .appManager
                .state
                .value
                .currentChat
                ?.messages
                ?.firstOrNull { it.body == message }
                ?.id
                .orEmpty()
        composeRule.onNodeWithTag("chatMessage-$messageId", useUnmergedTree = true).performClick()

        composeRule.waitForTag("messageReactButton")
        composeRule.onNodeWithTag("messageReactButton", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun submitted_messages_stay_scrolled_to_latest() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatPeerInput")
        composeRule.onNodeWithTag("newChatPeerInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)
        composeRule.waitForTag("chatMessageInput")

        val messagePrefix = "scroll pin ${System.nanoTime()}"
        repeat(18) { index ->
            val message = "$messagePrefix $index"
            composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
                .performTextInput(message)
            composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).performClick()
            composeRule.waitForText(message)
            composeRule.onNodeWithText(message, useUnmergedTree = true).assertIsDisplayed()
        }
    }

    @Test
    fun enter_key_keeps_mobile_draft_unsent() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatPeerInput")
        composeRule.onNodeWithTag("newChatPeerInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)

        composeRule.waitForTag("chatMessageInput")
        val message = "hello from enter ${System.nanoTime()}"
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
            .performTextInput(message)
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
            .performKeyPress(
                KeyEvent(
                    AndroidKeyEvent(
                        AndroidKeyEvent.ACTION_DOWN,
                        AndroidKeyEvent.KEYCODE_ENTER,
                    ),
                ),
            )

        composeRule.waitForIdle()
        assertFalse(currentChatMessageBodies().contains(message))
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).performClick()
        composeRule.waitForText(message)
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).assertIsNotEnabled()
    }

    @Test
    fun tapping_timeline_clears_message_input_focus() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatPeerInput")
        composeRule.onNodeWithTag("newChatPeerInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)

        composeRule.waitForTag("chatMessageInput")
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
            .performTextInput("dismiss keyboard")
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true).assertIsFocused()

        composeRule.onNodeWithTag("chatTimeline", useUnmergedTree = true).performTouchInput {
            click(center)
        }

        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true).assertIsNotFocused()
    }

    @Test
    fun scan_qr_starts_new_chat() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatPeerInput")
        composeRule.runOnUiThread {
            QrScannerTestOverrides.nextScannedValue = VALID_PEER_NPUB
        }
        composeRule.onNodeWithTag("newChatScanQrButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("chatMessageInput")
    }

    @Test
    fun link_device_shows_scannable_code() {
        composeRule.resetToWelcome()
        composeRule.onNodeWithTag("welcomeAddDeviceAction", useUnmergedTree = true).performClick()
        composeRule.waitForTag("addDeviceScreen")
        composeRule.waitForTag("linkDeviceQrCode")
        composeRule.onNodeWithTag("linkDeviceQrCode", useUnmergedTree = true)
            .assertIsDisplayed()
        composeRule.onNodeWithTag("linkDeviceCopyButton", useUnmergedTree = true)
            .assertIsDisplayed()
    }

    @Test
    fun restore_account_opens_chat_list() {
        composeRule.resetToWelcome()
        composeRule.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true).performClick()

        composeRule.waitForTag("restoreAccountScreen")
        composeRule.onNodeWithTag("importKeyField", useUnmergedTree = true)
            .performTextInput(VALID_OWNER_NSEC)
        composeRule.onNodeWithTag("importKeyButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("chatListNewChatButton")
    }

    @Test
    fun restore_invalid_secret_key_shows_invalid_key() {
        composeRule.resetToWelcome()
        composeRule.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true).performClick()

        composeRule.waitForTag("restoreAccountScreen")
        composeRule.onNodeWithTag("importKeyField", useUnmergedTree = true)
            .performTextInput("not a secret key")
        composeRule.onNodeWithTag("importKeyButton", useUnmergedTree = true).performClick()

        composeRule.waitForText("Invalid key.")
    }

    @Test
    fun new_chat_view_opens_group_flow() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newChatNewGroupButton")
        composeRule.onNodeWithTag("newChatNewGroupButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newGroupMemberStep")
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).assertIsNotEnabled()
    }

    @Test
    fun create_group_and_open_group_details() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("newChatNewGroupButton")
        composeRule.onNodeWithTag("newChatNewGroupButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newGroupMemberStep")
        composeRule.onNodeWithTag("newGroupMemberInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)
        composeRule.onNodeWithTag("newGroupAddMemberButton", useUnmergedTree = true).performClick()
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("newGroupDetailsStep")
        composeRule.onNodeWithTag("newGroupNameInput", useUnmergedTree = true)
            .performTextInput("Trip crew")
        composeRule.onNodeWithTag("newGroupCreateButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("newGroupCreateButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("chatMessageInput")
        composeRule.onNodeWithTag("chatHeaderTitleButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("groupDetailsScreen")
        composeRule.onNodeWithTag("groupDetailsNameInput", useUnmergedTree = true)
            .performScrollTo()
            .assertIsDisplayed()
        composeRule.onNodeWithTag("groupDetailsAddMembersButton", useUnmergedTree = true)
            .performScrollTo()
            .assertIsDisplayed()
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForTag(
        tag: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForText(
        text: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
    }

    companion object {
        private const val VALID_PEER_NPUB =
            "npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj"
        private const val VALID_OWNER_NSEC =
            "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"
        private const val SECONDARY_DEVICE_NPUB =
            "npub1p34efzmkewwdsksmpp2r0tk7quke9jcfdz2zl7ezk8wnsj43uz2s8x5sp4"
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.ensureChatList() {
        waitUntil(30_000) {
            hasTag("welcomeCreateAction") ||
                hasTag("chatListNewChatButton") ||
                hasTag("newChatPeerInput") ||
                hasTag("newGroupMemberStep") ||
                hasTag("newGroupDetailsStep") ||
                hasTag("newGroupNameInput") ||
                hasTag("chatMessageInput") ||
                hasTag("myProfileSheet") ||
                hasTag("groupDetailsScreen")
        }

        if (hasTag("welcomeCreateAction")) {
            onNodeWithTag("welcomeCreateAction", useUnmergedTree = true).performClick()
            waitForTag("createAccountScreen")
            onNodeWithTag("signupNameField", useUnmergedTree = true)
                .assertIsFocused()
                .performTextInput("android tester")
            onNodeWithTag("generateKeyButton", useUnmergedTree = true).performClick()
            waitForTag("chatListNewChatButton")
            return
        }

        repeat(3) {
            if (hasTag("chatListNewChatButton")) {
                return
            }
            if (
                hasTag("newChatPeerInput") ||
                hasTag("newGroupMemberStep") ||
                hasTag("newGroupDetailsStep") ||
                hasTag("newGroupNameInput") ||
                hasTag("chatMessageInput") ||
                hasTag("myProfileSheet") ||
                hasTag("groupDetailsScreen")
            ) {
                pressBack()
                waitForIdle()
            }
        }

        waitForTag("chatListNewChatButton")
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.resetToWelcome() {
        runOnUiThread {
            val activity = activity
            (activity.application as IrisChatApp).container.appManager.logout()
        }
        waitForTag("welcomeCreateAction", timeoutMillis = 30_000)
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.hasTag(tag: String): Boolean =
        onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun currentChatMessageBodies(): List<String> =
        (composeRule.activity.application as IrisChatApp)
            .container
            .appManager
            .state
            .value
            .currentChat
            ?.messages
            ?.map { it.body }
            .orEmpty()
}
