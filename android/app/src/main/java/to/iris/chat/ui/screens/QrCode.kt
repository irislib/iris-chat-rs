package to.iris.chat.ui.screens

import android.graphics.Bitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter

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
