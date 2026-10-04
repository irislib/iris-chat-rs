package to.iris.chat.ui.screens

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class InviteOpenOnJoinComposeTest {
    @get:Rule val composeRule = createComposeRule()

    @Test fun inviteCheckboxReflectsLiveChoiceAndTogglesFromItsLabel() {
        var checked by mutableStateOf(true)
        composeRule.setContent {
            IrisChatTheme(darkTheme = false) { InviteOpenOnJoinCheckbox(checked) { checked = it } }
        }
        val choice = composeRule.onNodeWithTag("inviteOpenOnJoinToggle")
        choice.assertIsOn()
        composeRule.onNodeWithText("Open chat when someone joins").performClick()
        choice.assertIsOff()
        composeRule.runOnIdle { checked = true }
        choice.assertIsOn()
    }
}
