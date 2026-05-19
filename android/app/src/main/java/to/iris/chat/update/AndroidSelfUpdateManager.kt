package to.iris.chat.update

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import java.io.File
import java.net.URI
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import to.iris.chat.BuildConfig

data class AndroidSelfUpdateState(
    val supported: Boolean = false,
    val autoCheckEnabled: Boolean = true,
    val checking: Boolean = false,
    val downloading: Boolean = false,
    val available: Boolean = false,
    val version: String = "",
    val status: String = "",
    val downloaded: Boolean = false,
) {
    val busy: Boolean get() = checking || downloading
}

class AndroidSelfUpdateManager(
    private val context: Context,
    private val dataStore: DataStore<Preferences>,
    private val scope: CoroutineScope,
    private val ioDispatcher: CoroutineDispatcher,
    private val httpClient: OkHttpClient = OkHttpClient(),
) {
    private val appContext = context.applicationContext
    private val stateFlow =
        MutableStateFlow(
            AndroidSelfUpdateState(
                supported = supportsSelfUpdate(),
            ),
        )
    val state: StateFlow<AndroidSelfUpdateState> = stateFlow.asStateFlow()

    private val checkMutex = Mutex()
    private var autoCheckJob: Job? = null
    private var autoCheckPreferenceLoaded = false
    private var automaticChecksWanted = false
    private var startupCheckDone = false
    private var lastCheckStartedAtMs = 0L
    private var availableAssetUrl: String? = null
    private var downloadedApk: File? = null

    init {
        if (stateFlow.value.supported) {
            scope.launch(ioDispatcher) {
                val enabled =
                    dataStore.data
                        .catch { emit(androidx.datastore.preferences.core.emptyPreferences()) }
                        .first()[AUTO_CHECK_KEY] ?: true
                autoCheckPreferenceLoaded = true
                stateFlow.update { it.copy(autoCheckEnabled = enabled) }
                if (enabled && automaticChecksWanted) {
                    startAutomaticChecks()
                }
            }
        }
    }

    fun setAutoCheckEnabled(enabled: Boolean) {
        if (!stateFlow.value.supported) return
        stateFlow.update { it.copy(autoCheckEnabled = enabled) }
        scope.launch(ioDispatcher) {
            dataStore.edit { preferences ->
                preferences[AUTO_CHECK_KEY] = enabled
            }
        }
        if (enabled) {
            startAutomaticChecks()
        } else {
            stopAutomaticChecks()
        }
    }

    fun startAutomaticChecks() {
        automaticChecksWanted = true
        val snapshot = stateFlow.value
        if (!snapshot.supported ||
            !snapshot.autoCheckEnabled ||
            !autoCheckPreferenceLoaded ||
            autoCheckJob != null
        ) {
            return
        }
        autoCheckJob =
            scope.launch(ioDispatcher) {
                checkIfDue()
                while (isActive) {
                    delay(updatePollIntervalMs())
                    checkIfDue()
                }
            }
    }

    fun stopAutomaticChecks() {
        automaticChecksWanted = false
        autoCheckJob?.cancel()
        autoCheckJob = null
    }

    fun check(manual: Boolean = true) {
        if (!stateFlow.value.supported) return
        scope.launch(ioDispatcher) {
            checkForUpdate(manual = manual)
        }
    }

    fun download() {
        if (!stateFlow.value.supported) return
        scope.launch(ioDispatcher) {
            downloadAvailableApk()
        }
    }

    fun install(context: Context) {
        if (!stateFlow.value.supported) return
        val apk = downloadedApk?.takeIf { it.exists() }
        if (apk == null) {
            stateFlow.update { it.copy(status = "Download update first", downloaded = false) }
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
            !appContext.packageManager.canRequestPackageInstalls()
        ) {
            stateFlow.update { it.copy(status = "Allow app installs, then tap Install again") }
            val intent =
                Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${appContext.packageName}"),
                )
            context.startActivitySafely(intent)
            return
        }

        val uri =
            FileProvider.getUriForFile(
                appContext,
                "${appContext.packageName}.fileprovider",
                apk,
            )
        val intent =
            Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, APK_MIME_TYPE)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        context.startActivitySafely(intent)
        stateFlow.update { it.copy(status = "Installer opened") }
    }

    private suspend fun checkIfDue() {
        val snapshot = stateFlow.value
        if (snapshot.available || snapshot.downloaded || snapshot.busy) {
            return
        }
        val now = System.currentTimeMillis()
        val due =
            if (!startupCheckDone) {
                startupCheckDone = true
                true
            } else {
                now - lastCheckStartedAtMs >= updatePollIntervalMs()
            }
        if (due) {
            checkForUpdate(manual = false)
        }
    }

    private suspend fun checkForUpdate(manual: Boolean) {
        if (!stateFlow.value.supported) return
        checkMutex.withLock {
            if (stateFlow.value.busy) return
            lastCheckStartedAtMs = System.currentTimeMillis()
            if (manual) {
                stateFlow.update { it.copy(checking = true, status = "Checking for updates") }
            } else {
                stateFlow.update { it.copy(checking = true) }
            }
            try {
                val manifestUrl = updateManifestUrl()
                val manifest = loadReleaseManifest(manifestUrl)
                val asset = manifest.preferredAndroidAsset()
                val assetUrl = asset?.path?.let { resolveAssetUrl(manifestUrl, it) }
                val newerVersion = versionIsNewer(manifest.tag, BuildConfig.VERSION_NAME)
                val available = newerVersion && assetUrl != null
                availableAssetUrl = if (available) assetUrl else null
                downloadedApk = null
                stateFlow.update {
                    it.copy(
                        checking = false,
                        available = available,
                        version = manifest.tag,
                        downloaded = false,
                        status =
                            when {
                                available -> "Update ${manifest.tag} available"
                                newerVersion -> "Update ${manifest.tag} found without Android APK"
                                manual -> "Up to date"
                                else -> ""
                            },
                    )
                }
            } catch (error: Exception) {
                stateFlow.update {
                    it.copy(
                        checking = false,
                        status = if (manual) error.message ?: "Update check failed" else it.status,
                    )
                }
            }
        }
    }

    private suspend fun downloadAvailableApk() {
        val assetUrl = availableAssetUrl
        if (assetUrl.isNullOrBlank() || stateFlow.value.downloading) return
        checkMutex.withLock {
            stateFlow.update { it.copy(downloading = true, status = "Downloading ${it.version}") }
            try {
                val file = downloadApk(assetUrl, stateFlow.value.version)
                verifyDownloadedApk(file)
                downloadedApk = file
                stateFlow.update {
                    it.copy(
                        downloading = false,
                        downloaded = true,
                        status = "Ready to install",
                    )
                }
            } catch (error: Exception) {
                downloadedApk = null
                stateFlow.update {
                    it.copy(
                        downloading = false,
                        downloaded = false,
                        status = error.message ?: "Download failed",
                    )
                }
            }
        }
    }

    private suspend fun loadReleaseManifest(manifestUrl: String): ReleaseManifest =
        withContext(ioDispatcher) {
            val body = readString(manifestUrl)
            val json = JSONObject(body)
            val assetsJson = json.optJSONArray("assets")
            val assets =
                buildList {
                    if (assetsJson != null) {
                        for (index in 0 until assetsJson.length()) {
                            val asset = assetsJson.optJSONObject(index) ?: continue
                            add(
                                ReleaseAsset(
                                    name = asset.optString("name"),
                                    path = asset.optString("path"),
                                ),
                            )
                        }
                    }
                }
            ReleaseManifest(tag = json.optString("tag"), assets = assets)
        }

    private suspend fun downloadApk(assetUrl: String, version: String): File =
        withContext(ioDispatcher) {
            val downloadDir = File(appContext.cacheDir, "updates").apply { mkdirs() }
            downloadDir.listFiles()?.forEach { file ->
                if (file.extension.equals("apk", ignoreCase = true)) {
                    file.delete()
                }
            }
            val fileName =
                assetUrl
                    .substringAfterLast('/')
                    .substringBefore('?')
                    .takeIf { it.endsWith(".apk", ignoreCase = true) }
                    ?: "iris-chat-$version.apk"
            val destination = File(downloadDir, fileName)
            if (destination.exists()) destination.delete()
            copyUrlToFile(assetUrl, destination)
            destination
        }

    private fun verifyDownloadedApk(file: File) {
        val info =
            appContext.packageManager.getPackageArchiveInfo(file.absolutePath, 0)
                ?: throw IllegalStateException("Downloaded file was not an app")
        if (info.packageName != appContext.packageName) {
            throw IllegalStateException("Downloaded app did not match Iris Chat")
        }
        val downloadedVersion = info.longVersionCodeCompat()
        val currentVersion = appContext.packageManager.currentPackageInfo().longVersionCodeCompat()
        if (downloadedVersion <= currentVersion) {
            throw IllegalStateException("Downloaded app was not newer")
        }
    }

    private fun readString(url: String): String =
        String(readBytes(url), Charsets.UTF_8)

    private fun readBytes(url: String): ByteArray {
        val uri = URI(url)
        if (uri.scheme.equals("file", ignoreCase = true)) {
            return File(uri).readBytes()
        }
        val request = Request.Builder().url(url).build()
        httpClient.newCall(request).execute().use { response ->
            if (!response.isSuccessful) {
                throw IllegalStateException("Update server returned ${response.code}")
            }
            return response.body.bytes()
        }
    }

    private fun copyUrlToFile(url: String, destination: File) {
        val uri = URI(url)
        if (uri.scheme.equals("file", ignoreCase = true)) {
            File(uri).copyTo(destination, overwrite = true)
            return
        }
        val request = Request.Builder().url(url).build()
        httpClient.newCall(request).execute().use { response ->
            if (!response.isSuccessful) {
                throw IllegalStateException("Update server returned ${response.code}")
            }
            response.body.byteStream().use { input ->
                destination.outputStream().use { output ->
                    input.copyTo(output)
                }
            }
        }
    }

    private fun supportsSelfUpdate(): Boolean =
        BuildConfig.SELF_UPDATE_ENABLED && !isKnownStoreInstall()

    private fun isKnownStoreInstall(): Boolean {
        val installer = appContext.packageManager.installerPackageNameCompat(appContext.packageName)
            ?: return false
        return installer in STORE_INSTALLERS
    }

    private fun PackageManager.installerPackageNameCompat(packageName: String): String? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            runCatching { getInstallSourceInfo(packageName).installingPackageName }.getOrNull()
        } else {
            @Suppress("DEPRECATION")
            runCatching { getInstallerPackageName(packageName) }.getOrNull()
        }

    private fun PackageManager.currentPackageInfo(): PackageInfo =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getPackageInfo(appContext.packageName, PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            getPackageInfo(appContext.packageName, 0)
        }

    private fun PackageInfo.longVersionCodeCompat(): Long =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            longVersionCode
        } else {
            @Suppress("DEPRECATION")
            versionCode.toLong()
        }

    private fun Context.startActivitySafely(intent: Intent) {
        val launchIntent =
            intent.addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK,
            )
        runCatching { startActivity(launchIntent) }
            .onFailure {
                stateFlow.update { state ->
                    state.copy(status = "Installer unavailable")
                }
            }
    }

    private data class ReleaseManifest(
        val tag: String,
        val assets: List<ReleaseAsset>,
    ) {
        fun preferredAndroidAsset(): ReleaseAsset? =
            assets.firstOrNull { it.name.endsWith(ANDROID_APK_SUFFIX, ignoreCase = true) }
    }

    private data class ReleaseAsset(
        val name: String,
        val path: String,
    )

    private companion object {
        private const val ANDROID_APK_SUFFIX = "-android-arm64.apk"
        private const val APK_MIME_TYPE = "application/vnd.android.package-archive"
        private val AUTO_CHECK_KEY = booleanPreferencesKey("android_self_update_auto_check")
        private val STORE_INSTALLERS =
            setOf(
                "com.android.vending",
                "com.google.android.feedback",
                "org.fdroid.fdroid",
                "com.zapstore.app",
                "com.sec.android.app.samsungapps",
                "com.amazon.venezia",
            )
    }
}

