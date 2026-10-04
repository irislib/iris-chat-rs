package to.iris.chat.ui.screens

import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import kotlinx.coroutines.*
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import to.iris.chat.core.AppManager
import to.iris.chat.core.MockRustAppClient
import to.iris.chat.core.RecordingSecureSecretStore
import to.iris.chat.rust.*
import to.iris.chat.ui.theme.IrisChatTheme

class GroupMemberPickerTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun scrollPastEightAndAddAgainWithoutReopeningSuggestions() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(context.cacheDir, "member-picker-${UUID.randomUUID()}").apply { mkdirs() }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val initial = buildLargeTestAppState(21u, 0u, 0u)
        val contacts = initial.chatList.filter { it.kind == ChatKind.DIRECT }.mapIndexed { index, chat ->
            chat.copy(chatId = (index + 1).toString(16).padStart(2, '0').padEnd(64, '0'),
                displayName = "Person $index", profileName = "Person $index", nickname = null,
                pictureUrl = null, subtitle = null)
        }
        val details = initial.groupDetails!!.copy(groupId = "picker-test", canManage = true,
            pictureUrl = null, members = emptyList())
        val state = mutableStateOf(initial.copy(chatList = contacts, groupDetails = details))
        val rust = MockRustAppClient(state.value)
        val store = PreferenceDataStoreFactory.create(scope = scope) { File(root, "test.preferences_pb") }
        val manager = AppManager(context, scope, RecordingSecureSecretStore(), dataStore = store,
            rustFactory = { _, _ -> rust }, clearDeliveredNotifications = {})
        try {
            compose.setContent { IrisChatTheme(darkTheme = false) {
                GroupDetailsScreen(manager, state.value, details.groupId)
            } }
            for ((turn, index) in listOf(19, 17).withIndex()) {
                val owner = contacts[index].chatId
                compose.onNodeWithTag("groupDetailsMemberCandidates").performScrollTo()
                    .performScrollToIndex(index)
                compose.onNodeWithTag("groupDetailsKnownUser-${owner.take(12)}").performClick()
                compose.onNodeWithTag("groupDetailsAddMembersButton").performScrollTo().performClick()
                compose.waitUntil(5_000) { rust.dispatchedActions.filterIsInstance<AppAction.AddGroupMembers>().size == turn + 1 }
                compose.runOnIdle {
                    val added = rust.dispatchedActions.filterIsInstance<AppAction.AddGroupMembers>().last()
                    assertEquals(listOf(owner), added.memberInputs)
                    val current = state.value
                    state.value = current.copy(rev = current.rev + 1uL,
                        groupDetails = current.groupDetails!!.copy(members = current.groupDetails!!.members +
                            GroupMemberSnapshot(null, owner, "Person $index", "", null, false, false, false)))
                }
                compose.waitForIdle()
                compose.onNodeWithTag("groupDetailsMemberCandidates").assertExists()
                compose.onNodeWithTag("groupDetailsKnownUser-${owner.take(12)}").assertDoesNotExist()
            }
            compose.onNodeWithTag("groupDetailsAddMemberInput").performScrollTo().performTextInput("Person 20")
            compose.onNodeWithTag("groupDetailsKnownUser-${contacts[20].chatId.take(12)}").assertExists()
        } finally {
            scope.cancel()
            root.deleteRecursively()
        }
    }
}
