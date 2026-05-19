package to.iris.chat.update

import java.net.URL
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidSelfUpdateManagerTest {
    @Test
    fun versionIsNewerHandlesDateBasedTags() {
        // Iris ships date-based release tags. Only versions whose first part
        // crosses the dev/release threshold are considered comparable.
        assertTrue(versionIsNewer("v2026.5.18.6", "2026.5.18.5"))
        assertTrue(versionIsNewer("v2026.6.1", "v2026.5.18.5"))
        assertFalse(versionIsNewer("v2026.5.18.5", "2026.5.18.5"))
        assertFalse(versionIsNewer("v2026.5.18.4", "2026.5.18.5"))
    }

    @Test
    fun versionIsNewerTreatsPreReleaseVersionsAsDevPlaceholders() {
        // 0.x.y is the default versionName until a release config is applied.
        // The updater must not claim an update is available in that state.
        assertFalse(versionIsNewer("v2026.5.18.5", "0.1.0"))
        assertFalse(versionIsNewer("v2026.5.18.5", ""))
    }

    @Test
    fun resolveAssetUrlJoinsRelativePaths() {
        val manifest =
            "https://upload.iris.to/npub.../releases%2Firis-chat-rs/latest/release.json"
        val joined = resolveAssetUrl(manifest, "assets/iris-chat-v2026.5.18.5-android-arm64.apk")
        assertTrue(
            "expected joined URL to keep manifest base, got: $joined",
            joined.endsWith("/latest/assets/iris-chat-v2026.5.18.5-android-arm64.apk"),
        )
    }

    @Test
    fun liveReleaseManifestExposesAndroidApk() {
        // Hits the production release manifest. Fails loudly if upload.iris.to
        // ever changes the JSON shape or the APK suffix our updater filters on.
        val body = URL(MANIFEST_URL).readText()
        val apkPattern = Regex(""""name"\s*:\s*"[^"]*-android-arm64\.apk"""")
        assertTrue(
            "release.json must contain a *-android-arm64.apk asset; got: $body",
            apkPattern.containsMatchIn(body),
        )
        val tagPattern = Regex(""""tag"\s*:\s*"v\d+""")
        assertTrue(
            "release.json must contain a versioned tag; got: $body",
            tagPattern.containsMatchIn(body),
        )
    }

    private companion object {
        const val MANIFEST_URL =
            "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest/release.json"
    }
}
