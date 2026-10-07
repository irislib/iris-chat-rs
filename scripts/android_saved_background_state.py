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


def inspect_database(path):
    connection = sqlite3.connect(Path(path).resolve().as_uri() + "?mode=ro", uri=True)
    try:
        owner = connection.execute("SELECT value FROM app_meta WHERE key='account_owner_pubkey_hex'").fetchone()[0]
        assert re.fullmatch(r"[a-f0-9]{64}", owner)
        devices = sorted(row[0] for row in connection.execute(
            "SELECT DISTINCT device_pubkey_hex FROM ndr_kv WHERE owner_pubkey_hex=?", (owner,)))
        cursor = connection.execute("SELECT * FROM preferences WHERE id=1")
        preferences = dict(zip((column[0] for column in cursor.description), cursor.fetchone()))
        policy = {key: value for key, value in preferences.items() if key not in ALERTS}
        return {"owner": owner, "devices": devices,
                "alerts": {key: preferences[key] for key in ALERTS},
                "relays": json.loads(preferences["nostr_relay_urls_json"]),
                "other_preferences_sha256": hashlib.sha256(json.dumps(policy, sort_keys=True).encode()).hexdigest(),
                "history_counts": {table: connection.execute("SELECT count(*) FROM " + table).fetchone()[0]
                                   for table in ("threads", "messages", "groups")}}
    finally:
        connection.close()


def read_saved_state(adb, binary):
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
        result = inspect_database(target)
    result["account_store_sha256"] = adb("shell", "run-as", PACKAGE, "sha256sum", STORE).split()[0]
    permissions = adb("shell", "dumpsys", "package", PACKAGE)
    result["permissions"] = dict(re.findall(r"(android\.permission\.[A-Z_]+): granted=(true|false)", permissions))
    return result


def require_preserved_state(before, after, alerts):
    for field in ("owner", "devices", "relays", "other_preferences_sha256", "permissions"):
        assert after[field] == before[field], "Saved receiver changed: " + field
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