private fun updateManifestUrl(): String =
    BuildConfig.UPDATE_MANIFEST_URL.ifBlank { AndroidSelfUpdateDefaults.manifestUrl }

private fun updatePollIntervalMs(): Long =
    BuildConfig.UPDATE_POLL_SECONDS
        .takeIf { it > 0L }
        ?.let { it * 1_000L }
        ?: 6 * 60 * 60 * 1_000L

internal fun resolveAssetUrl(manifestUrl: String, assetPath: String): String =
    URI(manifestUrl).resolve(assetPath).toString()

internal fun versionIsNewer(candidate: String, current: String): Boolean {
    if (isDevPlaceholderVersion(current)) return false
    val left = versionParts(candidate)
    val right = versionParts(current)
    val count = maxOf(left.size, right.size)
    for (index in 0 until count) {
        val leftValue = left.getOrElse(index) { 0 }
        val rightValue = right.getOrElse(index) { 0 }
        if (leftValue != rightValue) return leftValue > rightValue
    }
    return false
}

private fun isDevPlaceholderVersion(value: String): Boolean =
    versionParts(value).firstOrNull()?.let { it < 2000 } ?: true

private fun versionParts(value: String): List<Int> =
    value
        .trim()
        .trimStart('v', 'V')
        .split('.', '-', '+')
        .mapNotNull { part ->
            part.takeWhile(Char::isDigit).takeIf { it.isNotEmpty() }?.toIntOrNull()
        }

private object AndroidSelfUpdateDefaults {
    const val manifestUrl: String =
        "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest/release.json"
}
