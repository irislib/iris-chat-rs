"""Preserve the existing isolated Bluetooth peer through success and interruption."""
import base64
from contextlib import closing
import json
import re
import sqlite3
import subprocess

from android_saved_background_state import inspect_database, read_stopped_test_state, require_preserved_state
from android_saved_history import fingerprint, require_preserved_history

PACKAGE = "to.iris.chat.blegate"
SETTINGS = ("send_read_receipts", "nearby_enabled", "nearby_bluetooth_enabled", "nearby_lan_enabled")


def inspect_ble_database(path, probe):
    with closing(sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        contacts = [row[0] for row in db.execute(
            "SELECT DISTINCT chat_id FROM messages WHERE body=? AND is_outgoing=0", (probe,))]
        assert len(contacts) <= 1 and all(re.fullmatch(r"[a-f0-9]{64}", c) for c in contacts), "Ambiguous Bluetooth probe"
        contact = contacts[0] if contacts else None
        settings = dict(zip(SETTINGS, db.execute("SELECT " + ",".join(SETTINGS) + " FROM preferences WHERE id=1").fetchone()))
        assert all(value in (0, 1) for value in settings.values())
        accepted = json.loads(db.execute("SELECT accepted_owner_pubkeys_json FROM preferences WHERE id=1").fetchone()[0])
    result = inspect_database(path, include_history=True, new_contact=contact)
    assert result["devices"] and all(re.fullmatch(r"[a-f0-9]{64}", d) for d in result["devices"])
    result.update(ble_settings=settings, test_contact=contact,
                  accepted_fingerprints=[fingerprint(c) for c in accepted])
    return result


def require_ble_preserved(before, after):
    contact = after["test_contact"]
    new_contact = contact if contact and fingerprint(contact) not in before["accepted_fingerprints"] else None
    require_preserved_state(before, after, before["alerts"], new_contact=new_contact)
    assert before["permission_flags"] == after["permission_flags"], "Permission flags changed"
    assert before["ble_settings"] == after["ble_settings"], "Bluetooth test settings were not restored"
    return require_preserved_history(before["history"], after["history"], allowed_new_chat=contact)


class BlePreservation:
    def __init__(self, gate):
        self.gate = gate
        self.before = None
        self.started = False

    def device(self, *args, check=True, binary=False):
        result = subprocess.run(["adb", "-s", self.gate.args.android_serial, *args],
                                capture_output=True, text=not binary, timeout=30)
        if check and result.returncode:
            raise RuntimeError("Bluetooth account preservation device command failed")
        return result.stdout

    def read(self):
        result = read_stopped_test_state(self.device, lambda *a: self.device(*a, binary=True), PACKAGE,
                                        lambda path: inspect_ble_database(path, self.gate.probe))
        permissions = self.device("shell", "dumpsys", "package", PACKAGE)
        result["permission_flags"] = dict(re.findall(
            r"(android\.permission\.[A-Z_]+): granted=(?:true|false), flags=\[([^\]]*)\]", permissions))
        assert result["permission_flags"], "Missing permission flag evidence"
        return result

    def capture(self):
        before = self.read()
        assert before["test_contact"] is None, "Probe already exists in saved account"
        sdk = int(self.device("shell", "getprop", "ro.build.version.sdk"))
        self.borrowed_permissions = ["android.permission." + p for p in (
            ("BLUETOOTH_SCAN", "BLUETOOTH_CONNECT", "BLUETOOTH_ADVERTISE") if sdk >= 31 else ("ACCESS_FINE_LOCATION",))]
        assert all(p in before["permissions"] for p in self.borrowed_permissions), "Missing Bluetooth permission evidence"
        (self.gate.out / "android-account-before.json").write_text(json.dumps(before, indent=2) + "\n")
        self.before = before

    def arguments(self):
        assert self.before is not None, "Capture the saved account before instrumentation"
        return ["-e", "preserve_owner", self.before["owner"], "-e", "preserve_devices", ",".join(self.before["devices"])]

    def restore(self):
        if self.before is None or not self.started:
            return
        restored = self.gate.restored
        try:
            self.device("shell", "am", "force-stop", PACKAGE)
            settings = {**self.before["ble_settings"], "relays": self.before["relays"]}
            encoded = base64.b64encode(json.dumps(settings).encode()).decode()
            self.gate.action("restore_preferences", {"saved_preferences_restored": "true"},
                             ["-e", "saved_preferences", encoded], test_class="BleSavedAccountTest")
            restored["android_preferences"] = True
        except Exception:
            restored["android_preferences"] = False
        finally:
            try:
                self.device("shell", "am", "force-stop", PACKAGE)
                restored["android_account_stopped"] = True
            except Exception:
                restored["android_account_stopped"] = False
        # Revoking a borrowed permission can kill the app, so restore it only
        # after the app has persisted its preferences and stopped.
        restored["android_permissions"] = True
        for permission in self.borrowed_permissions:
            try:
                current = dict(re.findall(r"(android\.permission\.[A-Z_]+): granted=(true|false)",
                    self.device("shell", "dumpsys", "package", PACKAGE)))
                if current[permission] != self.before["permissions"][permission]:
                    action = "grant" if self.before["permissions"][permission] == "true" else "revoke"
                    self.device("shell", "pm", action, PACKAGE, permission)
            except Exception:
                restored["android_permissions"] = False
        try:
            after = self.read()
            (self.gate.out / "android-account-after.json").write_text(json.dumps(after, indent=2) + "\n")
            self.gate.result["androidTestHistoryAdded"] = require_ble_preserved(self.before, after)
            restored["android_account_preserved"] = True
        except Exception:
            restored["android_account_preserved"] = False
