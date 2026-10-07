"""One-shot aggregate health checks for the isolated background receiver harness."""
import json
import time

PACKAGE = "to.iris.chat.backgroundtest"
CACHE_FILE = "cache/background-health.json"
FLAGS = ("connect_in_flight", "connect_dirty", "force_reconnect_dirty",
         "publish_drain_in_flight", "publish_drain_dirty", "retry_scheduled")
COUNTS = ("configured_relay_count", "connected_relay_count", "pending_relay_publish_count",
          "retry_backoff_attempt", "pid", "elapsed_realtime_ms")


def classify_health(snapshot, pid, host_relay_reachable):
    """Fail closed; a reachable host listener alone does not prove client connection."""
    assert snapshot.get("schema_version") == 1 and snapshot.get("valid") is True
    for key in COUNTS:
        assert type(snapshot.get(key)) is int and snapshot[key] >= 0, f"Invalid health count: {key}"
    for key in FLAGS + ("expected_relay_matches",):
        assert type(snapshot.get(key)) is bool, f"Invalid health flag: {key}"
    assert snapshot["pid"] == int(pid), "Health query must observe the same receiver process"
    phase = snapshot.get("phase")
    assert phase in ("connecting", "publishing", "backoff", "connected", "offline")
    if not host_relay_reachable or not snapshot["expected_relay_matches"]:
        return "unavailable"
    if snapshot["configured_relay_count"] != 1 or snapshot["connected_relay_count"] != 1:
        return "reconnecting" if phase in ("connecting", "backoff") else "unavailable"
    if phase in ("connecting", "backoff", "offline") or any(snapshot[k] for k in FLAGS[:3]) or snapshot["retry_scheduled"]:
        return "reconnecting"
    if phase == "publishing" or snapshot["pending_relay_publish_count"] or any(snapshot[k] for k in FLAGS[3:5]):
        return "draining"
    return "connected-idle"


def query_health(adb, pid, expected_relay, host_relay_reachable):
    assert adb("shell", "pidof", PACKAGE).strip() == str(pid), "Receiver process changed before query"
    adb("shell", "run-as", PACKAGE, "rm", "-f", CACHE_FILE)
    # The receiver is not exported. Sending as its own UID retains that protection.
    adb("shell", "run-as", PACKAGE, "am", "broadcast", "--user", "current",
        "-n", PACKAGE + "/to.iris.chat.debug.BackgroundHealthReceiver",
        "-a", "to.iris.chat.BACKGROUND_HEALTH", "--ei", "expected_pid", str(pid),
        "--es", "expected_relay", expected_relay)
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        raw = adb("shell", "run-as", PACKAGE, "cat", CACHE_FILE, check=False)
        try:
            snapshot = json.loads(raw)
        except json.JSONDecodeError:
            time.sleep(0.1)
            continue
        assert adb("shell", "pidof", PACKAGE).strip() == str(pid), "Receiver process changed during query"
        snapshot["host_relay_reachable"] = bool(host_relay_reachable)
        snapshot["classification"] = classify_health(snapshot, pid, host_relay_reachable)
        return snapshot
    raise AssertionError("One-shot receiver health query did not return")
