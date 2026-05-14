package to.iris.chat.ui.screens

import android.graphics.Bitmap
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import to.iris.chat.ui.theme.IrisTheme

fun createQrBitmap(
    value: String,
    size: Int,
): Bitmap? =
    runCatching {
        val matrix = QRCodeWriter().encode(value, BarcodeFormat.QR_CODE, size, size)
        Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888).apply {
            for (x in 0 until size) {
                for (y in 0 until size) {
                    setPixel(
                        x,
                        y,
                        if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE,
                    )
                }
            }
        }
    }.getOrNull()

@Composable
fun IrisQrCodeImage(
    bitmap: Bitmap,
    contentDescription: String,
    modifier: Modifier = Modifier,
    size: Dp = 280.dp,
    tag: String? = null,
) {
    Surface(
        modifier = modifier,
        color = Color.White,
        contentColor = Color.Black,
        shape = RoundedCornerShape(18.dp),
        border = BorderStroke(1.dp, IrisTheme.palette.border.copy(alpha = 0.32f)),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = contentDescription,
            modifier =
                Modifier
                    .size(size)
                    .padding(16.dp)
                    .then(if (tag != null) Modifier.testTag(tag) else Modifier),
        )
    }
}
