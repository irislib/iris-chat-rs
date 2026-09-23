package to.iris.chat

import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import to.iris.chat.core.Nip55Signer

/** Each exchange keeps its own result key so a late response cannot answer a later request. */
fun ComponentActivity.attachNip55Signer(signer: Nip55Signer) {
    lifecycleScope.launch {
        repeatOnLifecycle(Lifecycle.State.STARTED) {
            signer.pendingRequest.collectLatest { pending ->
                if (pending == null) return@collectLatest
                val launcher =
                    activityResultRegistry.register(
                        "nip55:${pending.id}",
                        ActivityResultContracts.StartActivityForResult(),
                    ) { result ->
                        signer.onActivityResult(pending.id, result.resultCode, result.data)
                    }
                try {
                    signer.claimRequest(pending.id)?.let { request ->
                        try {
                            launcher.launch(request.intent())
                        } catch (_: RuntimeException) {
                            signer.launchFailed(request.id)
                        }
                    }
                    awaitCancellation()
                } finally {
                    launcher.unregister()
                }
            }
        }
    }
}
