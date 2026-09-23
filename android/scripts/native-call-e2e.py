#!/usr/bin/env python3
"""Run fresh Android/native FIPS calls with WAN denied and the setup relay stopped.

Build APKs with -Piris.testRunner=to.iris.chat.calls.NativeCallTestRunner first.
Requires a root-capable Android emulator, adb, and the opt-in Rust fixture binaries.
Only test-package traffic is restricted; all firewall/reverse rules are removed on exit.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import shlex
import shutil
import subprocess
import tempfile
import threading
import time


def lines(process):
    result = queue.Queue()

    def read():
        for line in process.stdout:
            result.put(line.rstrip())
        result.put(None)

    threading.Thread(target=read, daemon=True).start()
    return result


def stop(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--adb", default=shutil.which("adb"))
    parser.add_argument("--relay-port", type=int, default=19871)
    parser.add_argument("--fips-port", type=int, default=19872)
    parser.add_argument("--answer-with-voice", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.adb or not args.serial.startswith("emulator-"):
        parser.error("An Android emulator and adb path are required; physical phones are excluded")
    android = Path(__file__).resolve().parents[1]
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="native-call-", dir=args.output))
    adb_base = [args.adb, "-s", args.serial]

    def adb(*command, check=True):
        return subprocess.run(adb_base + list(command), check=check, text=True, capture_output=True).stdout.strip()

    def shell(*command, check=True):
        return adb("shell", shlex.join(command), check=check)

    # Updating test binaries keeps existing app/account storage intact.
    adb("install", "-r", "-d", str(android / "app/build/outputs/apk/debug/app-debug.apk"))
    adb("install", "-r", "-d", str(android / "app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"))
    package = "to.iris.chat.debug"
    uid_match = re.search(r"^package:" + re.escape(package) + r" uid:(\d+)$",
                          shell("pm", "list", "packages", "-U", package), re.MULTILINE)
    if not uid_match or "uid=0" not in shell("su", "0", "id"):
        raise RuntimeError("Root-capable emulator and installed debug app are required")
    uid = uid_match.group(1)
    rules, reverses = [], []
    relay = fixture = instrument = None
    fixture_log = (work / "fixture.log").open("w")
    relay_log = (work / "relay.log").open("w")
    test_log = (work / "instrumentation.log").open("w")
    try:
        for port in (args.relay_port, args.fips_port):
            endpoint = f"tcp:{port}"
            if endpoint in adb("reverse", "--list"):
                raise RuntimeError(f"Refusing to replace an existing reverse for {endpoint}")
            adb("reverse", endpoint, endpoint)
            reverses.append(endpoint)
        comment = f"iris-call-e2e-{time.time_ns()}"
        for table, loopback in (("iptables", "127.0.0.0/8"), ("ip6tables", "::1/128")):
            rule = ["-m", "owner", "--uid-owner", uid, "!", "-d", loopback,
                    "-m", "comment", "--comment", comment, "-j", "REJECT"]
            shell("su", "0", table, "-I", "OUTPUT", "1", *rule)
            rules.append((table, rule))
        sandbox = shutil.which("sandbox-exec")
        if not sandbox:
            raise RuntimeError("macOS sandbox-exec is required to enforce fixture WAN isolation")
        prefix = [sandbox, "-p", '(version 1)(allow default)(deny network-outbound)'
                  '(allow network-outbound (remote ip "localhost:*"))']
        relay = subprocess.Popen(prefix + [str(args.relay_binary), f"127.0.0.1:{args.relay_port}"],
                                 stdout=relay_log, stderr=subprocess.STDOUT)
        environment = dict(os.environ, IRIS_DEMO_RELAYS=f"ws://127.0.0.1:{args.relay_port}",
                           IRIS_FIPS_WEBSOCKET_SEED_URLS="",
                           IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR=f"127.0.0.1:{args.fips_port}",
                           IRIS_CHAT_FIPS_UDP_BIND_ADDR="127.0.0.1:0")
        fixture = subprocess.Popen(prefix + [str(args.fixture), str(work / "fixture-data")],
                                   env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=fixture_log, text=True, bufsize=1)
        fixture_lines = lines(fixture)
        ready = None
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            try:
                line = fixture_lines.get(timeout=1)
            except queue.Empty:
                continue
            if line is None:
                raise RuntimeError("Native call fixture exited before ready")
            fixture_log.write(line + "\n"); fixture_log.flush()
            event = json.loads(line)
            if event.get("event") == "ready":
                ready = event
                break
        if ready is None:
            raise RuntimeError("Native call fixture did not become ready")

        def command(value):
            fixture.stdin.write(value + "\n")
            fixture.stdin.flush()

        if args.answer_with_voice:
            command("answer voice")
        invocation = ["am", "instrument", "-w", "-r", "-e", "class",
                      "to.iris.chat.calls.NativeCallFipsE2eTest",
                      "-e", "call_relay", f"ws://127.0.0.1:{args.relay_port}",
                      "-e", "call_fips_seed", f"ws://127.0.0.1:{args.fips_port}/fips",
                      "-e", "call_invite", ready["invite"],
                      "-e", "call_peer_owner", ready["owner"],
                      "-e", "call_answer_voice", "1" if args.answer_with_voice else "0",
                      "to.iris.chat.test/to.iris.chat.calls.NativeCallTestRunner"]
        instrument = subprocess.Popen(adb_base + ["shell", shlex.join(invocation)],
                                      stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        output = lines(instrument)
        result = None
        control_path = None
        passed = False
        relay_stopped = False
        deadline = time.monotonic() + 210
        while time.monotonic() < deadline:
            try:
                line = output.get(timeout=1)
            except queue.Empty:
                continue
            if line is None:
                break
            test_log.write(line + "\n"); test_log.flush()
            if "nativeCallOwner=" in line:
                command("accept " + line.split("nativeCallOwner=", 1)[1])
            elif "nativeCallControlPath=" in line:
                control_path = line.split("nativeCallControlPath=", 1)[1]
                if not re.fullmatch(r"/data/(user/0|data)/" + re.escape(package) +
                                    r"/cache/native-call-e2e-[a-f0-9-]+/remote-ended", control_path):
                    raise RuntimeError("Unexpected test-only control path")
            elif "nativeCallPhase=contact_ready" in line:
                stop(relay)
                relay_stopped = True
                print("Local message server stopped; proceeding with FIPS-only call", flush=True)
            elif "nativeCallPhase=hangup_sent" in line:
                hangup_deadline = time.monotonic() + 8
                while time.monotonic() < hangup_deadline:
                    command("status")
                    try:
                        fixture_line = fixture_lines.get(timeout=0.25)
                    except queue.Empty:
                        continue
                    if fixture_line is None:
                        break
                    fixture_log.write(fixture_line + "\n"); fixture_log.flush()
                    event = json.loads(fixture_line)
                    if event.get("event") == "status" and (event.get("call") or {}).get("phase") == "ended":
                        shell("run-as", package, "touch", control_path)
                        break
            elif "nativeCallResult=" in line:
                result = line.split("nativeCallResult=", 1)[1]
                command("status")
            elif "nativeCallError=" in line:
                print(line, flush=True)
            if "OK (1 test)" in line:
                passed = True
        fixture_status = None
        command("status")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                line = fixture_lines.get(timeout=0.5)
            except queue.Empty:
                continue
            if line is None:
                break
            fixture_log.write(line + "\n"); fixture_log.flush()
            event = json.loads(line)
            if event.get("event") == "status":
                fixture_status = event
                break
        remote_ended = fixture_status is not None and (fixture_status.get("call") or {}).get("phase") == "ended"
        summary = {"passed": passed and result is not None and relay_stopped and remote_ended, "android": result,
                   "fixture": fixture_status, "relay_stopped": relay_stopped,
                   "wan_blocked": True, "remote_hangup": remote_ended, "answer_with_voice": args.answer_with_voice}
        (work / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary), flush=True)
        if not summary["passed"]:
            raise RuntimeError("Native call e2e failed; see instrumentation.log and fixture.log in output directory")
    finally:
        stop(instrument)
        if fixture is not None and fixture.poll() is None:
            try:
                fixture.stdin.write("stop\n"); fixture.stdin.flush()
                fixture.wait(5)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                stop(fixture)
        stop(relay)
        for table, rule in reversed(rules):
            shell("su", "0", table, "-D", "OUTPUT", *rule, check=False)
        for endpoint in reverses:
            adb("reverse", "--remove", endpoint, check=False)
        fixture_log.close(); relay_log.close(); test_log.close()


if __name__ == "__main__":
    main()
