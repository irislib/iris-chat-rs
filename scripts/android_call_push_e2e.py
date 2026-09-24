#!/usr/bin/env python3
"""Real push service -> cold Android app -> native call action -> FIPS media.

Use an English-language disposable test device with the debug and androidTest
APKs installed. This adds a fresh test contact and restores settings at cleanup.
Requires built iris-call-fixture and local_nostr_relay binaries, and adb.
No Firebase credentials or direct provider sends: use the app's existing service.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import socket
import subprocess
import threading
import time
import xml.etree.ElementTree as ET


def wait_for(description, predicate, seconds=25):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.25)
    raise AssertionError(f"Timed out: {description}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Explicit disposable Android test device")
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="Fresh private evidence directory")
    parser.add_argument("--push-server", default="https://notifications.iris.to")
    parser.add_argument("--adb", default="adb")
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    package = "to.iris.chat.debug"
    adb_base = [args.adb, "-s", args.serial]

    def adb(*parts, check=True):
        result = subprocess.run(adb_base + list(parts), capture_output=True, text=True, timeout=120)
        if check and result.returncode:
            (args.output / "adb-failure.log").write_text(result.stdout + result.stderr)
            # Do not print arguments: preparation includes a private invite.
            raise RuntimeError("Android command failed; see private adb-failure.log")
        return result.stdout

    def instrument(method, **extras):
        parts = ["shell", "am", "instrument", "-w", "-r", "-e", "class",
                 f"to.iris.chat.calls.CallPushHarnessTest#{method}"]
        for key, value in extras.items():
            parts += ["-e", key, value]
        output = adb(*parts, "to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner")
        (args.output / f"{method}.log").write_text(output)
        assert "OK (1 test)" in output, f"Instrumentation failed: {method}; see private log"
        return output

    def notification(channel=None):
        dump = adb("shell", "dumpsys", "notification", "--noredact")
        return any("NotificationRecord(" in line and f"pkg={package} " in line
                   and "id=7401 " in line and (not channel or f"channel={channel} " in line)
                   for line in dump.splitlines())

    assert adb("shell", "pm", "path", "com.google.android.gms", check=False).strip(), (
        "This FCM test requires Google Play services in the device's active profile")

    def native_action(action):
        adb("shell", "input", "keyevent", "WAKEUP")
        adb("shell", "input", "keyevent", "82")
        adb("shell", "cmd", "statusbar", "expand-notifications")
        for _ in range(3):
            adb("shell", "uiautomator", "dump", "/sdcard/iris-call-push-test.xml")
            tree = ET.fromstring(adb("shell", "cat", "/sdcard/iris-call-push-test.xml"))
            for node in tree.iter("node"):
                if node.get("content-desc") in action:
                    x1, y1, x2, y2 = map(int, re.findall(r"\d+", node.get("bounds")))
                    adb("shell", "input", "tap", str((x1+x2)//2), str((y1+y2)//2))
                    return
            time.sleep(0.5)
        raise AssertionError(f"Native action not found: {action}")

    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    relay_log = (args.output / "relay.log").open("w")
    fixture_log = (args.output / "fixture.log").open("w")
    relay = subprocess.Popen([str(args.bin_dir / "local_nostr_relay"), f"127.0.0.1:{port}"],
                             stdout=relay_log, stderr=relay_log)
    fixture = None
    prepared = False
    events = queue.Queue()
    try:
        def relay_ready():
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return True
            except OSError:
                return False
        wait_for("local message relay", relay_ready)
        adb("reverse", f"tcp:{port}", f"tcp:{port}")
        fixture = subprocess.Popen([str(args.bin_dir / "iris-call-fixture"), str(args.output / "account")],
            env={**os.environ, "IRIS_DEMO_RELAYS": f"ws://127.0.0.1:{port}",
                 "IRIS_CALL_PUSH_SERVER_URL": args.push_server},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=fixture_log, text=True, bufsize=1)

        def read_events():
            for line in fixture.stdout:
                value = json.loads(line)
                events.put(value)
                # The initial invite is a secret, so omit it from evidence.
                fixture_log.write(json.dumps({k: v for k, v in value.items() if k != "invite"}) + "\n")
                fixture_log.flush()
        threading.Thread(target=read_events, daemon=True).start()

        def event(kind):
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                value = events.get(timeout=max(0.01, deadline-time.monotonic()))
                if value["event"] == kind:
                    return value
            raise AssertionError(f"Fixture missing {kind}")

        def command(value):
            fixture.stdin.write(value + "\n")
            fixture.stdin.flush()

        def status():
            command("status")
            return event("status")

        ready = event("ready")
        prepared = True
        result = instrument("prepare_incoming_call_push", call_invite=ready["invite"],
            call_owner=ready["owner"], call_device=ready["device"],
            call_relay=f"ws://127.0.0.1:{port}", call_push_server=args.push_server)
        owner = re.search(r"INSTRUMENTATION_STATUS: owner=([a-f0-9]{64})", result).group(1)
        device = re.search(r"INSTRUMENTATION_STATUS: device=([a-f0-9]{64})", result).group(1)
        command(f"accept {owner}")
        event("accepted")
        wait_for("caller knows the recipient's verified device", lambda: device in status()["call_authors"])

        def cold_call(kind, doze=False):
            wait_for("previous notification dismissed", lambda: not notification())
            adb("shell", "cmd", "statusbar", "collapse")
            adb("shell", "input", "keyevent", "HOME")
            adb("shell", "am", "kill", package)
            # Telecom can briefly keep a finished call's process important.
            # Simulate an OS process kill without force-stop (which blocks FCM).
            pids = adb("shell", "pidof", package, check=False).split()
            if pids:
                assert all(pid.isdecimal() for pid in pids)
                adb("shell", "run-as", package, "kill", "-9", *pids)
            wait_for("app process stopped", lambda: not adb("shell", "pidof", package, check=False).strip())
            adb("shell", "input", "keyevent", "SLEEP")
            if doze:
                adb("shell", "dumpsys", "battery", "unplug")
                adb("shell", "dumpsys", "deviceidle", "force-idle")
                assert adb("shell", "dumpsys", "deviceidle", "get", "deep").strip() == "IDLE"
            command(f"call {owner} {kind}")
            wait_for("native incoming call after push", lambda: notification("incoming-calls"))
            print(f"PASS cold {kind} push{' in Doze' if doze else ''}", flush=True)

        for kind in ("voice", "video"):
            before = status()
            cold_call(kind)
            native_action(("Answer", "Video"))
            def media_received():
                now = status()
                return (now.get("call") or {}).get("phase") == "connected" and (
                    now["audio_frames"] > before["audio_frames"] + 20 and
                    (kind == "voice" or now["video_frames"] > before["video_frames"] + 10))
            wait_for("FIPS media after native answer", media_received)
            png = subprocess.check_output(adb_base + ["exec-out", "screencap", "-p"], timeout=10)
            (args.output / f"{kind}-connected.png").write_bytes(png)
            print(f"PASS native answer and {kind} media over FIPS", flush=True)
            command("end")
            wait_for("notification dismissed after remote end", lambda: not notification())

        cold_call("voice", doze=True)
        native_action(("Decline",))
        wait_for("decline reaches caller", lambda: (status().get("call") or {}).get("phase") == "ended")
        wait_for("declined notification dismissed", lambda: not notification())
        adb("shell", "dumpsys", "deviceidle", "unforce")
        adb("shell", "dumpsys", "battery", "reset")
        cold_call("voice")
        command("end")
        wait_for("ringing stops when caller cancels", lambda: not notification())
        print("PASS decline and caller cancellation dismiss native ringing", flush=True)
    finally:
        adb("shell", "dumpsys", "deviceidle", "unforce", check=False)
        adb("shell", "dumpsys", "battery", "reset", check=False)
        adb("shell", "input", "keyevent", "WAKEUP", check=False)
        adb("shell", "input", "keyevent", "82", check=False)
        if fixture:
            if fixture.poll() is None:
                fixture.stdin.write("end\nstop\n"); fixture.stdin.flush()
                try:
                    fixture.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    fixture.terminate(); fixture.wait(timeout=5)
        try:
            if prepared:
                instrument("restore_after_call_push")
        finally:
            adb("reverse", "--remove", f"tcp:{port}", check=False)
            relay.terminate(); relay.wait(timeout=5)
            relay_log.close(); fixture_log.close()


if __name__ == "__main__":
    main()
