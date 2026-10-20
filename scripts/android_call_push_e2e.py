#!/usr/bin/env python3
"""Real push service -> cold Android app -> native call action -> FIPS media.

Use an English-language ephemeral Android user with the debug and androidTest
APKs installed for that user. This adds a fresh test contact and restores settings at cleanup.
Requires built iris-call-fixture and local_nostr_relay binaries, and adb.
No Firebase credentials or direct provider sends: use the app's existing service.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import socket
import sqlite3
import subprocess
import threading
import time
import xml.etree.ElementTree as ET

from run_harness import verify_instrumentation_target, verify_isolated_android_user


def wait_for(description, predicate, seconds=25):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.25)
    raise AssertionError(f"Timed out: {description}")


def call_history(account, call_id):
    with sqlite3.connect((account / "core.sqlite3").resolve().as_uri() + "?mode=ro", uri=True) as connection:
        row = connection.execute("SELECT call_json FROM messages WHERE id = ?", (f"call:{call_id}",)).fetchone()
    return json.loads(row[0]) if row and row[0] else None


def ended_with_outcome(status, history, call_id, outcome):
    call = status.get("call") or {}
    return (call.get("id") == call_id and call.get("phase") == "ended" and
            history is not None and history.get("call_id") == call_id and
            history.get("direction") == "outgoing" and history.get("outcome") == outcome)


def canceled_before_no_answer(status, history, call_id, command_at_ms):
    if not ended_with_outcome(status, history, call_id, "canceled"):
        return False
    started, ended = history.get("started_at_secs"), history.get("ended_at_secs")
    # No-answer also persists "canceled" after 30 seconds; it is not remote cancellation.
    return (isinstance(started, int) and isinstance(ended, int) and
            command_at_ms // 1000 <= ended and 0 <= ended - started < 30)


def matched_call_callback(records, call_id):
    for resolved in reversed(records):
        preview = resolved.get("preview_call") or {}
        if (resolved.get("stage") != "resolved" or resolved.get("call_id") != call_id or
                preview.get("call_id") != call_id or preview.get("phase") != "incoming"):
            continue
        raw = resolved.get("raw_payload_json", "")
        try:
            outer = json.loads(raw)["event"]
            outer = json.loads(outer) if isinstance(outer, str) else outer
        except (ValueError, KeyError, TypeError):
            continue
        digest = hashlib.sha256(raw.encode()).hexdigest()
        if (isinstance(outer, dict) and outer.get("kind") == 21111 and outer.get("id") and
                outer.get("id") == resolved.get("outer_event_id") and
                resolved.get("raw_payload_sha256") == digest and
                any(record.get("stage") == "fcm_received" and
                    record.get("outer_event_id") == outer.get("id") and
                    record.get("raw_payload_sha256") == digest and
                    record.get("raw_payload_json") == raw for record in records)):
            return resolved
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Explicit disposable Android test device")
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="Fresh private evidence directory")
    parser.add_argument("--push-server", default="https://notifications.iris.to")
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--package", default="to.iris.chat.debug", help="Installed target application ID")
    parser.add_argument("--user", required=True, help="Current non-primary ephemeral Android test user")
    parser.add_argument("--test-package", default="to.iris.chat.test", help="Installed instrumentation application ID")
    args = parser.parse_args()
    os.umask(0o077)
    package = args.package
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+", args.test_package):
        parser.error("Invalid Android instrumentation package.")
    runner = f"{args.test_package}/androidx.test.runner.AndroidJUnitRunner"
    verify_isolated_android_user(args.adb, args.serial, args.user)
    verify_instrumentation_target(args.adb, args.serial, runner, package)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    adb_base = [args.adb, "-s", args.serial]

    def adb(*parts, check=True):
        result = subprocess.run(adb_base + list(parts), capture_output=True, text=True, timeout=120)
        if check and result.returncode:
            (args.output / "adb-failure.log").write_text(result.stdout + result.stderr)
            # Do not print arguments: preparation includes a private invite.
            raise RuntimeError("Android command failed; see private adb-failure.log")
        return result.stdout

    def instrument(method, **extras):
        parts = ["shell", "am", "instrument", "-w", "-r", "--user", args.user, "-e", "class",
                 f"to.iris.chat.calls.CallPushHarnessTest#{method}"]
        for key, value in extras.items():
            parts += ["-e", key, value]
        output = adb(*parts, runner)
        (args.output / f"{method}.log").write_text(output)
        assert "OK (1 test)" in output, f"Instrumentation failed: {method}; see private log"
        return output

    def call_probe():
        # Read the original callback without starting instrumentation or the app.
        raw = adb("exec-out", "run-as", package, "--user", args.user, "cat",
                  "shared_prefs/iris_push_probe.xml", check=False)
        try:
            root = ET.fromstring(raw)
            text = next((node.text for node in root if node.get("name") == "call_events"), "[]")
            records = json.loads(text or "[]")
        except (ET.ParseError, ValueError):
            return []
        (args.output / "call-push-probe.json").write_text(json.dumps(records, indent=2) + "\n")
        return records

    def matching_probe(call_id, stage, detail=None):
        return next((record for record in reversed(call_probe()) if record.get("stage") == stage
                     and record.get("call_id") == call_id
                     and (detail is None or record.get("detail") == detail)), None)

    def notification(channel=None, call_id=None):
        dump = adb("shell", "dumpsys", "notification", "--noredact")
        shown = any("NotificationRecord(" in line and f"pkg={package} " in line
                   and f"user=UserHandle{{{args.user}}}" in line and "id=7401 " in line and (not channel or f"channel={channel} " in line)
                   for line in dump.splitlines())
        return shown and (call_id is None or matching_probe(call_id, "notification_shown", channel))

    def dismissed(call_id, after_ms=0):
        record = matching_probe(call_id, "notification_dismissed")
        return record and record.get("at_ms", 0) >= after_ms and not notification()

    def capture_live_state(label):
        def private_command(*parts):
            return subprocess.run(adb_base + ["exec-out", "run-as", package, "--user", args.user, *parts],
                                  capture_output=True, text=True, timeout=120)

        # AccountStorage.currentDirectory uses the parent as the migration marker;
        # an absent active file must never make us read the stale legacy profile.
        marker = private_command("test", "-e", "no_backup/profiles")
        runtime_directory = "no_backup/profiles/active" if marker.returncode == 0 else "files"
        marker_valid = marker.returncode == 0 or (marker.returncode == 1 and not (marker.stdout + marker.stderr).strip())
        receipt = {"profile_marker_exit_code": marker.returncode, "captures": []}
        captures = [("shared_prefs/iris_push_probe.xml", "push-probe.xml", "xml"),
                    ("files/call_push_subscription_probe.json", "subscription-response.json", "json")]
        if marker_valid:
            captures.append((f"{runtime_directory}/iris_chat_runtime_debug.json", "runtime-debug.json", "json"))
        else:
            receipt["captures"].append({"name": "runtime-debug.json", "status": "profile_marker_unreadable"})
            (args.output / f"{label}-profile-marker.capture-error.txt").write_text(marker.stdout + marker.stderr)
        for relative, suffix, format_name in captures:
            result = private_command("cat", relative)
            item = {"source": relative, "name": suffix, "exit_code": result.returncode}
            try:
                if result.returncode:
                    raise OSError("private_read_failed")
                if format_name == "json":
                    json.loads(result.stdout)
                else:
                    ET.fromstring(result.stdout)
            except (OSError, ValueError, ET.ParseError) as error:
                item["status"] = "missing_or_unreadable" if result.returncode else "invalid_format"
                item["error_type"] = type(error).__name__
                (args.output / f"{label}-{suffix}.capture-error.txt").write_text(result.stdout + result.stderr)
            else:
                item.update(status="captured", sha256=hashlib.sha256(result.stdout.encode()).hexdigest())
                (args.output / f"{label}-{suffix}").write_text(result.stdout)
            receipt["captures"].append(item)
        (args.output / f"{label}-capture-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        call_probe()

    def sample_device_clock():
        before_ms = int(time.time() * 1000)
        raw = adb("shell", "date", "+%s%3N").strip()
        after_ms = int(time.time() * 1000)
        assert raw.isdecimal() and len(raw) == 13, "Device millisecond epoch unavailable"
        device_ms = int(raw)
        return {"device_at_ms": device_ms, "host_before_ms": before_ms, "host_after_ms": after_ms,
                "device_minus_host_min_ms": device_ms - after_ms,
                "device_minus_host_max_ms": device_ms - before_ms}

    def package_pids():
        pids = []
        for line in adb("shell", "ps", "-A", "-o", "UID,PID,NAME").splitlines():
            fields = line.split()
            if len(fields) != 3:
                continue
            uid, pid, name = fields
            if (uid.isdecimal() and pid.isdecimal() and int(uid) // 100_000 == int(args.user)
                    and (name == package or name.startswith(package + ":"))):
                pids.append(pid)
        return pids

    assert adb("shell", "pm", "path", "--user", args.user, "com.google.android.gms", check=False).strip(), (
        "This FCM test requires Google Play services in the device's active profile")

    def native_action(action, call_id):
        assert notification("incoming-calls", call_id), "Expected exact incoming call before native action"
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
                    if "Decline" in action:
                        wait_for("exact decline action dispatched", lambda:
                            matching_probe(call_id, "notification_end_action", "runtime_id_matches") or
                            matching_probe(call_id, "telecom_end_action", "reject"))
                    else:
                        wait_for("exact native answer requested", lambda: matching_probe(call_id, "answer_requested"))
                    capture_live_state(f"action-{call_id}")
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

        def event(kind, matches=lambda _: True):
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                value = events.get(timeout=max(0.01, deadline-time.monotonic()))
                if value["event"] == kind and matches(value):
                    return value
            raise AssertionError(f"Fixture missing {kind}")

        def command(value):
            fixture.stdin.write(value + "\n")
            fixture.stdin.flush()

        def status():
            command("status")
            return event("status")

        def peer_ready(owner):
            command(f"peer {owner}")
            return event("peer")["ready"]

        ready = event("ready")
        prepared = True
        result = instrument("prepare_incoming_call_push", call_invite=ready["invite"],
            call_owner=ready["owner"], call_device=ready["device"],
            call_relay=f"ws://127.0.0.1:{port}", call_push_server=args.push_server)
        owner = re.search(r"INSTRUMENTATION_STATUS: owner=([a-f0-9]{64})", result).group(1)
        command(f"accept {owner}")
        event("accepted")
        wait_for("caller has a verified recipient session", lambda: peer_ready(owner))

        seen_calls = set()
        def cold_call(kind, doze=False):
            wait_for("previous notification dismissed", lambda: not notification())
            adb("shell", "cmd", "statusbar", "collapse")
            adb("shell", "input", "keyevent", "HOME")
            adb("shell", "am", "kill", "--user", args.user, package)
            # Telecom can briefly keep a finished call's process important.
            # Simulate an OS process kill without force-stop (which blocks FCM).
            pids = package_pids()
            if pids:
                assert all(pid.isdecimal() for pid in pids)
                adb("shell", "run-as", package, "--user", args.user, "kill", "-9", *pids)
            wait_for("app process stopped", lambda: not package_pids())
            adb("shell", "input", "keyevent", "SLEEP")
            if doze:
                adb("shell", "dumpsys", "battery", "unplug")
                adb("shell", "dumpsys", "deviceidle", "force-idle")
                assert adb("shell", "dumpsys", "deviceidle", "get", "deep").strip() == "IDLE"
            command(f"call {owner} {kind}")
            started = event("call", lambda value: value.get("phase") == "outgoing" and value.get("id") not in seen_calls)
            call_id = started["id"]
            seen_calls.add(call_id)
            assert started["phase"] == "outgoing", "Expected a new outgoing caller state"
            wait_for("exact authenticated original call callback", lambda: matched_call_callback(call_probe(), call_id))
            wait_for("exact native incoming call after push", lambda: notification("incoming-calls", call_id))
            capture_live_state(f"incoming-{call_id}")
            print(f"PASS matched cold {kind} callback and native notification{' in Doze' if doze else ''}", flush=True)
            return call_id

        for kind in ("voice", "video"):
            before = status()
            call_id = cold_call(kind)
            native_action(("Answer", "Video"), call_id)
            def media_received():
                now = status()
                return (now.get("call") or {}).get("id") == call_id and (now.get("call") or {}).get("phase") == "connected" and (
                    now["audio_frames"] > before["audio_frames"] + 20 and
                    (kind == "voice" or now["video_frames"] > before["video_frames"] + 10))
            wait_for("FIPS media after native answer", media_received)
            png = subprocess.check_output(adb_base + ["exec-out", "screencap", "-p"], timeout=10)
            (args.output / f"{kind}-connected.png").write_bytes(png)
            print(f"PASS exact native answer and Android-to-fixture {kind} media over FIPS", flush=True)
            command("end")
            wait_for("exact notification dismissed after remote end", lambda: dismissed(call_id))

        call_id = cold_call("voice", doze=True)
        native_action(("Decline",), call_id)
        wait_for("exact decline outcome reaches caller", lambda: ended_with_outcome(
            status(), call_history(args.output / "account", call_id), call_id, "declined"))
        wait_for("exact declined notification dismissed", lambda: dismissed(call_id))
        adb("shell", "dumpsys", "deviceidle", "unforce")
        adb("shell", "dumpsys", "battery", "reset")
        call_id = cold_call("voice")
        current = status().get("call") or {}
        assert current.get("id") == call_id and current.get("phase") in ("outgoing", "ringing"), (
            "Caller must still be awaiting an answer before the cancellation command")
        clock = sample_device_clock()
        assert notification("incoming-calls", call_id), "Exact notification must remain active before cancellation"
        cancel_at_ms = int(time.time() * 1000)
        (args.output / "caller-cancel-command.json").write_text(json.dumps(
            {"call_id": call_id, "sent_at_host_ms": cancel_at_ms, "before": current, "clock_sample": clock}) + "\n")
        command("end")
        wait_for("exact caller cancellation recorded before no-answer timeout", lambda: canceled_before_no_answer(
            status(), call_history(args.output / "account", call_id), call_id, cancel_at_ms), seconds=5)
        wait_for("exact ringing stops when caller cancels", lambda: dismissed(call_id, clock["device_at_ms"]))
        print("PASS exact decline outcome and actual caller cancellation dismiss native ringing", flush=True)
    finally:
        # Capture before instrumentation, preference restoration, or cleanup end commands.
        try:
            capture_live_state("before-cleanup")
            if fixture and fixture.poll() is None:
                (args.output / "caller-before-cleanup.json").write_text(json.dumps(status(), indent=2) + "\n")
        except Exception as error:
            (args.output / "capture-error.txt").write_text(type(error).__name__ + "\n")
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
