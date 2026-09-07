package to.iris.chat.ui.screens

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import to.iris.chat.ui.components.ImageLoadRequest
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.loadHttpImage
import to.iris.chat.ui.components.rememberIrisHapticFeedback

@Composable
internal fun ProfilePictureDialog(
    imageRequest: ImageLoadRequest?,
    imageData: ByteArray?,
    onDismiss: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val dismissInteractionSource = remember { MutableInteractionSource() }
    val dataBitmap =
        remember(imageData) {
            imageData?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
        }
    val urlBitmap by produceState<Bitmap?>(initialValue = null, imageRequest) {
        value = withContext(Dispatchers.IO) {
            imageRequest?.let { request ->
                loadHttpImage(request) { data -> BitmapFactory.decodeByteArray(data, 0, data.size) }
            }
        }
    }
    val resolvedBitmap = dataBitmap ?: urlBitmap
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.92f))
                    .clickable(
                        interactionSource = dismissInteractionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onDismiss()
                    }
                    .testTag("myProfilePictureViewer"),
            contentAlignment = Alignment.Center,
        ) {
            resolvedBitmap?.let { loadedBitmap ->
                Image(
                    bitmap = loadedBitmap.asImageBitmap(),
                    contentDescription = "Profile picture",
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(18.dp),
                    contentScale = ContentScale.Fit,
                )
            } ?: CircularProgressIndicator(color = Color.White)
            IconButton(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.TopEnd),
            ) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Close profile picture",
                    tint = Color.White,
                )
            }
        }
    }
}
