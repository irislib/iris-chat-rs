package to.iris.chat.ui.screens

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException

internal fun decodeChatAttachmentImage(
    path: String,
    maxPixelSize: Int?,
): Bitmap? {
    if (!File(path).exists()) {
        return null
    }
    val exif =
        try {
            ExifInterface(path)
        } catch (_: IOException) {
            null
        }
    val bitmap =
        if (maxPixelSize == null) {
            BitmapFactory.decodeFile(path)
        } else {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(path, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
                return null
            }
            val options =
                BitmapFactory.Options().apply {
                    inSampleSize =
                        chatAttachmentSampleSize(
                            bounds.outWidth,
                            bounds.outHeight,
                            maxPixelSize,
                        )
                }
            BitmapFactory.decodeFile(path, options)
        } ?: return null

    return applyExifOrientation(
        source = bitmap,
        isFlipped = exif?.isFlipped == true,
        rotationDegrees = exif?.rotationDegrees ?: 0,
    )
}

internal fun decodeChatAttachmentImage(
    data: ByteArray,
    maxPixelSize: Int?,
): Bitmap? {
    val exif =
        try {
            ByteArrayInputStream(data).use(::ExifInterface)
        } catch (_: IOException) {
            null
        }
    val bitmap =
        if (maxPixelSize == null) {
            BitmapFactory.decodeByteArray(data, 0, data.size)
        } else {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
                return null
            }
            val options =
                BitmapFactory.Options().apply {
                    inSampleSize =
                        chatAttachmentSampleSize(
                            bounds.outWidth,
                            bounds.outHeight,
                            maxPixelSize,
                        )
                }
            BitmapFactory.decodeByteArray(data, 0, data.size, options)
        } ?: return null

    return applyExifOrientation(
        source = bitmap,
        isFlipped = exif?.isFlipped == true,
        rotationDegrees = exif?.rotationDegrees ?: 0,
    )
}

private fun applyExifOrientation(
    source: Bitmap,
    isFlipped: Boolean,
    rotationDegrees: Int,
): Bitmap {
    if (!isFlipped && rotationDegrees == 0) {
        return source
    }
    val matrix = Matrix()
    if (isFlipped) {
        matrix.postScale(-1f, 1f)
    }
    if (rotationDegrees != 0) {
        matrix.postRotate(rotationDegrees.toFloat())
    }
    val oriented =
        Bitmap.createBitmap(
            source,
            0,
            0,
            source.width,
            source.height,
            matrix,
            true,
        )
    if (oriented !== source) {
        source.recycle()
    }
    return oriented
}

private fun chatAttachmentSampleSize(
    width: Int,
    height: Int,
    maxPixelSize: Int,
): Int {
    var sampleSize = 1
    while (
        width / (sampleSize * 2) >= maxPixelSize ||
            height / (sampleSize * 2) >= maxPixelSize
    ) {
        sampleSize *= 2
    }
    return sampleSize
}
