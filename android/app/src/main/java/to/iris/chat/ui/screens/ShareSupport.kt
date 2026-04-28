package to.iris.chat.ui.screens

import android.content.Context
import android.content.Intent
import android.widget.Toast

internal fun canShareText(
    context: Context,
    mimeType: String = "text/plain",
): Boolean {
    val intent = Intent(Intent.ACTION_SEND).setType(mimeType)
    val chooser = Intent.createChooser(intent, "Share")
    return chooser.resolveActivity(context.packageManager) != null
}

internal fun shareText(
    context: Context,
    text: String,
    title: String,
    mimeType: String = "text/plain",
    subject: String? = null,
) {
    val intent =
        Intent(Intent.ACTION_SEND).apply {
            type = mimeType
            putExtra(Intent.EXTRA_TEXT, text)
            subject?.let { putExtra(Intent.EXTRA_SUBJECT, it) }
        }
    val chooser = Intent.createChooser(intent, title)
    if (chooser.resolveActivity(context.packageManager) == null) {
        Toast.makeText(context, "Sharing unavailable", Toast.LENGTH_SHORT).show()
        return
    }
    try {
        context.startActivity(chooser)
    } catch (_: RuntimeException) {
        Toast.makeText(context, "Sharing unavailable", Toast.LENGTH_SHORT).show()
    }
}
