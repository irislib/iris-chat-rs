package to.iris.chat.push

import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.google.android.gms.tasks.Task
import com.google.firebase.messaging.FirebaseMessaging
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import to.iris.chat.BuildConfig
import to.iris.chat.rust.AppState
import to.iris.chat.rust.MobilePushSubscriptionRequest
import to.iris.chat.rust.buildMobilePushCreateSubscriptionRequest
import to.iris.chat.rust.buildMobilePushDeleteSubscriptionRequest
import to.iris.chat.rust.buildMobilePushListSubscriptionsRequest
import to.iris.chat.rust.buildMobilePushUpdateSubscriptionRequest
import to.iris.chat.rust.mobilePushSubscriptionIdKey

class AndroidMobilePushRuntime(
    private val dataStore: DataStore<Preferences>,
    private val httpClient: OkHttpClient = OkHttpClient(),
    private val messaging: FirebaseMessaging = FirebaseMessaging.getInstance(),
) {
    private val syncMutex = Mutex()
    @Volatile private var lastSyncSignature: String? = null

    suspend fun sync(
        state: AppState,
        ownerNsec: String?,
    ): Boolean = syncMutex.withLock {
        val owner = state.mobilePush.ownerPubkeyHex?.trim()?.ifEmpty { null }
        val ownerSecret = ownerNsec?.trim()?.ifEmpty { null }
        val authors = state.mobilePush.messageAuthorPubkeys
        val inviteResponses = state.mobilePush.inviteResponsePubkeys
        val enabled = state.preferences.desktopNotificationsEnabled
        val serverOverride = userServerOverride(state) ?: buildServerOverride()
        val signature =
            listOf(
                if (enabled) "1" else "0",
                owner.orEmpty(),
                if (ownerSecret == null) "0" else "1",
                authors.joinToString(","),
                inviteResponses.joinToString(","),
                serverOverride.orEmpty(),
            ).joinToString("|")
        if (signature == lastSyncSignature) {
            return@withLock true
        }

        val storageKeyName = mobilePushSubscriptionIdKey(PLATFORM_KEY)
        val storageKey = stringPreferencesKey(storageKeyName)
        if (!enabled || ownerSecret == null || (authors.isEmpty() && inviteResponses.isEmpty())) {
            val disabled = disableStoredSubscription(ownerSecret, storageKey, serverOverride)
            if (disabled) {
                lastSyncSignature = signature
            }
            return@withLock disabled
        }

        val token = messaging.token.await()?.trim()?.ifEmpty { null }
        if (token == null) {
            Log.w(TAG, "FCM token unavailable; mobile push sync will retry")
            return@withLock false
        }
        val storedId = currentStoredId(storageKey)
        val existingId = resolveExistingSubscriptionId(ownerSecret, token, storedId, serverOverride)
        if (existingId != null && updateSubscription(ownerSecret, existingId, token, authors, inviteResponses, storageKey, serverOverride)) {
            lastSyncSignature = signature
            return@withLock true
        }
        val created = createSubscription(ownerSecret, token, authors, inviteResponses, storageKey, serverOverride)
        if (created) {
            lastSyncSignature = signature
        }
        created
    }

    fun invalidate() {
        lastSyncSignature = null
    }

    suspend fun unregisterStoredSubscription(
        state: AppState,
        ownerNsec: String?,
    ) {
        val storageKeyName = mobilePushSubscriptionIdKey(PLATFORM_KEY)
        val storageKey = stringPreferencesKey(storageKeyName)
        val serverOverride = userServerOverride(state) ?: buildServerOverride()
        disableStoredSubscription(ownerNsec?.trim()?.ifEmpty { null }, storageKey, serverOverride)
        lastSyncSignature = null
    }

    private suspend fun resolveExistingSubscriptionId(
        ownerNsec: String,
        pushToken: String,
        storedId: String?,
        serverOverride: String?,
    ): String? {
        val request =
            buildMobilePushListSubscriptionsRequest(
                ownerNsec = ownerNsec,
                platformKey = PLATFORM_KEY,
                isRelease = !BuildConfig.DEBUG,
                serverUrlOverride = serverOverride,
            ) ?: return storedId
        val response = perform(request)
        val body = response.body ?: return storedId
        val subscriptions = runCatching { JSONObject(body) }.getOrNull() ?: return storedId
        if (storedId != null && subscriptions.has(storedId)) {
            return storedId
        }
        val keys = subscriptions.keys()
        while (keys.hasNext()) {
            val subscriptionId = keys.next()
            val subscription = subscriptions.optJSONObject(subscriptionId) ?: continue
            val tokens = subscription.optJSONArray("fcm_tokens") ?: continue
            for (index in 0 until tokens.length()) {
                if (tokens.optString(index) == pushToken) {
                    return subscriptionId
                }
            }
        }
        return null
    }

    private suspend fun updateSubscription(
        ownerNsec: String,
        subscriptionId: String,
        pushToken: String,
        authors: List<String>,
        inviteResponses: List<String>,
        storageKey: Preferences.Key<String>,
        serverOverride: String?,
    ): Boolean {
        val request =
            buildMobilePushUpdateSubscriptionRequest(
                ownerNsec = ownerNsec,
                subscriptionId = subscriptionId,
                platformKey = PLATFORM_KEY,
                pushToken = pushToken,
                apnsTopic = null,
                messageAuthorPubkeys = authors,
                inviteResponsePubkeys = inviteResponses,
                isRelease = !BuildConfig.DEBUG,
                serverUrlOverride = serverOverride,
            ) ?: return false
        val response = perform(request)
        if (response.isSuccess) {
            dataStore.edit { preferences -> preferences[storageKey] = subscriptionId }
            return true
        }
        if (response.statusCode == 404) {
            dataStore.edit { preferences -> preferences.remove(storageKey) }
        }
        return false
    }

    private suspend fun createSubscription(
        ownerNsec: String,
        pushToken: String,
        authors: List<String>,
        inviteResponses: List<String>,
        storageKey: Preferences.Key<String>,
        serverOverride: String?,
    ): Boolean {
        val request =
            buildMobilePushCreateSubscriptionRequest(
                ownerNsec = ownerNsec,
                platformKey = PLATFORM_KEY,
                pushToken = pushToken,
                apnsTopic = null,
                messageAuthorPubkeys = authors,
                inviteResponsePubkeys = inviteResponses,
                isRelease = !BuildConfig.DEBUG,
                serverUrlOverride = serverOverride,
            ) ?: return false
        val response = perform(request)
        if (!response.isSuccess) {
            Log.w(TAG, "mobile push subscription create failed with status ${response.statusCode}")
            return false
        }
        val id =
            response.body
                ?.let { runCatching { JSONObject(it) }.getOrNull() }
                ?.optString("id")
                ?.trim()
                ?.ifEmpty { null }
                ?: return false
        dataStore.edit { preferences -> preferences[storageKey] = id }
        return true
    }

    private suspend fun disableStoredSubscription(
        ownerNsec: String?,
        storageKey: Preferences.Key<String>,
        serverOverride: String?,
    ): Boolean {
        val storedId = currentStoredId(storageKey) ?: return true
        if (ownerNsec == null) {
            dataStore.edit { preferences -> preferences.remove(storageKey) }
            return true
        }
        val request =
            buildMobilePushDeleteSubscriptionRequest(
                ownerNsec = ownerNsec,
                subscriptionId = storedId,
                platformKey = PLATFORM_KEY,
                isRelease = !BuildConfig.DEBUG,
                serverUrlOverride = serverOverride,
            ) ?: return false
        val response = perform(request)
        if (response.isSuccess || response.statusCode == 404) {
            dataStore.edit { preferences -> preferences.remove(storageKey) }
            return true
        }
        return false
    }

    private suspend fun currentStoredId(storageKey: Preferences.Key<String>): String? {
        return dataStore.awaitFirst()[storageKey]?.trim()?.ifEmpty { null }
    }

    private fun perform(request: MobilePushSubscriptionRequest): MobilePushHttpResponse {
        val builder =
            Request.Builder()
                .url(request.url)
                .header("accept", "application/json")
                .header("authorization", request.authorizationHeader)
        val bodyJson = request.bodyJson
        if (bodyJson != null) {
            builder.header("content-type", "application/json")
            builder.method(request.method, bodyJson.toRequestBody(JSON_MEDIA_TYPE))
        } else {
            builder.method(request.method, null)
        }
        return runCatching {
            httpClient.newCall(builder.build()).execute().use { response ->
                MobilePushHttpResponse(response.code, response.body.string())
            }
        }.getOrElse { error ->
            Log.w(TAG, "mobile push subscription request failed", error)
            MobilePushHttpResponse(0, null)
        }
    }

    private fun userServerOverride(state: AppState): String? =
        state.preferences.mobilePushServerUrl.trim().ifEmpty { null }

    private fun buildServerOverride(): String? =
        BuildConfig.MOBILE_PUSH_SERVER_URL.trim().ifEmpty { null }

    private companion object {
        const val TAG = "IrisPush"
        const val PLATFORM_KEY = "android"
        val JSON_MEDIA_TYPE = "application/json".toMediaType()
    }
}

private data class MobilePushHttpResponse(
    val statusCode: Int,
    val body: String?,
) {
    val isSuccess: Boolean = statusCode in 200..299
}

private suspend fun <T> Task<T>.await(): T? =
    suspendCancellableCoroutine { continuation ->
        addOnCompleteListener { task ->
            if (task.isSuccessful) {
                continuation.resume(task.result)
            } else {
                continuation.resume(null)
            }
        }
    }

private suspend fun DataStore<Preferences>.awaitFirst(): Preferences {
    return data.first()
}
