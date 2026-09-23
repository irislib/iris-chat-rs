package to.iris.chat.core

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.Collections
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.buildLargeTestAppState
import to.iris.chat.rust.peerInputToNpub

/** Lifecycle/correlation checks complement the separate-APK end-to-end suite. */
@RunWith(AndroidJUnit4::class)
class Nip55SignerContractTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val actions = Collections.synchronizedList(mutableListOf<AppAction>())
    private val errors = Collections.synchronizedList(mutableListOf<String>())
    private lateinit var scope: CoroutineScope
    private lateinit var signer: Nip55Signer

    @Before
    fun setup() {
        assumeTrue(
            "Install the :test-signer debug APK",
            context.packageManager.queryIntentActivities(Intent(Intent.ACTION_VIEW, Uri.parse("nostrsigner:")), 0)
                .any { it.activityInfo.packageName == FIXTURE_PACKAGE },
        )
        scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        signer = newSigner()
    }

    @After
    fun teardown() {
        if (::signer.isInitialized) signer.cancel()
        if (::scope.isInitialized) scope.cancel()
    }

    @Test
    fun activity_recreation_claims_once_and_old_results_cannot_complete_a_retry() {
        val original = startAndClaim()
        assertEquals("get_public_key", original.intent().getStringExtra("type"))
        assertNull("Recollecting on resume must not relaunch", signer.claimRequest(original.id))
        signer.cancel()
        val retry = startAndClaim()
        signer.onActivityResult(original.id, Activity.RESULT_OK, publicKeyResult())
        assertTrue(actions.isEmpty())
        assertTrue(signer.busy.value)
        signer.onActivityResult(retry.id, Activity.RESULT_OK, publicKeyResult())
        assertEquals(listOf(AppAction.BeginSignerLogin(OWNER)), actions.toList())
    }

    @Test
    fun process_restart_discards_unmatched_result_and_can_start_again() {
        val oldRequest = startAndClaim()
        val restarted = newSigner()
        restarted.onActivityResult(oldRequest.id, Activity.RESULT_OK, publicKeyResult())
        assertFalse(restarted.busy.value)
        assertTrue(actions.isEmpty())
        restarted.startLogin()
        assertNotNull(restarted.pendingRequest.value)
        restarted.cancel()
    }

    @Test
    fun accepts_amber_npub_and_pins_signing_to_the_selected_app_and_identity() {
        val getKey = startAndClaim()
        signer.onActivityResult(getKey.id, Activity.RESULT_OK, publicKeyResult(peerInputToNpub(OWNER)))
        assertEquals(AppAction.BeginSignerLogin(OWNER), actions.single())
        val unsigned = """{"id":"event-id","pubkey":"$OWNER","created_at":1,"kind":37368,"tags":[],"content":""}"""
        signer.signEvent(AppUpdate.SignerLoginSignEvent("core-request", OWNER, unsigned))
        val sign = signer.claimRequest("core-request")!!
        assertEquals("sign_event", sign.intent().getStringExtra("type"))
        assertEquals(FIXTURE_PACKAGE, sign.intent().`package`)
        assertEquals(OWNER, sign.intent().getStringExtra("current_user"))
        assertNull(signer.claimRequest(sign.id))
        signer.onActivityResult(sign.id, Activity.RESULT_OK, Intent().putExtra("result", "signature"))
        val completed = actions.last() as AppAction.CompleteSignerLogin
        assertEquals(sign.id, completed.requestId)
        assertEquals("signature", JSONObject(completed.signedEventJson).getString("sig"))
        assertEquals("event-id", JSONObject(completed.signedEventJson).getString("id"))
    }

    @Test
    fun refuses_secret_key_and_unselected_package_responses() {
        val getSecret = startAndClaim()
        signer.onActivityResult(getSecret.id, Activity.RESULT_OK, publicKeyResult("nsec1notapublickey"))
        assertFalse(signer.busy.value)
        assertTrue(actions.isEmpty())
        val getPackage = startAndClaim()
        signer.onActivityResult(
            getPackage.id,
            Activity.RESULT_OK,
            publicKeyResult().putExtra("package", "not.an.installed.signer"),
        )
        assertFalse(signer.busy.value)
        assertTrue(actions.isEmpty())
        assertEquals(2, errors.size)
    }

    @Test
    fun coalesced_core_failure_clears_busy_and_allows_retry() {
        val getKey = startAndClaim()
        signer.onActivityResult(getKey.id, Activity.RESULT_OK, publicKeyResult())
        val failed = buildLargeTestAppState(0u, 0u, 0u).apply {
            account = null
            busy.restoringSession = false
            toast = "Couldn’t reach your message servers. Please try again."
        }
        signer.onAppState(failed)
        assertFalse(signer.busy.value)
        assertNull(signer.pendingRequest.value)
        assertNotNull(startAndClaim())
    }

    private fun newSigner() = Nip55Signer(context, scope, { actions += it; true }, { errors += it })

    private fun startAndClaim(): SignerActivityRequest {
        signer.startLogin()
        return signer.claimRequest(signer.pendingRequest.value!!.id)!!
    }

    private fun publicKeyResult(value: String = OWNER): Intent =
        Intent().putExtra("result", value).putExtra("package", FIXTURE_PACKAGE)

    private companion object {
        const val FIXTURE_PACKAGE = "to.iris.test.signer"
        const val OWNER = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    }
}
