package to.iris.chat.account

import android.content.Context
import java.io.File
import java.io.IOException
import java.util.UUID

/** Keeps legacy installs readable and new profiles separate from stranded account data. */
internal class AccountStorage(private val legacyDirectory: File, noBackupDirectory: File) {
    private val profiles = File(noBackupDirectory, "profiles")
    private val active = File(profiles, "active")

    // The parent is also the migration marker. A crash while preparing an empty
    // active directory must never send the next launch back to the legacy store.
    fun currentDirectory(): File = if (profiles.exists()) active else legacyDirectory

    /**
     * A restored database may outlive its Keystore key. Explicit signup gets a
     * fresh store without adopting or erasing the stranded account. Shut down
     * the previous core before moving its database and WAL.
     */
    fun prepareNewProfile(): File {
        requireDirectory(profiles)
        if (active.exists()) {
            val recovered = File(profiles, "recovered-${UUID.randomUUID()}")
            if (!active.renameTo(recovered)) {
                throw IOException("Could not preserve previous profile")
            }
        }
        requireDirectory(active)
        return active
    }

    private fun requireDirectory(directory: File) {
        if (!directory.isDirectory && !directory.mkdirs()) {
            throw IOException("Could not create profile folder")
        }
    }
}

internal fun wipeAppStorage(appContext: Context) {
    wipeDirectoryContents(appContext.filesDir)
    wipeDirectoryContents(appContext.noBackupFilesDir)
    wipeDirectoryContents(appContext.cacheDir)
    appContext.externalCacheDirs.forEach(::wipeDirectoryContents)
    appContext.getExternalFilesDirs(null).forEach { dir ->
        if (dir != null) {
            wipeDirectoryContents(dir)
        }
    }
    appContext.filesDir.mkdirs()
    appContext.noBackupFilesDir.mkdirs()
}

private fun wipeDirectoryContents(directory: File?) {
    val dir = directory ?: return
    if (!dir.exists()) {
        return
    }
    dir.listFiles()?.forEach { child ->
        runCatching { child.deleteRecursively() }
    }
}
