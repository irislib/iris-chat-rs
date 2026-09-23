package to.iris.test.signer;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;

/** Test driver controls; this component exists only in the separate debug fixture APK. */
public final class TestControlProvider extends ContentProvider {
    @Override public boolean onCreate() { return true; }

    @Override public Bundle call(String method, String arg, Bundle extras) {
        SignerState state = new SignerState(getContext());
        switch (method) {
            case "reset": state.reset(); break;
            case "mode": state.setMode(arg); break;
            case "sign_fixture":
                try {
                    Bundle result = new Bundle();
                    result.putString("event", state.signEvent(arg, false).toString());
                    return result;
                } catch (Exception error) { throw new IllegalArgumentException(error); }
            case "status": break;
            default: throw new IllegalArgumentException("Unknown control");
        }
        return state.status();
    }

    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String sort) { return null; }
    @Override public String getType(Uri uri) { return null; }
    @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri, String selection, String[] args) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri, ContentValues values, String selection, String[] args) { throw new UnsupportedOperationException(); }
}
