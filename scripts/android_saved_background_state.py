"""Read only the existing synthetic receiver's identity, settings and history counts."""
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import tempfile

PACKAGE = "to.iris.chat.backgroundtest"
ALERTS = ("desktop_notifications_enabled", "voice_calls_enabled", "video_calls_enabled")
STORE = "files/datastore/iris_chat_secure_store.preferences_pb.preferences_pb"


def inspect_database(path, *, include_history=False, new_contact=None):
    connection = sqlite3.connect(Path(path).resolve().as_uri() + "?mode=ro", uri=True)
    try:
        owner = connection.execute("SELECT value FROM app_meta WHERE key='account_owner_pubkey_hex'").fetchone()[0]
        assert re.fullmatch(r"[a-f0-9]{64}", owner)
        devices = sorted(row[0] for row in connection.execute(
            "SELECT DISTINCT device_pubkey_hex FROM ndr_kv WHERE owner_pubkey_hex=?", (owner,)))
        cursor = connection.execute("SELECT * FROM preferences WHERE id=1")
        preferences = dict(zip((column[0] for column in cursor.description), cursor.fetchone()))
        policy = {key: value for key, value in preferences.items() if key not in ALERTS}
        result = {"owner": owner, "devices": devices,
                "alerts": {key: preferences[key] for key in ALERTS},
                "relays": json.loads(preferences["nostr_relay_urls_json"]),
                "other_preferences_sha256": hashlib.sha256(json.dumps(policy, sort_keys=True).encode()).hexdigest(),
                "history_counts": {table: connection.execute("SELECT count(*) FROM " + table).fetchone()[0]
                                   for table in ("threads", "messages", "groups")}}
        if new_contact is not None:
            assert re.fullmatch(r"[a-f0-9]{64}", new_contact)
            accepted = json.loads(policy["accepted_owner_pubkeys_json"])
            assert isinstance(accepted, list) and accepted.count(new_contact) <= 1
            if new_contact in accepted:
                prior = {**policy, "accepted_owner_pubkeys_json": json.dumps(
                    [owner for owner in accepted if owner != new_contact], separators=(",", ":"))}
                result["test_contact_acceptance"] = {"owner": new_contact,
                    "prior_preferences_sha256": hashlib.sha256(json.dumps(prior, sort_keys=True).encode()).hexdigest()}
        if include_history:
            from android_saved_history import history_snapshot
            result["history"] = history_snapshot(connection)
        return result
    finally:
        connection.close()


def read_saved_state(adb, binary, *, include_history=False, new_contact=None):
    assert not adb("shell", "pidof", PACKAGE, check=False).strip(), "Stop receiver before a consistent state read"
    # A stopped process cannot write between these reads. Copy any retained WAL
    # alongside the DB so the private, read-only inspection sees committed data.
    with tempfile.TemporaryDirectory(prefix="iris-saved-state-") as directory:
        target = Path(directory) / "core.sqlite3"
        for suffix in ("", "-wal"):
            source = "files/core.sqlite3" + suffix
            if suffix and not adb("shell", "run-as", PACKAGE, "ls", source, check=False).strip():
                continue
            size = int(adb("shell", "run-as", PACKAGE, "stat", "-c", "%s", source))
            assert 0 <= size < 64 * 1024 * 1024, "Unexpected receiver database size"
            payload = binary("exec-out", "run-as", PACKAGE, "cat", source)
            assert len(payload) == size
            with os.fdopen(os.open(str(target) + suffix, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
                file.write(payload)
        result = inspect_database(target, include_history=include_history, new_contact=new_contact)
    result["account_store_sha256"] = adb("shell", "run-as", PACKAGE, "sha256sum", STORE).split()[0]
    permissions = adb("shell", "dumpsys", "package", PACKAGE)
    result["permissions"] = dict(re.findall(r"(android\.permission\.[A-Z_]+): granted=(true|false)", permissions))
    return result


def require_preserved_state(before, after, alerts, *, new_contact=None):
    for field in ("owner", "devices", "relays", "permissions"):
        assert after[field] == before[field], "Saved receiver changed: " + field
    if after["other_preferences_sha256"] != before["other_preferences_sha256"]:
        acceptance = after.get("test_contact_acceptance", {})
        assert new_contact is not None and acceptance.get("owner") == new_contact, "Saved receiver preferences changed"
        assert acceptance.get("prior_preferences_sha256") == before["other_preferences_sha256"], "Unrelated preferences changed"
    assert after["alerts"] == alerts, "Receiver alert preferences were not restored"


def installed_hash(adb, package):
    assert package in (PACKAGE, PACKAGE + ".test")
    paths = adb("shell", "pm", "path", package).strip().splitlines()
    assert len(paths) == 1 and paths[0].startswith("package:")
    path = paths[0][len("package:"):]
    assert re.fullmatch(r"/data/app/[-A-Za-z0-9_./=+~]+/base\.apk", path)
    digest = adb("shell", "sha256sum", path).split()[0]
    assert re.fullmatch(r"[a-f0-9]{64}", digest)
    return digest
