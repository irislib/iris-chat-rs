"""One-shot aggregate health checks for the isolated background receiver harness."""
import json
import time
from android_fips_health import filter_fips_health

PACKAGE = "to.iris.chat.backgroundtest"
CONTROL_PACKAGE = "to.iris.chat.backgroundcontrol"
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


def query_health(adb, pid, expected_relay, host_relay_reachable, *, require_fips=False, package=PACKAGE):
    assert package in (PACKAGE, CONTROL_PACKAGE), "Health query requires an exact isolated test package"
    assert adb("shell", "pidof", package).strip() == str(pid), "Receiver process changed before query"
    # Resolve this as shell: app UIDs cannot resolve the special USER_CURRENT (-2).
    user = adb("shell", "am", "get-current-user").strip()
    assert user.isdecimal(), "Missing explicit Android user"
    adb("shell", "run-as", package, "rm", "-f", CACHE_FILE)
    # The receiver is not exported. Sending as its own UID retains that protection.
    adb("shell", "run-as", package, "am", "broadcast", "--user", user,
        "-n", package + "/to.iris.chat.debug.BackgroundHealthReceiver",
        "-a", "to.iris.chat.BACKGROUND_HEALTH", "--ei", "expected_pid", str(pid),
        "--es", "expected_relay", expected_relay, "--ez", "require_fips", str(require_fips).lower())
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        raw = adb("shell", "run-as", package, "cat", CACHE_FILE, check=False)
        try:
            snapshot = json.loads(raw)
        except json.JSONDecodeError:
            time.sleep(0.1)
            continue
        assert adb("shell", "pidof", package).strip() == str(pid), "Receiver process changed during query"
        snapshot["host_relay_reachable"] = bool(host_relay_reachable)
        snapshot["classification"] = classify_health(snapshot, pid, host_relay_reachable)
        if "fips_transport" in snapshot:
            snapshot["fips_transport"] = filter_fips_health(snapshot["fips_transport"])
        else:
            assert not require_fips, "Required FIPS diagnostics missing"
        return snapshot
    raise AssertionError("One-shot receiver health query did not return")
