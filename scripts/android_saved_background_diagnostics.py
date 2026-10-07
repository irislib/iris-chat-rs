#!/usr/bin/env python3
"""One saved-account idle attribution run; no new contact, history reset or permission grant."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import socket
import subprocess
import sys
import time

from android_background_delivery_e2e import wait_for, write_marker
from android_background_health import query_health
from android_fips_health import comparable_fips_interval
from android_saved_background_state import ALERTS, PACKAGE, installed_hash, read_saved_state, require_preserved_state
from native_lab import acquire, release


def signing_digests(output):
    values = set(re.findall(r"^(?:V[0-9]+ )?Signer[^\n]*certificate SHA-256 digest: ([a-fA-F0-9]{64})$", output, re.M))
    assert len(values) == 1, "Expected one verified APK signing identity"
    return next(iter(values)).lower()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("serial", "app-apk", "previous-apk", "test-apk", "aapt2", "relay-bin", "account-receipt", "output"):
        parser.add_argument("--" + name, required=True, type=str if name == "serial" else Path)
    args = parser.parse_args()
    for artifact, package in ((args.app_apk, PACKAGE), (args.previous_apk, PACKAGE), (args.test_apk, PACKAGE + ".test")):
        badging = subprocess.check_output([str(args.aapt2), "dump", "badging", str(artifact)], text=True)
        assert re.search(r"^package: name='([^']+)'", badging, re.M)[1] == package
    def signer(artifact):
        value = subprocess.check_output([str(args.aapt2.parent / "apksigner"), "verify", "--print-certs", str(artifact)], text=True)
        return signing_digests(value)
    assert signer(args.app_apk) == signer(args.previous_apk), "Preserving account data requires the same signing identity"
    receipt = json.loads(args.account_receipt.read_text())
    assert receipt["package"] == PACKAGE
    assert all(receipt["preferences"][key] == 0 for key in ALERTS)
    relays = json.loads(receipt["preferences"]["nostr_relay_urls_json"])
    assert len(relays) == 1
    match = re.fullmatch(r"ws://127\.0\.0\.1:([0-9]+)", relays[0])
    assert match and 0 < int(match[1]) <= 65535
    port = int(match[1]); relay_url = relays[0]
    args.output.mkdir(parents=True, exist_ok=False, mode=0o700)
    lock, _ = acquire("android:" + args.serial, 3600)
    assert lock is not None, "Phone reserved by another test"
    base = ["adb", "-s", args.serial]
    before = None; relay = None; relay_log = None
    bridge = False; exemption = False; initial_exempt = False; initial_awake = None
    features_touched = False; app_touched = False

    def adb(*parts, check=True):
        result = subprocess.run(base + list(parts), capture_output=True, text=True, timeout=180)
        if check and result.returncode:
            (args.output / "device-error.log").write_text(result.stdout + result.stderr)
            raise RuntimeError("Device operation failed; private evidence saved")
        return result.stdout
    def binary(*parts):
        return subprocess.check_output(base + list(parts), timeout=30)
    def instrument(method):
        value = adb("shell", "am", "instrument", "-w", "-r", "-e", "class",
                    "to.iris.chat.push.BackgroundDeliveryHarnessTest#" + method,
                    "-e", "background_harness", "1", "-e", "fixture_account_owner", receipt["owner"],
                    PACKAGE + ".test/androidx.test.runner.AndroidJUnitRunner")
        (args.output / (method + ".log")).write_text(value)
        assert "OK (1 test)" in value, "Saved receiver preparation failed"
    def activity_matches(fields):
        dump = adb("shell", "dumpsys", "activity", "activities")
        blocks = [b for b in re.split(r"\n\s*\* Hist", dump) if f"packageName={PACKAGE} " in b]
        matches = [all(field in block for field in fields) for block in blocks]
        return bool(matches) and (any(matches) if "state=RESUMED" in fields else all(matches))
    def service_active():
        value = adb("shell", "dumpsys", "activity", "services", PACKAGE)
        return "BackgroundMessageService" in value and "isForeground=true" in value
    def relay_ready():
        if relay is None or relay.poll() is not None: return False
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=.2): return True
        except OSError: return False

    try:
        initial_awake = "mWakefulness=Awake" in adb("shell", "dumpsys", "power")
        assert "UPDATES STOPPED" not in adb("shell", "dumpsys", "battery")
        assert not adb("shell", "pm", "path", "com.google.android.gms", check=False).strip()
        for package, artifact in ((PACKAGE, args.previous_apk), (PACKAGE + ".test", args.test_apk)):
            digest = installed_hash(adb, package)
            assert digest == receipt["installed_artifacts"][package] == hashlib.sha256(artifact.read_bytes()).hexdigest()
        before = read_saved_state(adb, binary)
        assert before["owner"] == receipt["owner"] and receipt["device"] in before["devices"]
        assert before["relays"] == relays and before["alerts"] == {key: 0 for key in ALERTS}
        assert before["history_counts"] == receipt["history_counts"]
        assert before["account_store_sha256"] == receipt["account_store_sha256"]
        write_marker(args.output / "state-before.json", before)
        relay_log = (args.output / "relay.log").open("w")
        relay = subprocess.Popen([str(args.relay_bin), f"127.0.0.1:{port}"], stdout=relay_log, stderr=relay_log)
        wait_for("preserved local message server", relay_ready)
        assert all(f"tcp:{port}" not in line.split()[1:2] for line in adb("reverse", "--list").splitlines())
        adb("reverse", f"tcp:{port}", f"tcp:{port}"); bridge = True
        initial_exempt = any(line.split(",")[1:2] == [PACKAGE]
                             for line in adb("shell", "dumpsys", "deviceidle", "whitelist").splitlines())
        adb("shell", "dumpsys", "deviceidle", "whitelist", "+" + PACKAGE); exemption = True
        app_touched = True
        (args.output / "update.log").write_text(adb("install", "-r", str(args.app_apk)))
        assert installed_hash(adb, PACKAGE) == hashlib.sha256(args.app_apk.read_bytes()).hexdigest()
        adb("shell", "am", "force-stop", PACKAGE)
        after_update = read_saved_state(adb, binary)
        write_marker(args.output / "state-after-update.json", after_update)
        require_preserved_state(before, after_update, before["alerts"])
        assert after_update["account_store_sha256"] == before["account_store_sha256"], "App update changed encrypted account storage"
        assert after_update["history_counts"] == before["history_counts"], "App update changed saved history counts"
        adb("shell", "input", "keyevent", "WAKEUP")
        features_touched = True
        instrument("resume_saved_background_receiver")
        launch = adb("shell", "am", "start", "-W", "-n", PACKAGE + "/to.iris.chat.MainActivity")
        (args.output / "normal-launch.log").write_text(launch)
        assert "Status: ok" in launch
        wait_for("normal foreground receiver", lambda: activity_matches(
            ("state=RESUMED", "mVisible=true", "mVisibleRequested=true")) and service_active())
        adb("shell", "input", "keyevent", "HOME"); adb("shell", "input", "keyevent", "SLEEP")
        hidden = ("state=STOPPED", "mAppStopped=true", "mVisible=false", "mVisibleRequested=false")
        wait_for("receiver stopped and hidden", lambda: activity_matches(hidden) and service_active())
        pid = adb("shell", "pidof", PACKAGE).strip(); assert pid.isdecimal()
        def lifecycle():
            power = adb("shell", "dumpsys", "power")
            screen = re.search(r"mWakefulness=(\w+)", power)[1]
            assert adb("shell", "pidof", PACKAGE).strip() == pid and activity_matches(hidden) and service_active()
            assert screen in ("Asleep", "Dozing")
            return {"pid": int(pid), "stopped_hidden": True, "foreground_service": True, "screen": screen}
        def health():
            value = query_health(adb, pid, relay_url, relay_ready(), require_fips=True)
            assert value["fips_transport"]["valid"] is True, "Authenticated transport aggregates unavailable"
            return value
        deadline = time.monotonic() + 90
        while True:
            pre = health(); write_marker(args.output / "health-before.json", pre)
            if pre["classification"] == "connected-idle": break
            assert time.monotonic() < deadline, "Saved receiver did not reach healthy idle"
            time.sleep(2)
        lifecycle(); time.sleep(30); initial = lifecycle()
        write_marker(args.output / "profile-ready.json", {"pid": int(pid), "package": PACKAGE,
            "settle_seconds": 30, "receiver_health_handshake": True, "diagnostic_only": True})
        print("Saved receiver ready: same account, live expected server, empty queue, normal background service", flush=True)
        pair_path = args.output / "cpu-pair-complete.json"
        wait_for("saved CPU pair", pair_path.exists, timeout=600)
        pair = json.loads(pair_path.read_text())
        assert pair["pid"] == int(pid) and pair["elapsed_seconds"] >= 120 and pair["background_verified"] is True
        post = health(); write_marker(args.output / "health-after.json", post)
        assert post["classification"] == "connected-idle"
        interval = post["fips_transport"]["interval"]
        if interval["valid"]: comparable_fips_interval(pre["fips_transport"], post["fips_transport"])
        lifecycle(); write_marker(args.output / "health-after-ready.json", {"pid": int(pid), "classification": post["classification"]})
        external = args.output / "external-idle.json"
        wait_for("saved external result", external.exists, timeout=600)
        result = json.loads(external.read_text())
        assert result["pid"] == int(pid) and result["cpu_percent_one_core"] == pair["cpu_percent_one_core"]
        write_marker(args.output / "result.json", {"diagnostic_only": True, "release_gate_replaced": False,
            "cpu_percent_one_core": result["cpu_percent_one_core"], "before": initial, "after": lifecycle(),
            "native_interval_valid": interval["valid"], "native_interval_reason": interval["reason"],
            "workload_differences": ["Original live counterpart absent", "Idle-only resume without preceding call sequence"],
            "alert_preferences_during_sample": {key: 1 for key in ALERTS},
            "fips_interval_scope": "Health snapshots include settling; UID and CPU spans are separately labelled"})
        print("Saved-account attribution completed; prior release gate is retained", flush=True)
    finally:
        active_error = sys.exc_info()[0] is not None; errors = []
        def clean(label, action):
            try: action()
            except Exception as error: errors.append({"step": label, "error_type": type(error).__name__})
        if app_touched and active_error:
            for name, parts in (("activities", ("activity", "activities")), ("services", ("activity", "services", PACKAGE)),
                                ("windows", ("window",)), ("power", ("power",))):
                clean("capture " + name, lambda: (args.output / ("failure-" + name + ".txt")).write_text(adb("shell", "dumpsys", *parts)))
        if features_touched:
            clean("wake for preference restoration", lambda: adb("shell", "input", "keyevent", "WAKEUP"))
            clean("restore initial alert preferences", lambda: instrument("stop_when_alerts_disabled"))
        if app_touched:
            clean("stop saved receiver", lambda: adb("shell", "am", "force-stop", PACKAGE))
            def verify_restored():
                state = read_saved_state(adb, binary); write_marker(args.output / "state-after-cleanup.json", state)
                require_preserved_state(before, state, before["alerts"])
                result_file = args.output / "result.json"
                if result_file.exists():
                    final = json.loads(result_file.read_text())
                    final.update(history_counts_before=before["history_counts"], history_counts_after=state["history_counts"],
                                 history_counts_unchanged=before["history_counts"] == state["history_counts"],
                                 initial_alert_preferences_restored=True)
                    write_marker(result_file, final)
            clean("verify account settings and permissions restored", verify_restored)
        if exemption and not initial_exempt:
            clean("restore battery exemption", lambda: adb("shell", "dumpsys", "deviceidle", "whitelist", "-" + PACKAGE))
        if bridge: clean("remove owned message bridge", lambda: adb("reverse", "--remove", f"tcp:{port}"))
        if relay is not None:
            def stop_relay():
                if relay.poll() is None:
                    relay.terminate()
                    try: relay.wait(timeout=10)
                    except subprocess.TimeoutExpired: relay.kill(); relay.wait(timeout=10)
            clean("stop owned message server", stop_relay)
        if app_touched:
            clean("restore home", lambda: adb("shell", "input", "keyevent", "HOME"))
            clean("restore screen", lambda: adb("shell", "input", "keyevent", "WAKEUP" if initial_awake else "SLEEP"))
        if relay_log is not None: clean("close server log", relay_log.close)
        clean("release phone reservation", lambda: release(lock))
        write_marker(args.output / "cleanup.json", {"errors": errors})
        if errors and not active_error: raise RuntimeError("Saved receiver cleanup incomplete; private evidence saved")


if __name__ == "__main__": main()
