#!/usr/bin/env python3
"""Verify Google-free, screen-off message/call alerts in a separate Android test app.

Requires backgroundtest APKs and local_nostr_relay / iris-call-fixture binaries.
Never replaces, logs into, or resets the installed production app. Evidence is private.
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

from native_lab import acquire, release


def wait_for(label, check, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.5)
    raise AssertionError(f"Timed out: {label}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--external-idle-result", type=Path,
                        help="Wait up to 10 minutes for a separate profiler's CPU percentages for this PID")
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    package = "to.iris.chat.backgroundtest"
    adb_base = ["adb", "-s", args.serial]
    lock, _ = acquire(f"android:{args.serial}", 6 * 60 * 60)
    if lock is None:
        raise RuntimeError("Pixel is reserved by another test")

    def adb(*parts, check=True):
        result = subprocess.run(adb_base + list(parts), capture_output=True, text=True, timeout=150)
        if check and result.returncode:
            (args.output / "adb-failure.log").write_text(result.stdout + result.stderr)
            raise RuntimeError("Android operation failed; inspect private evidence")
        return result.stdout

    def instrument(method, **extras):
        parts = ["shell", "am", "instrument", "-w", "-r", "-e", "class",
                 f"to.iris.chat.push.BackgroundDeliveryHarnessTest#{method}"]
        for key, value in extras.items():
            parts += ["-e", key, value]
        output = adb(*parts, "to.iris.chat.backgroundtest.test/androidx.test.runner.AndroidJUnitRunner")
        (args.output / f"{method}.log").write_text(output)
        assert "OK (1 test)" in output, f"Failed {method}; inspect private evidence"
        return output

    def notification(channel, body=None):
        dump = adb("shell", "dumpsys", "notification", "--noredact")
        for block in re.split(r"\n(?=\s*NotificationRecord\()", dump):
            header = block.splitlines()[0] if block else ""
            if f"pkg={package} " in header and f"channel={channel} " in header:
                if body is None or body in block:
                    return True
        return False

    relay = fixture = None
    port = None
    original_whitelist = False
    idle_forced = False
    initial_interactive = "mWakefulness=Awake" in adb("shell", "dumpsys", "power")
    try:
        assert not adb("shell", "pm", "path", "com.google.android.gms", check=False).strip(), (
            "This regression requires a Google-free Android device")
        whitelist = adb("shell", "dumpsys", "deviceidle", "whitelist")
        original_whitelist = any(package in line for line in whitelist.splitlines())
        # Equivalent to accepting this test app's Android background-receiving
        # prompt. Restore the original allowance at exit.
        adb("shell", "dumpsys", "deviceidle", "whitelist", f"+{package}")
        adb("shell", "input", "keyevent", "WAKEUP")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        relay_log = (args.output / "relay.log").open("w")
        relay = subprocess.Popen([str(args.bin_dir / "local_nostr_relay"), f"127.0.0.1:{port}"],
                                 stdout=relay_log, stderr=relay_log)

        def relay_ready():
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return True
            except OSError:
                return False
        wait_for("test message server", relay_ready)
        adb("reverse", f"tcp:{port}", f"tcp:{port}")
        fixture_log = (args.output / "fixture.log").open("w")
        fixture = subprocess.Popen([str(args.bin_dir / "iris-call-fixture"), str(args.output / "sender")],
            env={**os.environ, "IRIS_DEMO_RELAYS": f"ws://127.0.0.1:{port}"},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=fixture_log, text=True, bufsize=1)
        events = queue.Queue()

        def read_events():
            for line in fixture.stdout:
                events.put(json.loads(line))
        threading.Thread(target=read_events, daemon=True).start()

        def command(value):
            fixture.stdin.write(value + "\n")
            fixture.stdin.flush()

        def event(kind):
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                value = events.get(timeout=max(0.01, deadline - time.monotonic()))
                if value["event"] == kind:
                    return value
            raise AssertionError(f"Missing fixture event: {kind}")

        def status():
            command("status")
            return event("status")

        ready = event("ready")
        prepared = instrument("prepare_background_receiver", fixture_invite=ready["invite"],
            fixture_owner=ready["owner"], fixture_device=ready["device"],
            fixture_relay=f"ws://127.0.0.1:{port}")
        owner = re.search(r"INSTRUMENTATION_STATUS: owner=([a-f0-9]{64})", prepared).group(1)
        device = re.search(r"INSTRUMENTATION_STATUS: device=([a-f0-9]{64})", prepared).group(1)
        # Android finishes instrumentation by killing its target process. Launch
        # normally before testing user backgrounding, rather than mistaking that
        # test-runner teardown for an ordinary closed app.
        adb("shell", "am", "start", "-n", f"{package}/to.iris.chat.MainActivity")
        wait_for("background service after instrumentation", lambda: notification("background-receiving"))
        command(f"accept {owner}")
        event("accepted")
        wait_for("verified receiving device", lambda: device in status()["call_authors"])
        adb("shell", "input", "keyevent", "HOME")
        adb("shell", "input", "keyevent", "SLEEP")
        def activity_stopped():
            dump = adb("shell", "dumpsys", "activity", "activities")
            return any(f"packageName={package} " in block and "state=STOPPED" in block
                       for block in re.split(r"\n\s*\* Hist", dump))
        wait_for("activity stopped after closing animation", activity_stopped)
        wait_for("background service", lambda: notification("background-receiving"))
        # Screen off, with the most recent chat still selected in the router.
        message = f"screen-off-message-{time.time_ns()}"
        command(f"message {owner} {message}")
        event("message-sent")
        wait_for("screen-off message alert", lambda: notification("iris_chat_message_alerts", message))
        print("PASS screen-off message alert without Google services", flush=True)
        for kind in ("voice", "video"):
            command(f"call {owner} {kind}")
            wait_for(f"screen-off {kind} ringing", lambda: notification("incoming-calls"))
            command("end")
            wait_for("caller cancellation dismisses ringing", lambda: not notification("incoming-calls"))
            print(f"PASS screen-off {kind} ringing and cancellation", flush=True)

        assert "UPDATES STOPPED" not in adb("shell", "dumpsys", "battery"), "Battery is already simulated"
        idle_forced = True
        adb("shell", "dumpsys", "battery", "unplug")
        adb("shell", "dumpsys", "deviceidle", "force-idle")
        assert adb("shell", "dumpsys", "deviceidle", "get", "deep").strip() == "IDLE"
        command(f"call {owner} video")
        wait_for("video ringing in Doze", lambda: notification("incoming-calls"))
        command("end")
        wait_for("Doze call cancellation", lambda: not notification("incoming-calls"))
        print("PASS video ringing in Doze with background allowance", flush=True)
        adb("shell", "dumpsys", "deviceidle", "unforce")
        adb("shell", "dumpsys", "battery", "reset")
        idle_forced = False

        adb("shell", "input", "keyevent", "WAKEUP")
        adb("shell", "cmd", "statusbar", "expand-notifications")
        (args.output / "notifications.png").write_bytes(subprocess.check_output(
            adb_base + ["exec-out", "screencap", "-p"], timeout=15))
        adb("shell", "cmd", "statusbar", "collapse")
        adb("shell", "input", "keyevent", "SLEEP")

        # Observe the real app process; do not reset device-wide battery history.
        pid = adb("shell", "pidof", package).strip()
        assert pid.isdecimal()
        if args.external_idle_result:
            assert not args.external_idle_result.exists(), "Refusing a stale profiling result"
            (args.output / "profile-ready.json").write_text(json.dumps({"pid": int(pid), "package": package}))
            print("Functional checks passed; quiet receiver ready for separate CPU profiling", flush=True)
            wait_for("external idle profile", args.external_idle_result.exists, timeout=600)
            result = json.loads(args.external_idle_result.read_text())
            assert result["pid"] == int(pid) and result["elapsed_seconds"] >= 60
            percentages = result["cpu_percent_one_core"]
            assert isinstance(percentages, list) and percentages
        else:
            def ticks():
                values = adb("shell", "run-as", package, "cat", f"/proc/{pid}/stat").rsplit(")", 1)[1].split()
                return int(values[11]) + int(values[12])
            hz = int(adb("shell", "getconf", "CLK_TCK").strip())
            time.sleep(30)  # Same settling period as the native idle gate.
            before, started = ticks(), time.monotonic()
            time.sleep(60)
            percentages = [(ticks() - before) / hz / (time.monotonic() - started) * 100]
            (args.output / "idle.json").write_text(json.dumps({"cpu_percent_one_core": percentages}))
        assert all(0 <= value < 5 for value in percentages), "Background receiver exceeded the native idle CPU gate"
        print("PASS settled background CPU budget", flush=True)
        adb("shell", "input", "keyevent", "WAKEUP")
        instrument("stop_when_alerts_disabled", background_harness="1")
        print("PASS receiver stops when message and call alerts are disabled", flush=True)
    finally:
        if idle_forced:
            adb("shell", "dumpsys", "deviceidle", "unforce", check=False)
            adb("shell", "dumpsys", "battery", "reset", check=False)
        adb("shell", "am", "force-stop", package, check=False)
        if not original_whitelist:
            adb("shell", "dumpsys", "deviceidle", "whitelist", f"-{package}", check=False)
        if fixture:
            fixture.terminate()
            fixture.wait(timeout=10)
        if relay:
            relay.terminate()
            relay.wait(timeout=10)
        if port:
            adb("reverse", "--remove", f"tcp:{port}", check=False)
        adb("shell", "input", "keyevent", "HOME", check=False)
        if not initial_interactive:
            adb("shell", "input", "keyevent", "SLEEP", check=False)
        release(lock)


if __name__ == "__main__":
    main()
