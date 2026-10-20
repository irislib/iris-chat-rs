package to.iris.chat

import android.os.SystemClock
import java.io.File
import java.nio.file.Files

/** Test-only host barrier; never reads or changes account data or preferences. */
internal class BleBackgroundRelease(filesDir: File, val token: String) {
    private val releaseFile: File

    init {
        require(Regex("[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}").matches(token)) {
            "Background release requires a canonical random UUID"
        }
        val directory = File(filesDir.canonicalFile, "ble-background-release")
        check(directory.mkdirs() || directory.isDirectory) { "Cannot prepare background barrier directory" }
        check(directory.canonicalFile == directory) { "Background barrier directory cannot be a symlink" }
        releaseFile = File(directory, "$token.ready")
        check(!releaseFile.exists() && !Files.isSymbolicLink(releaseFile.toPath())) {
            "Background release file already exists; preserve the stale file"
        }
    }

    fun awaitRelease() {
        val deadline = SystemClock.elapsedRealtime() + 180_000
        try {
            while (!releaseFile.exists()) {
                check(!Files.isSymbolicLink(releaseFile.toPath())) { "Invalid background release file" }
                check(SystemClock.elapsedRealtime() < deadline) { "Host did not release the background barrier" }
                SystemClock.sleep(100)
            }
            check(!Files.isSymbolicLink(releaseFile.toPath()) && releaseFile.isFile && releaseFile.length() == 0L) {
                "Invalid background release file"
            }
            check(releaseFile.delete()) { "Cannot consume background release file" }
        } finally {
            // Only this fresh run's exact file; never sweep previous probes.
            if (releaseFile.exists() || Files.isSymbolicLink(releaseFile.toPath())) {
                check(releaseFile.delete()) { "Cannot clean up background release file" }
            }
        }
    }
}
