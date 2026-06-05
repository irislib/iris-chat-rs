package to.iris.chat

import android.Manifest
import android.os.Build
import android.view.KeyEvent as AndroidKeyEvent
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsNotFocused
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.click
import androidx.compose.ui.test.longClick
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
import androidx.test.platform.app.InstrumentationRegistry
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
        composeRule.waitForDisplayedTag("settingsProfileQrButton")
        composeRule.waitForDisplayedTagAfterScroll("settingsDevicesRow")
        composeRule.openSettingsPage("settingsProfileRow")
        composeRule.waitForDisplayedTagAfterScroll("myProfileShowQrButton")
        composeRule.waitForDisplayedTagAfterScroll("myProfileDisplayNameInput")
        composeRule.waitForDisplayedTagAfterScroll("myProfileAboutInput")
    }

    @Test
    fun profile_sheet_opens_manage_devices() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()

        composeRule.openSettingsPage("settingsDevicesRow")

        composeRule.waitForTag("deviceRosterOwnerNpub")
        composeRule.onNodeWithTag("deviceRosterAddInput", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun profile_sheet_toggles_debug_logging_and_exposes_debug_dump() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("myProfileSheet")
        composeRule.openSettingsPage("settingsSupportRow")
        composeRule.onNodeWithTag("myProfileDebugLoggingSwitch", useUnmergedTree = true)
            .performScrollTo()
            .assertIsOff()
            .performClick()
        composeRule.waitUntil(10_000) {
            (composeRule.activity.application as IrisChatApp)
                .container
                .appManager
                .state
                .value
                .preferences
                .debugLoggingEnabled
        }
        composeRule.onNodeWithTag("myProfileDebugLoggingSwitch", useUnmergedTree = true)
            .assertIsOn()
        composeRule.onNodeWithTag("myProfileCopySupportBundleButton", useUnmergedTree = true)
            .performScrollTo()
            .assertIsDisplayed()
    }

    @Test
    fun nearby_row_opens_nearby_sheet() {
        grantNearbyPermissions()
        composeRule.ensureChatList()
        composeRule.hideKeyboard()

        composeRule.onNodeWithTag("nearbyChatRow", useUnmergedTree = true).performClick()

        composeRule.waitForDisplayedTag("nearbyIrisSheet")
        composeRule.waitForDisplayedTag("nearbyCloseButton")
        composeRule.waitForDisplayedTag("nearbyEnabledSwitch")
        composeRule.waitForDisplayedTag("nearbyVisibilitySwitch")
        composeRule.waitForDisplayedTag("nearbyLanSwitch")
        composeRule.onNodeWithTag("nearbyEnabledSwitch", useUnmergedTree = true)
            .assertIsOn()
        composeRule.onNodeWithTag("nearbyVisibilitySwitch", useUnmergedTree = true)
            .assertIsOff()
            .assertIsEnabled()
        composeRule.onNodeWithTag("nearbyLanSwitch", useUnmergedTree = true)
            .assertIsOff()
            .assertIsEnabled()

        composeRule.onNodeWithTag("nearbyEnabledSwitch", useUnmergedTree = true).performClick()
        composeRule.waitUntil(10_000) {
            runCatching {
                composeRule.onNodeWithTag("nearbyEnabledSwitch", useUnmergedTree = true)
                    .assertIsOff()
                composeRule.onNodeWithTag("nearbyVisibilitySwitch", useUnmergedTree = true)
                    .assertIsNotEnabled()
                composeRule.onNodeWithTag("nearbyLanSwitch", useUnmergedTree = true)
                    .assertIsNotEnabled()
                true
            }.getOrDefault(false)
        }
        composeRule.onNodeWithTag("nearbyVisibilitySwitch", useUnmergedTree = true)
            .assertIsNotEnabled()
        composeRule.onNodeWithTag("nearbyLanSwitch", useUnmergedTree = true)
            .assertIsNotEnabled()

        composeRule.onNodeWithTag("nearbyCloseButton", useUnmergedTree = true).performClick()
        composeRule.waitUntil(10_000) {
            !composeRule.hasTag("nearbyIrisSheet")
        }
    }

    @Test
    fun manage_devices_valid_link_code_enables_authorize_action() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()
        composeRule.openSettingsPage("settingsDevicesRow")

        composeRule.waitForTag("deviceRosterAddInput")
        composeRule.onNodeWithTag("deviceRosterAddInput", useUnmergedTree = true)
            .performTextInput(LINK_DEVICE_INVITE_URL)
        composeRule.onNodeWithTag("deviceRosterAddButton", useUnmergedTree = true)
            .assertIsEnabled()
    }

    @Test
    fun manage_devices_plain_device_key_keeps_authorize_action_disabled() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()
        composeRule.openSettingsPage("settingsDevicesRow")

        composeRule.waitForTag("deviceRosterAddInput")
        composeRule.onNodeWithTag("deviceRosterAddInput", useUnmergedTree = true)
            .performTextInput(SECONDARY_DEVICE_NPUB)
        composeRule.onNodeWithTag("deviceRosterAddButton", useUnmergedTree = true)
            .assertIsNotEnabled()
    }

    @Test
    fun scan_device_approval_qr_authorizes_device() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListProfileButton", useUnmergedTree = true).performClick()
        composeRule.openSettingsPage("settingsDevicesRow")

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
        composeRule.waitForTag("newChatInviteShareButton")
        composeRule.onNodeWithTag("newChatInviteShareButton", useUnmergedTree = true)
            .assertIsDisplayed()
        composeRule.onNodeWithTag("newChatScanQrButton", useUnmergedTree = true).assertIsDisplayed()
        composeRule.onNodeWithTag("newChatPeerInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)

        composeRule.waitForTag("chatMessageInput")
        composeRule.onNodeWithTag("chatAttachButton", useUnmergedTree = true).assertIsDisplayed()
        composeRule.onAllNodesWithTag("chatSendButton", useUnmergedTree = true).assertCountEquals(0)
        composeRule.onNodeWithTag("chatMessageInput", useUnmergedTree = true)
            .performTextInput("hello from test")
        composeRule.onNodeWithTag("chatInlineAttachButton", useUnmergedTree = true).assertIsDisplayed()
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("chatSendButton", useUnmergedTree = true).performClick()

        composeRule.waitForText("hello from test")
        composeRule.waitUntil(10_000) { !composeRule.hasTag("chatSendButton") }
        composeRule.onNodeWithTag("chatAttachButton", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun long_pressing_message_on_mobile_reveals_actions() {
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
        composeRule.onNodeWithTag("chatMessage-$messageId", useUnmergedTree = true)
            .performTouchInput { longClick() }

        composeRule.waitForTag("messageActionsSheet")
        composeRule.onNodeWithTag("messageActionsSheet", useUnmergedTree = true).assertIsDisplayed()
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
            composeRule.waitForCurrentChatMessage(message)
            composeRule.waitForDisplayedText(message)
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
        composeRule.waitUntil(10_000) { !composeRule.hasTag("chatSendButton") }
        composeRule.onNodeWithTag("chatAttachButton", useUnmergedTree = true).assertIsDisplayed()
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
        composeRule.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true)
            .performClick()
        composeRule.waitForTag("restoreAccountScreen")
        composeRule.onNodeWithTag("restoreLinkDeviceAction", useUnmergedTree = true)
            .performClick()
        composeRule.waitForTag("addDeviceScreen")
        composeRule.waitForTag("linkDeviceQrCode")
        composeRule.onNodeWithTag("linkDeviceQrCode", useUnmergedTree = true)
            .assertIsDisplayed()
        composeRule.waitForDisplayedTagAfterScroll("linkDeviceCopyButton")
    }

    @Test
    fun restore_account_opens_chat_list() {
        composeRule.resetToWelcome()
        composeRule.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true)
            .performClick()

        composeRule.waitForTag("restoreAccountScreen")
        composeRule.onNodeWithTag("importKeyField", useUnmergedTree = true)
            .performTextInput(VALID_OWNER_NSEC)
        if (!composeRule.hasTag("chatListNewChatButton", timeoutMillis = 5_000)) {
            composeRule.onNodeWithTag("importKeyButton", useUnmergedTree = true).performClick()
        }

        composeRule.waitForTag("chatListNewChatButton")
    }

    @Test
    fun restore_invalid_secret_key_shows_invalid_key() {
        composeRule.resetToWelcome()
        composeRule.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true)
            .performClick()

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
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).assertIsEnabled()
    }

    @Test
    fun create_group_and_open_group_details() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("newChatNewGroupButton")
        composeRule.onNodeWithTag("newChatNewGroupButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newGroupMemberStep")
        composeRule.onAllNodesWithTag("newGroupPasteButton", useUnmergedTree = true).assertCountEquals(0)
        composeRule.onAllNodesWithTag("newGroupScanQrButton", useUnmergedTree = true).assertCountEquals(0)
        composeRule.onAllNodesWithTag("newGroupAddMemberButton", useUnmergedTree = true).assertCountEquals(0)
        composeRule.onNodeWithTag("newGroupMemberInput", useUnmergedTree = true)
            .performTextInput(VALID_PEER_NPUB)
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.waitForTag("memberChipRemove")
        composeRule.onAllNodesWithTag("memberChipRemove", useUnmergedTree = true).assertCountEquals(1)
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("newGroupDetailsStep")
        composeRule.onNodeWithTag("newGroupNameInput", useUnmergedTree = true)
            .performTextInput("Trip crew")
        composeRule.onNodeWithTag("newGroupCreateButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("newGroupCreateButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("chatMessageInput")
        composeRule.onNodeWithTag("chatHeaderTitleButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("groupDetailsScreen")
        composeRule.waitForDisplayedTagAfterScroll("groupDetailsNameInput")
        composeRule.waitForDisplayedTagAfterScroll("groupDetailsAddMembersButton")
    }

    @Test
    fun create_self_only_group() {
        composeRule.ensureChatList()
        composeRule.onNodeWithTag("chatListNewChatButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("newChatNewGroupButton")
        composeRule.onNodeWithTag("newChatNewGroupButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("newGroupMemberStep")
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("newGroupNextButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("newGroupDetailsStep")
        composeRule.onNodeWithTag("newGroupNameInput", useUnmergedTree = true)
            .performTextInput("Solo notes")
        composeRule.onNodeWithTag("newGroupCreateButton", useUnmergedTree = true).assertIsEnabled()
        composeRule.onNodeWithTag("newGroupCreateButton", useUnmergedTree = true).performClick()

        composeRule.waitForTag("chatMessageInput")
        composeRule.onNodeWithTag("chatHeaderTitleButton", useUnmergedTree = true).performClick()
        composeRule.waitForTag("groupDetailsScreen")
        composeRule.waitForDisplayedTagAfterScroll("groupDetailsNameInput")
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForTag(
        tag: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            runCatching {
                onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
            }.getOrDefault(false)
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForDisplayedTag(
        tag: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            runCatching {
                onNodeWithTag(tag, useUnmergedTree = true).assertIsDisplayed()
                true
            }.getOrDefault(false)
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForDisplayedTagAfterScroll(
        tag: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            runCatching {
                onNodeWithTag(tag, useUnmergedTree = true)
                    .performScrollTo()
                    .assertIsDisplayed()
                true
            }.getOrDefault(false)
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.openSettingsPage(tag: String) {
        waitForTag(tag)
        waitUntil(15_000) {
            runCatching {
                onNodeWithTag(tag, useUnmergedTree = true).performScrollTo().performClick()
                true
            }.getOrDefault(false)
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForText(
        text: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            runCatching {
                onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
            }.getOrDefault(false)
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForDisplayedText(
        text: String,
        timeoutMillis: Long = 20_000,
    ) {
        waitUntil(timeoutMillis) {
            runCatching {
                onNodeWithText(text, useUnmergedTree = true).assertIsDisplayed()
                true
            }.getOrDefault(false)
        }
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.waitForCurrentChatMessage(
        message: String,
        timeoutMillis: Long = 15_000,
    ) {
        waitUntil(timeoutMillis) {
            currentChatMessageBodies().contains(message)
        }
    }

    companion object {
        private const val VALID_PEER_NPUB =
            "npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj"
        private const val VALID_OWNER_NSEC =
            "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"
        private const val SECONDARY_DEVICE_NPUB =
            "npub1p34efzmkewwdsksmpp2r0tk7quke9jcfdz2zl7ezk8wnsj43uz2s8x5sp4"
        private const val LINK_DEVICE_INVITE_URL =
            "https://chat.iris.to/#%7B%22purpose%22%3A%22link%22%2C%22ephemeralKey%22%3A%22x%22%2C%22sharedSecret%22%3A%22y%22%7D"
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
            waitForTag("signupNameField")
            onNodeWithTag("signupNameField", useUnmergedTree = true)
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
        (activity.application as IrisChatApp)
            .container
            .appManager
            .resetForUiTestsBlocking()
        waitForDisplayedTag("welcomeCreateAction", timeoutMillis = 30_000)
    }

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.hasTag(tag: String): Boolean =
        runCatching {
            onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }.getOrDefault(false)

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.hasTag(
        tag: String,
        timeoutMillis: Long,
    ): Boolean =
        runCatching {
            waitUntil(timeoutMillis) {
                hasTag(tag)
            }
            true
        }.getOrDefault(false)

    private fun androidx.compose.ui.test.junit4.AndroidComposeTestRule<*, *>.hideKeyboard() {
        runOnUiThread {
            val activity = activity
            activity
                .getSystemService(InputMethodManager::class.java)
                .hideSoftInputFromWindow(activity.window.decorView.windowToken, 0)
            activity.window.decorView.clearFocus()
        }
        waitForIdle()
    }

    private fun grantNearbyPermissions() {
        val packageName = composeRule.activity.packageName
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
        val localNetworkPermissions =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                listOf(Manifest.permission.NEARBY_WIFI_DEVICES)
            } else {
                emptyList()
            }
        val uiAutomation = InstrumentationRegistry.getInstrumentation().uiAutomation
        (permissions + localNetworkPermissions).forEach { permission ->
            runCatching {
                uiAutomation.grantRuntimePermission(packageName, permission)
            }
        }
    }

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
