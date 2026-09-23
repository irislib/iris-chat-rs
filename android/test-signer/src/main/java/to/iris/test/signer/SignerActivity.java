package to.iris.test.signer;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;
import org.json.JSONObject;

/** A real NIP-55 Intent endpoint in a different package/process from Iris. */
public final class SignerActivity extends Activity {
    private SignerState state;
    private String method;
    private String payload;

    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        handle(getIntent());
    }

    @Override public void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handle(intent);
    }

    private void handle(Intent intent) {
        state = new SignerState(this);
        method = intent.getStringExtra("type");
        payload = intent.getData() == null ? "" : intent.getData().getSchemeSpecificPart();
        state.record(method == null ? "unknown" : method, payload);

        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(40, 80, 40, 40);
        TextView title = new TextView(this);
        title.setText("Iris test signer\n" + method);
        title.setTextSize(24);
        layout.addView(title);
        Button approve = new Button(this);
        approve.setText("Approve");
        approve.setOnClickListener(view -> respond(false));
        layout.addView(approve);
        Button reject = new Button(this);
        reject.setText("Reject");
        reject.setOnClickListener(view -> respond(true));
        layout.addView(reject);
        setContentView(layout);
        // Automated fixture modes still use the same public Intent/result boundary.
        if (state.mode().equals("approve") || state.mode().equals("wrong_event") || state.mode().equals("wrong_id")) {
            respond(false);
        } else if (state.mode().equals("reject") || (state.mode().equals("reject_sign") && "sign_event".equals(method))) {
            respond(true);
        }
    }

    private void respond(boolean rejected) {
        Intent response = new Intent();
        String requestId = getIntent().getStringExtra("id");
        response.putExtra("id", state.mode().equals("wrong_id") ? "wrong-id" : requestId);
        try {
            if (rejected) {
                response.putExtra("rejected", true);
            } else if ("get_public_key".equals(method)) {
                response.putExtra("result", state.publicKey());
                response.putExtra("package", getPackageName());
            } else if ("sign_event".equals(method)) {
                if (!state.publicKey().equals(getIntent().getStringExtra("current_user"))) {
                    throw new IllegalArgumentException("Missing or wrong current_user");
                }
                JSONObject event = state.signEvent(payload, state.mode().equals("wrong_event"));
                response.putExtra("result", event.getString("sig"));
                response.putExtra("event", event.toString());
            } else {
                throw new IllegalArgumentException("Unsupported method");
            }
            setResult(RESULT_OK, response);
        } catch (Exception error) {
            response.putExtra("error", error.getMessage());
            setResult(RESULT_CANCELED, response);
        }
        finish();
    }
}
