import copy
import json
from pathlib import Path
import sqlite3
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from android_ble_preservation import inspect_ble_database, require_ble_preserved
from physical_ble_idle import Gate


class BlePreservationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "core.sqlite3"
        self.db = sqlite3.connect(self.path)
        self.addCleanup(self.db.close)
        self.db.executescript("""
            CREATE TABLE app_meta(key TEXT,value TEXT);
            CREATE TABLE ndr_kv(owner_pubkey_hex TEXT,device_pubkey_hex TEXT);
            CREATE TABLE preferences(id INTEGER,desktop_notifications_enabled INTEGER,voice_calls_enabled INTEGER,
                video_calls_enabled INTEGER,nostr_relay_urls_json TEXT,accepted_owner_pubkeys_json TEXT,
                send_read_receipts INTEGER,nearby_enabled INTEGER,nearby_bluetooth_enabled INTEGER,nearby_lan_enabled INTEGER,
                private_proxy_key TEXT);
            INSERT INTO preferences VALUES(1,0,0,0,'["ws://127.0.0.1:1234"]','[]',0,0,0,1,'secret');
            CREATE TABLE threads(chat_id TEXT PRIMARY KEY,draft TEXT);
            CREATE TABLE messages(chat_id TEXT,id TEXT,kind TEXT,author TEXT,body TEXT,is_outgoing INTEGER,
                created_at_secs INTEGER,attachments_json TEXT);
            CREATE TABLE groups(group_id TEXT,name TEXT,picture TEXT,created_at_ms INTEGER,group_json TEXT);
            INSERT INTO threads VALUES('original','saved draft');
            INSERT INTO messages VALUES('original','old','text','peer','saved message',0,1,'[]');
        """)
        self.db.execute("INSERT INTO app_meta VALUES('account_owner_pubkey_hex',?)", ("a" * 64,))
        self.db.execute("INSERT INTO ndr_kv VALUES(?,?)", ("a" * 64, "b" * 64))
        self.db.commit()
        self.permission = "android.permission.BLUETOOTH_SCAN"
        self.before = self.read()

    def read(self):
        self.db.commit()
        value = inspect_ble_database(self.path, "unique-test-probe")
        value.update(permissions={self.permission: "false"}, permission_flags={self.permission: " USER_SET "})
        return value

    def add_probe(self, contact="c" * 64):
        self.db.execute("INSERT INTO threads VALUES(?,'')", (contact,))
        self.db.execute("INSERT INTO messages VALUES(?,'new','text',?,'unique-test-probe',0,2,'[]')", (contact, contact))
        self.db.execute("UPDATE preferences SET accepted_owner_pubkeys_json=?", (json.dumps([contact], separators=(",", ":")),))

    def test_real_database_allows_only_exact_probe_contact_and_preserves_saved_content(self):
        self.add_probe()
        after = self.read()
        self.assertEqual(require_ble_preserved(self.before, after), dict(threads=1, messages=1, groups=0))
        self.assertNotIn("secret", json.dumps(after))
        self.assertNotIn("saved message", json.dumps(after))
        self.assertEqual(after["ble_settings"]["nearby_lan_enabled"], 1)

    def test_equal_counts_cannot_hide_lost_identity_settings_or_history(self):
        original = self.path.read_bytes()
        statements = ["UPDATE app_meta SET value='" + "d" * 64 + "'", "DELETE FROM ndr_kv",
                      "UPDATE messages SET body='replacement'", "UPDATE threads SET draft='replacement'",
                      "UPDATE preferences SET send_read_receipts=1", "UPDATE preferences SET private_proxy_key='changed'",
                      "UPDATE preferences SET accepted_owner_pubkeys_json='[\"" + "d" * 64 + "\"]'"]
        for sql in statements:
            with self.subTest(sql=sql):
                self.db.execute(sql)
                with self.assertRaises((AssertionError, TypeError)):
                    require_ble_preserved(self.before, self.read())
                self.db.close()
                self.path.write_bytes(original)
                self.db = sqlite3.connect(self.path)
                self.addCleanup(self.db.close)

    def test_ambiguous_probe_cannot_authorize_unrelated_contact(self):
        self.add_probe()
        self.add_probe("d" * 64)
        with self.assertRaises(AssertionError): self.read()

    def test_permission_grants_and_flags_must_match(self):
        for key, changed in (("permissions", "true"), ("permission_flags", " USER_FIXED ")):
            after = copy.deepcopy(self.before)
            after[key][self.permission] = changed
            with self.assertRaises(AssertionError): require_ble_preserved(self.before, after)

    def test_interrupted_preference_restore_still_stops_app_and_revokes_borrowed_permission(self):
        gate = Gate(SimpleNamespace(artifact_dir=Path(self.directory.name) / "run", android_serial="test"))
        guard = gate.saved_account
        guard.before, guard.started = self.before, True
        guard.borrowed_permissions = [self.permission]
        with patch.object(gate, "action", side_effect=RuntimeError("Disconnected during restore")), \
                patch.object(guard, "device", return_value=self.permission + ": granted=true") as device, \
                patch.object(guard, "read", return_value=self.before):
            gate.restore()
        device.assert_any_call("shell", "pm", "revoke", "to.iris.chat.blegate", self.permission)
        self.assertTrue(gate.restored["android_account_stopped"])
        self.assertTrue(gate.restored["android_account_preserved"])
        self.assertFalse(gate.result["ok"])
        self.assertFalse(gate.restored["android_preferences"])

    def test_all_controller_actions_require_a_captured_identity(self):
        gate = Gate(SimpleNamespace(artifact_dir=Path(self.directory.name) / "run", android_serial="test"))
        with self.assertRaises(AssertionError): gate.action_command("report_logged_in_identity")
        gate.saved_account.before = self.before
        command = gate.action_command("report_logged_in_identity")
        self.assertIn("preserve_owner", command)
        self.assertIn("preserve_devices", command)
        self.assertNotIn("create_account_and_report_identity", " ".join(command))


if __name__ == "__main__": unittest.main()
