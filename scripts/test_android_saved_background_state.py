import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

from android_saved_background_state import inspect_database, require_preserved_state


class SavedReceiverStateTest(unittest.TestCase):
    def test_snapshot_preserves_identity_counts_and_detects_other_preference_changes_without_exporting_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "core.sqlite3")
            c = sqlite3.connect(path)
            c.executescript("""
                CREATE TABLE app_meta(key TEXT,value TEXT);
                CREATE TABLE ndr_kv(owner_pubkey_hex TEXT,device_pubkey_hex TEXT);
                CREATE TABLE preferences(id INTEGER,desktop_notifications_enabled INTEGER,voice_calls_enabled INTEGER,
                  video_calls_enabled INTEGER,nostr_relay_urls_json TEXT,private_proxy_key TEXT);
                CREATE TABLE threads(id INTEGER); CREATE TABLE messages(id INTEGER); CREATE TABLE groups(id INTEGER);
                INSERT INTO threads VALUES(1); INSERT INTO messages VALUES(1);
                INSERT INTO preferences VALUES(1,0,0,0,'["ws://127.0.0.1:1234"]','private-value');
            """)
            c.execute("INSERT INTO app_meta VALUES('account_owner_pubkey_hex',?)", ("a" * 64,))
            c.execute("INSERT INTO ndr_kv VALUES(?,?)", ("a" * 64, "b" * 64)); c.commit()
            before = inspect_database(path); before["permissions"] = {"example": "false"}
            self.assertNotIn("private-value", json.dumps(before))
            self.assertEqual(dict(threads=1, messages=1, groups=0), before["history_counts"])
            c.execute("UPDATE preferences SET desktop_notifications_enabled=1,voice_calls_enabled=1,video_calls_enabled=1"); c.commit()
            after = inspect_database(path); after["permissions"] = before["permissions"]
            require_preserved_state(before, after, {key: 1 for key in before["alerts"]})
            c.execute("UPDATE preferences SET private_proxy_key='changed'"); c.commit()
            altered = inspect_database(path); altered["permissions"] = before["permissions"]
            with self.assertRaises(AssertionError): require_preserved_state(before, altered, after["alerts"])
            c.close()


if __name__ == "__main__": unittest.main()
