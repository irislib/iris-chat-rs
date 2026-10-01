package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun ChatMessageInput(
    state: TextFieldState,
    modifier: Modifier,
) {
    BasicTextField(
        state = state,
        modifier = modifier,
        textStyle = MaterialTheme.typography.bodyLarge.copy(
            fontSize = to.iris.chat.ui.theme.LocalMessageFontSize.current.body.sp,
            lineHeight = (to.iris.chat.ui.theme.LocalMessageFontSize.current.body * 1.4f).sp,
            color = MaterialTheme.colorScheme.onSurface,
        ),
        cursorBrush = SolidColor(IrisTheme.palette.accent),
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        lineLimits = TextFieldLineLimits.MultiLine(minHeightInLines = 1, maxHeightInLines = 5),
        decorator = { innerTextField ->
            Box(
                modifier = Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(vertical = 8.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (state.text.isEmpty()) {
                    Text("Message", style = MaterialTheme.typography.bodyLarge, color = IrisTheme.palette.muted)
                }
                innerTextField()
            }
        },
    )
}
