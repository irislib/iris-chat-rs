#!/usr/bin/env python3
"""Diagnostic-only fresh backgroundcontrol app; never replaces the saved-account gate.

Requires paired control APKs and newly built iris-call-fixture/local_nostr_relay.
No runtime permissions, account resets, logouts, or existing-app installations.
"""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import queue
import re
import socket
import subprocess
import sys
import threading
import time

from android_background_delivery_e2e import wait_for, write_marker
from android_background_health import CONTROL_PACKAGE, query_health
from android_fips_health import comparable_fips_interval, filter_fips_health, require_single_static_udp_peer
from native_lab import acquire, release


def private_ipv4(value):
    address = ipaddress.IPv4Address(value)
    assert any(address in ipaddress.IPv4Network(net) for net in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"))
    return str(address)


def isolated_host_environment(relay, bind, device_npub, peer):
    assert re.fullmatch(r"npub1[023456789acdefghjklmnpqrstuvwxyz]{58}", device_npub)
    environment = {**os.environ, "IRIS_DEMO_RELAYS": relay, "IRIS_CALL_ISOLATED_CONTROL": "1",
        "IRIS_FIPS_WEBSOCKET_SEED_URLS": "", "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR": "",
        "IRIS_CHAT_FIPS_ENABLE_WEBRTC": "0", "IRIS_CHAT_SAME_HOST_HASHTREE": "0",
        "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR": "", "IRIS_CHAT_FIPS_ROUTED_PEERS": "",
        "IRIS_CHAT_FIPS_STATIC_PEERS": f"{device_npub}=udp:{peer}", "IRIS_CHAT_FIPS_UDP_BIND_ADDR": bind,
        "IRIS_RUNTIME_DEBUG_SNAPSHOT": "0", "IRIS_UPDATE_RELAYS": relay, "IRIS_UPDATE_BLOSSOM_SERVERS": "",
        "IRIS_UPDATE_HTREE_REF": f"htree://{device_npub}/diagnostic-control/latest"}
    environment.pop("IRIS_CALL_DESKTOP_CODEC", None)
    return environment


def require_control_artifacts(aapt2, app_apk, test_apk):
    for apk, expected in [(app_apk, CONTROL_PACKAGE), (test_apk, CONTROL_PACKAGE + ".test")]:
        badging = subprocess.check_output([str(aapt2), "dump", "badging", str(apk)], text=True)
        match = re.search(r"^package: name='([^']+)'", badging, re.M)
        assert match is not None and match.group(1) == expected, "Artifact does not target the separate control package"


def stage_control_config(adb, config):
    assert not adb("shell", "pidof", CONTROL_PACKAGE, check=False).strip(), "Stop control app before changing startup config"
    # Fixed UID-private path; JSON is stdin, never shell code.
    adb("shell", "run-as", CONTROL_PACKAGE, "sh", "-c", "'cat > cache/background-control.json'",
        input=json.dumps(config))


def reject_unexpected_peers(health):
    fips = health["fips_transport"]
    if fips["valid"]:
        assert fips["configured_direct_peer_count"] == 1, "Control must configure exactly one static peer"
        assert fips["unexpected_connected_peer_count"] == 0, "Unexpected peer observed"
        assert fips["connected_peer_count"] <= 1, "Additional connection observed"
        assert set(fips["transports"]) <= {"udp"}, "Unexpected transport observed"


def control_foreground_ready(adb):
    focus = adb("shell", "dumpsys", "window", "windows")
    if any("mCurrentFocus=" in line and "permissioncontroller" in line for line in focus.splitlines()):
        adb("shell", "input", "keyevent", "BACK")  # Dismiss, never grant.
        return False
    activities = adb("shell", "dumpsys", "activity", "activities")
    ours = [block for block in re.split(r"\n\s*\* Hist", activities)
            if f"packageName={CONTROL_PACKAGE} " in block]
    visible = ("state=RESUMED", "mVisible=true", "mVisibleRequested=true", "mAppStopped=false")
    services = adb("shell", "dumpsys", "activity", "services", CONTROL_PACKAGE)
    return any(all(field in block for field in visible) for block in ours) and (
        "BackgroundMessageService" in services and "isForeground=true" in services)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--host", required=True, type=private_ipv4)
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--app-apk", required=True, type=Path)
    parser.add_argument("--test-apk", required=True, type=Path)
    parser.add_argument("--aapt2", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--external-idle-result", required=True, type=Path)
    args = parser.parse_args()
    package, test_package = CONTROL_PACKAGE, CONTROL_PACKAGE + ".test"
    # Fail before installation if either artifact could target an existing app.
    require_control_artifacts(args.aapt2, args.app_apk, args.test_apk)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    lock, _ = acquire(f"android:{args.serial}", 3600)
    assert lock is not None, "Phone reserved by another test"
    base = ["adb", "-s", args.serial]
    processes, logs = [], []
    installed = False
    reverse_port = None
    original_whitelist = False
    whitelist_changed = False
    initial_awake = False

    def adb(*parts, check=True, input=None):
        result = subprocess.run(base + list(parts), input=input, text=True, capture_output=True, timeout=180)
        if check and result.returncode:
            (args.output / "device-failure.log").write_text(result.stdout + result.stderr)
            raise RuntimeError("Control device operation failed; private evidence saved")
        return result.stdout

    def free_port(host, kind):
        with socket.socket(type=kind) as sock:
            sock.bind((host, 0))
            return sock.getsockname()[1]

    def instrument(classes, label, tests=1, **extras):
        command = ["shell", "am", "instrument", "-w", "-r", "-e", "class", classes]
        for key, value in extras.items():
            command += ["-e", key, str(value)]
        output = adb(*command, test_package + "/androidx.test.runner.AndroidJUnitRunner")
        (args.output / f"{label}.log").write_text(output)
        assert f"OK ({tests} test{'s' if tests != 1 else ''})" in output, "Control instrumentation failed"
        return output

    def stopped():
        blocks = re.split(r"\n\s*\* Hist", adb("shell", "dumpsys", "activity", "activities"))
        ours = [block for block in blocks if f"packageName={package} " in block]
        return bool(ours) and all("state=STOPPED" in block for block in ours)

    try:
        for name in (package, test_package):
            assert not adb("shell", "pm", "path", name, check=False).strip(), "Control requires a fresh separate package; no reset allowed"
        assert not adb("shell", "pm", "path", "com.google.android.gms", check=False).strip()
        initial_awake = "mWakefulness=Awake" in adb("shell", "dumpsys", "power")
        assert "UPDATES STOPPED" not in adb("shell", "dumpsys", "battery")
        addresses = adb("shell", "ip", "-o", "-4", "addr", "show", "wlan0")
        phone_ip = private_ipv4(re.search(r"inet ([0-9.]+)/", addresses).group(1))
        reverse_port = free_port("127.0.0.1", socket.SOCK_STREAM)
        relay_url = f"ws://127.0.0.1:{reverse_port}"
        relay_log = (args.output / "relay.log").open("w")
        logs.append(relay_log)
        relay = subprocess.Popen([str(args.bin_dir / "local_nostr_relay"), f"127.0.0.1:{reverse_port}"],
                                 stdout=relay_log, stderr=relay_log)
        processes.append(relay)

        def relay_ready():
            if relay.poll() is not None:
                return False
            try:
                with socket.create_connection(("127.0.0.1", reverse_port), timeout=0.2):
                    return True
            except OSError:
                return False

        wait_for("control message server", relay_ready)
        adb("reverse", f"tcp:{reverse_port}", f"tcp:{reverse_port}")
        adb("install", str(args.app_apk))
        installed = True
        adb("install", str(args.test_apk))
        stage_control_config(adb, {"phase": "bootstrap", "relay_url": relay_url})
        # Pure parser tests run before creating an account or opening an Activity.
        classes = ",".join("to.iris.chat.push." + name for name in (
            "BackgroundHealthSnapshotTest", "BackgroundFipsHealthSnapshotTest", "BackgroundControlStartupTest"))
        instrument(classes, "parser-tests", tests=10)
        adb("shell", "am", "force-stop", package)
        adb("shell", "input", "keyevent", "WAKEUP")
        prepared = instrument("to.iris.chat.push.BackgroundControlHarnessTest#bootstrap_fresh_identity",
                              "bootstrap", background_control="1")
        def field(name, pattern):
            return re.search(r"INSTRUMENTATION_STATUS: " + name + "=(" + pattern + ")", prepared).group(1)
        owner = field("owner", "[a-f0-9]{64}")
        device_npub = field("device_npub", "npub1[a-z0-9]+")
        udp_port = int(field("udp_port", "[0-9]+"))
        assert "INSTRUMENTATION_STATUS: mesh_absent=true" in prepared
        adb("shell", "am", "force-stop", package)

        host_port = free_port(args.host, socket.SOCK_DGRAM)
        environment = isolated_host_environment(relay_url, f"{args.host}:{host_port}", device_npub, f"{phone_ip}:{udp_port}")
        fixture_log = (args.output / "fixture.log").open("w")
        logs.append(fixture_log)
        fixture = subprocess.Popen([str(args.bin_dir / "iris-call-fixture"), str(args.output / "host-account")],
            env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=fixture_log, text=True, bufsize=1)
        processes.append(fixture)
        events = queue.Queue()
        def read_events():
            for line in fixture.stdout:
                events.put(json.loads(line))
            events.put({"event": "process-ended"})
        threading.Thread(target=read_events, daemon=True).start()
        def event(kind):
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                value = events.get(timeout=max(.01, deadline - time.monotonic()))
                assert value["event"] != "process-ended", "Control host stopped"
                if value["event"] == kind:
                    return value
            raise AssertionError("Control fixture response missing")
        def command(value):
            fixture.stdin.write(value + "\n")
            fixture.stdin.flush()
        ready = event("ready")
        receipt = args.output / "host-account" / "fixture-account-bundle.json"
        assert receipt.is_file() and receipt.stat().st_mode & 0o777 == 0o600, "Missing private host identity receipt"
        # The host exports its device npub directly; the owner key is not a FIPS peer identity.
        paired = {"phase": "paired", "relay_url": relay_url, "peer_npub": ready["device_npub"],
                  "peer_udp": f"{args.host}:{host_port}", "local_udp_port": udp_port}
        stage_control_config(adb, paired)
        command(f"accept {owner}")
        event("accepted")
        instrument("to.iris.chat.push.BackgroundControlHarnessTest#connect_control_peer", "pairing",
            background_control="1", fixture_account_owner=owner, fixture_relay=relay_url,
            fixture_owner=ready["owner"], fixture_invite=ready["invite"])
        whitelist = adb("shell", "dumpsys", "deviceidle", "whitelist")
        original_whitelist = any(package in line for line in whitelist.splitlines())
        adb("shell", "dumpsys", "deviceidle", "whitelist", f"+{package}")
        whitelist_changed = True
        launch = adb("shell", "am", "start", "-W", "-n", package + "/to.iris.chat.MainActivity")
        (args.output / "normal-launch.log").write_text(launch)
        assert "Status: ok" in launch, "Normal activity launch failed"
        # Wait for the actual Activity and service. Sending Home while a cold
        # launch is still pending can prevent onCreate/onStart entirely.
        wait_for("normal foreground activity and service", lambda: control_foreground_ready(adb))
        adb("shell", "input", "keyevent", "HOME")
        adb("shell", "input", "keyevent", "SLEEP")
        wait_for("control Activity stopped", stopped)
        pid = adb("shell", "pidof", package).strip()
        assert pid.isdecimal()
        def lifecycle():
            services = adb("shell", "dumpsys", "activity", "services", package)
            power = adb("shell", "dumpsys", "power")
            value = {"pid": int(adb("shell", "pidof", package).strip()), "activity_stopped": stopped(),
                     "foreground_service": "BackgroundMessageService" in services and "isForeground=true" in services,
                     "screen": re.search(r"mWakefulness=(\w+)", power).group(1)}
            assert value["pid"] == int(pid) and value["activity_stopped"] and value["foreground_service"]
            assert value["screen"] in ("Asleep", "Dozing")
            return value
        def health_pair():
            receiver = query_health(adb, pid, relay_url, relay_ready(), require_fips=True, package=package)
            command("health")
            host = event("health")
            host["fips_transport"] = filter_fips_health(host["fips_transport"])
            return receiver, host
        def isolated(pair):
            receiver, host = pair
            assert receiver["classification"] == "connected-idle"
            assert host["expected_relay_matches"] is True and host["connected_relay_count"] == 1
            assert host["pending_relay_publish_count"] == 0
            for value in pair:
                require_single_static_udp_peer(value["fips_transport"])
        deadline = time.monotonic() + 90
        while True:
            before = health_pair()
            for name, value in zip(("health-before", "host-health-before"), before):
                write_marker(args.output / (name + ".json"), value)
                reject_unexpected_peers(value)
            try:
                isolated(before)
                break
            except AssertionError:
                assert time.monotonic() < deadline, "Observed control isolation did not become ready"
                time.sleep(2)
        for name, value in zip(("health-before", "host-health-before"), before):
            write_marker(args.output / (name + ".json"), value)
        lifecycle()
        time.sleep(30)
        initial = lifecycle()
        write_marker(args.output / "profile-ready.json", {"pid": int(pid), "package": package,
            "settle_seconds": 30, "receiver_health_handshake": True, "diagnostic_only": True})
        print("Isolated control ready: normal background service, intended UDP peer, empty queue", flush=True)
        pair_file = args.output / "cpu-pair-complete.json"
        wait_for("control CPU pair", pair_file.exists, timeout=600)
        measured = json.loads(pair_file.read_text())
        assert measured["pid"] == int(pid) and measured["elapsed_seconds"] >= 120 and measured["background_verified"] is True
        after = health_pair()
        for name, value in zip(("health-after", "host-health-after"), after):
            write_marker(args.output / (name + ".json"), value)
        isolated(after)
        for old, new in zip(before, after):
            comparable_fips_interval(old["fips_transport"], new["fips_transport"])
        lifecycle()
        write_marker(args.output / "health-after-ready.json", {"pid": int(pid), "classification": after[0]["classification"]})
        wait_for("external control result", args.external_idle_result.exists, timeout=600)
        final = json.loads(args.external_idle_result.read_text())
        assert final["pid"] == int(pid) and final["cpu_percent_one_core"] == measured["cpu_percent_one_core"]
        write_marker(args.output / "result.json", {"diagnostic_only": True, "saved_account_gate_replaced": False,
            "host_identity_receipt_saved": True,
            "cpu_percent_one_core": final["cpu_percent_one_core"], "before": initial, "after": lifecycle(),
            "features": {"messages": True, "voice_calls": True, "video_calls": True,
                         "nearby_discovery": False, "runtime_permissions_granted_by_harness": False},
            "comparison_limitations": ["Fresh account and history", "One reciprocal static UDP peer",
                                       "Connected authenticated snapshots cannot exclude transient or unauthenticated traffic"],
            "fips_interval_scope": "health snapshots including settling; UID/CPU spans recorded separately"})
        print("Completed isolated diagnostic; saved-account CPU gate remains unchanged", flush=True)
    finally:
        active_error = sys.exc_info()[0] is not None
        cleanup_errors = []
        def clean(label, action):
            try:
                action()
            except Exception as error:
                cleanup_errors.append({"step": label, "error_type": type(error).__name__})
        if installed and active_error:
            for name, command_parts in [
                ("activities", ("activity", "activities")), ("windows", ("window", "windows")),
                ("power", ("power",)), ("services", ("activity", "services", package)),
            ]:
                clean("capture " + name, lambda: (args.output / ("failure-" + name + ".txt")).write_text(
                    adb("shell", "dumpsys", *command_parts, check=False)))
        if installed:
            clean("stop control", lambda: adb("shell", "am", "force-stop", package))
        if whitelist_changed and not original_whitelist:
            clean("restore battery exemption", lambda: adb("shell", "dumpsys", "deviceidle", "whitelist", f"-{package}"))
        for process in reversed(processes):
            def stop_child():
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=10)
            clean("stop owned host process", stop_child)
        if reverse_port is not None:
            clean("remove message server bridge", lambda: adb("reverse", "--remove", f"tcp:{reverse_port}"))
        if installed:
            clean("restore home", lambda: adb("shell", "input", "keyevent", "HOME"))
            clean("restore screen", lambda: adb("shell", "input", "keyevent", "WAKEUP" if initial_awake else "SLEEP"))
        for log in logs:
            clean("close evidence", log.close)
        clean("release phone reservation", lambda: release(lock))
        write_marker(args.output / "cleanup.json", {"errors": cleanup_errors})
        if cleanup_errors and not active_error:
            raise RuntimeError("Control cleanup incomplete; private evidence saved")


if __name__ == "__main__":
    main()
