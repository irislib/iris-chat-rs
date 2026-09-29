package to.iris.chat.push

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.Collections
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.core.AppManagerContractDefaults
import to.iris.chat.rust.MobilePushDelayedAuthor

@RunWith(AndroidJUnit4::class)
class MobilePushRuntimeTest {
    @Test
    fun tokenRefreshDuringMessageRegistrationIsNotLost() = tokenRefreshDuringRegistration(false)

    @Test
    fun tokenRefreshDuringCallRegistrationIsNotLost() = tokenRefreshDuringRegistration(true)

    @Test fun timedMuteUsesServerDeadlineWhenSupported() = timedMuteRegistration(true)

    @Test fun timedMuteKeepsOldServersConservativelySilent() = timedMuteRegistration(false)

    private fun timedMuteRegistration(supported: Boolean) = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val file = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "push-timed-${UUID.randomUUID()}.preferences_pb")
        val dataStore = PreferenceDataStoreFactory.create(scope = scope, produceFile = { file })
        val registered = mutableListOf<JSONObject>()
        val http = OkHttpClient.Builder().addInterceptor { chain ->
            val request = chain.request()
            val body = when {
                request.url.encodedPath.endsWith("/info") -> "{\"supports_timed_filters\":$supported}"
                request.method == "GET" -> "{}"
                else -> {
                    val buffer = Buffer()
                    requireNotNull(request.body).writeTo(buffer)
                    registered.add(JSONObject(buffer.readUtf8()))
                    "{\"id\":\"test-subscription\"}"
                }
            }
            Response.Builder().request(request).protocol(Protocol.HTTP_1_1).code(200).message("OK")
                .body(body.toResponseBody("application/json".toMediaType())).build()
        }.build()
        val runtime = AndroidMobilePushRuntime(dataStore, httpClient = http, fetchToken = { "test-token" })
        val initial = AppManagerContractDefaults.initialState()
        val author = "22".repeat(32)
        val state = initial.copy(
            preferences = initial.preferences.copy(mobilePushServerUrl = "https://notifications.invalid"),
            mobilePush = initial.mobilePush.copy(ownerPubkeyHex = "11".repeat(32),
                messageAuthorPubkeys = emptyList(), delayedMessageAuthors = listOf(MobilePushDelayedAuthor(author, 200uL))),
        )
        try {
            assertTrue(runtime.sync(state, "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"))
            val body = registered.single()
            if (supported) {
                assertEquals(200, body.getJSONObject("filter").getInt("since"))
                assertEquals(author, body.getJSONObject("filter").getJSONArray("authors").getString(0))
            } else {
                assertEquals(0, body.getJSONObject("filter").getJSONArray("authors").length())
                assertTrue(!body.toString().contains(author))
            }
        } finally {
            scope.cancel(); file.delete()
            http.dispatcher.executorService.shutdown(); http.connectionPool.evictAll()
        }
    }

    private fun tokenRefreshDuringRegistration(forCalls: Boolean) = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val file = File(
            InstrumentationRegistry.getInstrumentation().targetContext.cacheDir,
            "push-runtime-${UUID.randomUUID()}.preferences_pb",
        )
        val dataStore = PreferenceDataStoreFactory.create(scope = scope, produceFile = { file })
        val token = AtomicReference("old-test-token")
        val registeredTokens = Collections.synchronizedList(mutableListOf<String>())
        val oldRegistrationStarted = CountDownLatch(1)
        val finishOldRegistration = CountDownLatch(1)
        val httpClient = OkHttpClient.Builder().addInterceptor { chain ->
            // Intercept every request: real Rust request construction/signing and
            // runtime persistence, without contacting a server or real Firebase.
            val request = chain.request()
            val body = if (request.method == "GET") {
                "{}"
            } else {
                val buffer = Buffer()
                requireNotNull(request.body).writeTo(buffer)
                val registered = JSONObject(buffer.readUtf8()).getJSONArray("fcm_tokens").getString(0)
                registeredTokens.add(registered)
                if (registered == "old-test-token") {
                    oldRegistrationStarted.countDown()
                    check(finishOldRegistration.await(5, TimeUnit.SECONDS))
                }
                "{\"id\":\"test-subscription\"}"
            }
            Response.Builder().request(request).protocol(Protocol.HTTP_1_1).code(200).message("OK")
                .body(body.toResponseBody("application/json".toMediaType())).build()
        }.build()
        val runtime = AndroidMobilePushRuntime(dataStore, forCalls, httpClient, fetchToken = { token.get() })
        val initial = AppManagerContractDefaults.initialState()
        val state = initial.copy(
            preferences = initial.preferences.copy(mobilePushServerUrl = "https://notifications.invalid"),
            mobilePush = initial.mobilePush.copy(
                ownerPubkeyHex = "11".repeat(32),
                messageAuthorPubkeys = listOf("22".repeat(32)),
                callDevicePubkeyHex = "33".repeat(32),
                callAuthorPubkeys = listOf("22".repeat(32)),
            ),
        )
        // Public test vector, never an actual account or persisted user secret.
        val testSecret = "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"
        try {
            val oldSync = async(Dispatchers.IO) { runtime.sync(state, testSecret) }
            assertTrue("old token must reach registration", oldRegistrationStarted.await(5, TimeUnit.SECONDS))
            token.set("new-test-token")
            runtime.invalidate() // Same operation used by Firebase onNewToken.
            val refreshedSync = async(Dispatchers.IO) { runtime.sync(state, testSecret) }
            finishOldRegistration.countDown()
            assertTrue(oldSync.await())
            assertTrue(refreshedSync.await())
            assertEquals(listOf("old-test-token", "new-test-token"), registeredTokens.toList())
            // A completed, non-invalidated registration may still be cached.
            assertTrue(runtime.sync(state, testSecret))
            assertEquals(2, registeredTokens.size)
        } finally {
            finishOldRegistration.countDown()
            scope.cancel()
            file.delete()
            httpClient.dispatcher.executorService.shutdown()
            httpClient.connectionPool.evictAll()
        }
    }
}
