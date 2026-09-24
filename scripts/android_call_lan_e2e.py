#!/usr/bin/env python3
"""Physical Android camera/audio -> Wi-Fi -> FIPS echo -> native decode.

Requires a private IPv4 address on this host and a phone on the same LAN.
Uses NativeCallTestRunner and a fresh account; never opens the stored account.
Temporarily installs paired debug/test APKs and restores both original APKs.
Run inside a native_lab.py Android reservation. No forwarding or network changes.
"""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import queue
import re
import shlex
import socket
import subprocess
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--host", required=True, help="This laptop's private LAN IPv4 address")
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--app-apk", required=True, type=Path)
    parser.add_argument("--test-apk", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--voice-only", action="store_true")
    parser.add_argument("--adb", default="adb")
    args = parser.parse_args()
    address = ipaddress.IPv4Address(args.host)
    assert address.is_private and not address.is_loopback and not address.is_unspecified
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    adb_base = [args.adb, "-s", args.serial]

    def adb(*parts, check=True):
        result = subprocess.run(adb_base + list(parts), capture_output=True, text=True, timeout=120)
        if check and result.returncode:
            (args.output / "adb-failure.log").write_text(result.stdout + result.stderr)
            raise RuntimeError("Android command failed; see private adb-failure.log")
        return result.stdout

    route = adb("shell", "ip", "route", "get", args.host)
    assert re.search(r"\bdev wlan\d+\b", route), "Phone must reach laptop over Wi-Fi"
    packages = [("to.iris.chat.debug", args.app_apk), ("to.iris.chat.test", args.test_apk)]
    backups = []
    for package, apk in packages:
        assert apk.is_file(), "Build both paired APKs before running"
        paths = adb("shell", "pm", "path", package).strip().splitlines()
        assert len(paths) == 1 and paths[0].startswith("package:"), "Requires installed monolithic debug/test APKs"
        backup = args.output / (package + ".apk")
        adb("pull", paths[0][8:], str(backup))
        backups.append((package, apk, backup))

    def port():
        with socket.socket() as sock:
            sock.bind((args.host, 0))
            return sock.getsockname()[1]

    relay_port, fips_port = port(), port()
    while fips_port == relay_port:
        fips_port = port()
    relay_url = f"ws://{args.host}:{relay_port}"
    events = queue.Queue()
    processes = []
    changed = []
    logs = []
    readers = []

    def spawn_log(name, command, **kwargs):
        log = (args.output / f"{name}.log").open("w")
        logs.append(log)
        process = subprocess.Popen(command, stderr=log, **kwargs)
        processes.append(process)
        return process, log

    def lines(process, name, log):
        for line in process.stdout:
            log.write(line)
            log.flush()
            events.put((name, line.strip()))
        events.put((name, None))

    def read_lines(process, name, log):
        reader = threading.Thread(target=lines, args=(process, name, log), daemon=True)
        readers.append(reader)
        reader.start()

    def listening(number):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                with socket.create_connection((args.host, number), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.1)
        raise AssertionError("Local test listener did not start")

    try:
        relay, _ = spawn_log("relay", [str(args.bin_dir / "local_nostr_relay"), f"{args.host}:{relay_port}"], stdout=subprocess.DEVNULL)
        listening(relay_port)
        environment = {**os.environ, "IRIS_DEMO_RELAYS": relay_url,
            "IRIS_FIPS_WEBSOCKET_SEED_URLS": "", "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR": f"{args.host}:{fips_port}",
            "IRIS_CHAT_FIPS_UDP_BIND_ADDR": f"{args.host}:0", "IRIS_CHAT_SAME_HOST_HASHTREE": "0"}
        environment.pop("IRIS_CALL_DESKTOP_CODEC", None)  # Exact echo permits frame-integrity checks.
        fixture, fixture_log = spawn_log("fixture", [str(args.bin_dir / "iris-call-fixture"), str(args.output / "account")],
            env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        read_lines(fixture, "fixture", fixture_log)
        deadline = time.monotonic() + 40
        ready = None
        while ready is None and time.monotonic() < deadline:
            _, line = events.get(timeout=max(0.1, deadline - time.monotonic()))
            assert line is not None, "Fixture stopped before ready"
            try:
                value = json.loads(line)
                if value.get("event") == "ready":
                    ready = value
            except json.JSONDecodeError:
                pass
        assert ready is not None, "Fixture did not create a test account"
        listening(fips_port)
        for package, apk, backup in backups:
            changed.append(backup)
            adb("install", "-r", str(apk))

        def command(value):
            fixture.stdin.write(value + "\n")
            fixture.stdin.flush()

        if args.voice_only:
            command("answer voice")
        parts = ["am", "instrument", "-w", "-r", "-e", "class", "to.iris.chat.calls.NativeCallFipsE2eTest"]
        extras = {"call_lan": "1", "call_relay": relay_url,
            "call_fips_seed": f"ws://{args.host}:{fips_port}/fips", "call_invite": ready["invite"],
            "call_peer_owner": ready["owner"], "call_answer_voice": "1" if args.voice_only else "0"}
        for key, value in extras.items():
            parts.extend(["-e", key, value])
        parts.append("to.iris.chat.test/to.iris.chat.calls.NativeCallTestRunner")
        test, test_log = spawn_log("android", adb_base + ["shell", shlex.join(parts)], stdout=subprocess.PIPE, text=True, bufsize=1)
        read_lines(test, "android", test_log)
        control = None
        ended = False
        passed = False
        server_stopped = False
        benchmark = None
        deadline = time.monotonic() + 210
        while time.monotonic() < deadline:
            name, line = events.get(timeout=max(0.1, deadline - time.monotonic()))
            if name == "fixture" and line:
                try:
                    value = json.loads(line)
                    ended |= value.get("event") == "call" and value.get("phase") == "ended"
                except json.JSONDecodeError:
                    pass
            elif name == "android":
                if line is None:
                    break
                if line.startswith("INSTRUMENTATION_STATUS: "):
                    key, _, value = line[len("INSTRUMENTATION_STATUS: "):].partition("=")
                    if key == "nativeCallOwner":
                        assert re.fullmatch(r"[a-f0-9]{64}", value)
                        command("accept " + value)
                    elif key == "nativeCallControlPath":
                        assert re.fullmatch(r"/data/user/\d+/to\.iris\.chat\.debug/cache/native-call-e2e-[a-f0-9-]+/remote-ended", value)
                        control = value
                    elif key == "nativeCallPhase" and value == "contact_ready":
                        relay.terminate()
                        relay.wait(timeout=5)
                        server_stopped = True
                    elif key in ("nativeLanBenchmark", "nativeCallResult", "nativeMutedCall"):
                        if key == "nativeLanBenchmark":
                            benchmark = value
                        print(key + ": " + value, flush=True)
                passed |= "OK (1 test)" in line
            if ended and control:
                adb("shell", "run-as", "to.iris.chat.debug", "touch", control)
                control = None
        assert passed and ended and server_stopped, "Physical LAN call failed; see private evidence"
        assert args.voice_only or benchmark is not None, "Missing sustained video benchmark"
        (args.output / "result.json").write_text(json.dumps({"passed": True, "wifi": True,
            "message_server_stopped": server_stopped, "remote_hangup": ended, "benchmark": benchmark}, indent=2))
        print("PASS physical Wi-Fi call, native codecs, server stopped, mute/camera off, and remote hangup", flush=True)
    finally:
        if changed:
            adb("pull", "/sdcard/Android/data/to.iris.chat.debug/files/call-tests/native-peer-log.json",
                str(args.output / "native-peer-log.json"), check=False)
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        # Stop only this dedicated test runner before restoring installed packages.
        if changed:
            adb("shell", "am", "force-stop", "to.iris.chat.debug", check=False)
        failures = []
        for backup in changed:
            try:
                adb("install", "-r", str(backup))
            except Exception as error:
                failures.append(str(error))
        for reader in readers:
            reader.join(timeout=2)
        for log in logs:
            log.close()
        if failures:
            raise RuntimeError("Could not restore all original test APKs; backups preserved in output")


if __name__ == "__main__":
    main()
