package to.iris.chat.ui.screens

import android.graphics.Bitmap
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.SocialBadge
import to.iris.chat.rust.SocialConnectionSnapshot
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisChatListRow
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.LocalNearbyAvatarOwners
import to.iris.chat.ui.components.nearbyAvatarOwners
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class NearbyAvatarComposeTest {
    @get:Rule val composeRule = createComposeRule()

    @Test
    fun enabledLiveOwnersExcludeSelfAndClearWhenDisabledOrRemoved() {
        val snapshot = snapshot()
        assertEquals(setOf("alice"), nearbyAvatarOwners(snapshot, true, "self"))
        assertEquals(emptySet<String>(), nearbyAvatarOwners(snapshot, true, null))
        assertEquals(emptySet<String>(), nearbyAvatarOwners(snapshot, true, " "))
        assertEquals(emptySet<String>(), nearbyAvatarOwners(snapshot, false, "self"))
        assertEquals(emptySet<String>(), nearbyAvatarOwners(snapshot.copy(localNetworkVisible = false), true, "self"))
        assertEquals(emptySet<String>(), nearbyAvatarOwners(snapshot.copy(peers = emptyList()), true, "self"))
    }

    @Test
    fun headerAndListBadgesCoexistWithIdentityAndUpdateLive() {
        val peers = mutableStateOf(snapshot())
        val enabled = mutableStateOf(true)
        val social = SocialConnectionSnapshot(SocialBadge.FOLLOWING, 1u, 0u, "Followed by you")
        composeRule.setContent {
            IrisChatTheme(darkTheme = false) {
                CompositionLocalProvider(LocalNearbyAvatarOwners provides nearbyAvatarOwners(peers.value, enabled.value, "self")) {
                    Surface {
                        Column(Modifier.testTag("nearbyAvatarScreen")) {
                            IrisTopBar(title = "Alice", onBack = {}, titleAccessoryLeading = {
                                IrisAvatar(socialConnection = social, ownerPubkeyHex = "alice", label = "Alice", size = 36.dp)
                            })
                            IrisChatListRow(socialConnection = social, ownerPubkeyHex = "alice", title = "Alice", preview = "See you soon", timeLabel = "Now", unreadCount = 0, lastMessageMine = false, lastDelivery = null, onClick = {})
                            IrisChatListRow(ownerPubkeyHex = "self", title = "Note to self", preview = "Packing list", timeLabel = "Yesterday", unreadCount = 0, lastMessageMine = true, lastDelivery = null, onClick = {})
                            IrisChatListRow(title = "Weekend plans", preview = "A group conversation", timeLabel = "Yesterday", unreadCount = 0, lastMessageMine = false, lastDelivery = null, onClick = {})
                        }
                    }
                }
            }
        }
        composeRule.onAllNodesWithTag("nearbyAvatarBadge", useUnmergedTree = true).assertCountEquals(2)
        composeRule.onAllNodesWithContentDescription("Followed by you", useUnmergedTree = true).assertCountEquals(2)
        composeRule.onNodeWithTag("nearbyAvatarScreen").assertIsDisplayed()
        capture("android-nearby-header-list.png")
        composeRule.runOnIdle { enabled.value = false }
        composeRule.onAllNodesWithTag("nearbyAvatarBadge", useUnmergedTree = true).assertCountEquals(0)
        composeRule.runOnIdle { enabled.value = true }
        composeRule.onAllNodesWithTag("nearbyAvatarBadge", useUnmergedTree = true).assertCountEquals(2)
        composeRule.runOnIdle { peers.value = peers.value.copy(peers = emptyList()) }
        composeRule.onAllNodesWithTag("nearbyAvatarBadge", useUnmergedTree = true).assertCountEquals(0)
        capture("android-nearby-cleared.png")
    }

    private fun snapshot(): IrisNearbyService.Snapshot {
        val peers = listOf("alice", "self", null).mapIndexed { index, owner ->
            IrisNearbyService.Peer("device-$index", "Alice", owner, null, null, null, 0L)
        }
        return IrisNearbyService.Snapshot(false, "Off", false, true, true, true, "Visible", peers.size, peers, emptyList(), peers, null)
    }

    private fun capture(name: String) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val output = File(context.getExternalFilesDir("screenshots"), name)
        output.parentFile?.mkdirs()
        val image = composeRule.onNodeWithTag("nearbyAvatarScreen").captureToImage().asAndroidBitmap()
        output.outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}
