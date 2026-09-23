package to.iris.test.signer;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.Bundle;
import org.json.JSONArray;
import org.json.JSONObject;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;

final class SignerState {
    private final SharedPreferences preferences;

    SignerState(Context context) {
        preferences = context.getSharedPreferences("test-signer", Context.MODE_PRIVATE);
        if (!preferences.contains("secret")) reset();
    }

    void reset() {
        preferences.edit().clear().putString("secret", TestSchnorr.newSecret()).commit();
    }

    String publicKey() { return TestSchnorr.publicKey(preferences.getString("secret", "")); }
    String mode() { return preferences.getString("mode", "manual"); }
    void setMode(String mode) { preferences.edit().putString("mode", mode).commit(); }

    void record(String method, String payload) {
        preferences.edit()
            .putInt("request_count", preferences.getInt("request_count", 0) + 1)
            .putInt(method + "_count", preferences.getInt(method + "_count", 0) + 1)
            .putString("last_method", method)
            .putString("last_payload", payload)
            .commit();
    }

    Bundle status() {
        Bundle result = new Bundle();
        result.putString("pubkey", publicKey());
        result.putInt("request_count", preferences.getInt("request_count", 0));
        result.putInt("get_public_key_count", preferences.getInt("get_public_key_count", 0));
        result.putInt("sign_event_count", preferences.getInt("sign_event_count", 0));
        result.putString("last_method", preferences.getString("last_method", ""));
        result.putString("last_payload", preferences.getString("last_payload", ""));
        result.putString("last_event", preferences.getString("last_event", ""));
        return result;
    }

    JSONObject signEvent(String json, boolean mutate) throws Exception {
        JSONObject event = new JSONObject(json);
        if (!publicKey().equals(event.getString("pubkey"))) throw new IllegalArgumentException("Wrong user");
        if (mutate) event.put("content", event.optString("content") + "fixture mutation");
        JSONArray canonical = new JSONArray().put(0).put(event.getString("pubkey"))
            .put(event.getLong("created_at")).put(event.getInt("kind"))
            .put(event.getJSONArray("tags")).put(event.getString("content"));
        // Android JSON escapes slashes; Nostr canonical serialization does not.
        byte[] id = TestSchnorr.sha256(canonical.toString().replace("\\/", "/").getBytes(StandardCharsets.UTF_8));
        byte[] aux = new byte[32]; new SecureRandom().nextBytes(aux);
        event.put("id", TestSchnorr.hex(id));
        event.put("sig", TestSchnorr.sign(preferences.getString("secret", ""), id, aux));
        preferences.edit().putString("last_event", event.toString()).commit();
        return event;
    }
}
