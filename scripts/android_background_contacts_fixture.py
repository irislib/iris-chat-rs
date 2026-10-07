#!/usr/bin/env python3
"""Add verified synthetic contacts, then take them offline for receiver power tests.

Only operates on the disposable backgroundtest package. Run the background
delivery harness afterward to restore its live message server and measure idle.
"""
import argparse
import base64
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--account-evidence", type=Path, required=True,
                        help="Current account's prepare_background_receiver.log")
    parser.add_argument("--count", type=int, default=8)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert 1 <= args.count <= 20
    owner = re.search(r"INSTRUMENTATION_STATUS: owner=([a-f0-9]{64})",
                      args.account_evidence.read_text()).group(1)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    package = "to.iris.chat.backgroundtest"
    adb_base = ["adb", "-s", args.serial]
    lock, _ = acquire(f"android:{args.serial}", 3600)
    assert lock is not None, "Pixel is reserved by another test"
    processes, logs, readers = [], [], []
    port = None

    def adb(*parts):
        result = subprocess.run(adb_base + list(parts), capture_output=True, text=True, timeout=300)
        if result.returncode:
            (args.output / "adb-failure.log").write_text(result.stdout + result.stderr)
            raise RuntimeError("Android operation failed; inspect private evidence")
        return result.stdout

    try:
        assert adb("shell", "pm", "path", package).strip().startswith("package:")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        relay_log = (args.output / "relay.log").open("w")
        logs.append(relay_log)
        relay = subprocess.Popen([str(args.bin_dir / "local_nostr_relay"), f"127.0.0.1:{port}"],
                                 stdout=relay_log, stderr=relay_log)
        processes.append(relay)
        deadline = time.monotonic() + 30
        while True:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    break
            except OSError:
                assert time.monotonic() < deadline, "Local message server did not start"
                time.sleep(0.1)
        adb("reverse", f"tcp:{port}", f"tcp:{port}")
        contacts = []
        for index in range(args.count):
            log = (args.output / f"fixture-{index}.log").open("w")
            logs.append(log)
            fixture = subprocess.Popen(
                [str(args.bin_dir / "iris-call-fixture"), str(args.output / f"contact-{index}")],
                env={**os.environ, "IRIS_DEMO_RELAYS": f"ws://127.0.0.1:{port}"},
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, bufsize=1)
            processes.append(fixture)
            events = queue.Queue()

            def read_events(process=fixture, output=events):
                for line in process.stdout:
                    output.put(json.loads(line))
            reader = threading.Thread(target=read_events, daemon=True)
            reader.start()
            readers.append(reader)
            ready = events.get(timeout=45)
            assert ready["event"] == "ready"
            contacts.append({key: ready[key] for key in ("invite", "owner", "device")})
            fixture.stdin.write(f"accept {owner}\n")
            fixture.stdin.flush()
            deadline = time.monotonic() + 30
            while events.get(timeout=max(0.01, deadline - time.monotonic()))["event"] != "accepted":
                assert time.monotonic() < deadline
        encoded = base64.b64encode(json.dumps(contacts).encode()).decode()
        adb("shell", "input", "keyevent", "WAKEUP")
        result = adb("shell", "am", "instrument", "-w", "-r", "-e", "class",
                     "to.iris.chat.push.BackgroundDeliveryHarnessTest#prepare_offline_contacts",
                     "-e", "fixture_account_owner", owner,
                     "-e", "fixture_contacts_b64", encoded,
                     "-e", "fixture_relay", f"ws://127.0.0.1:{port}",
                     "to.iris.chat.backgroundtest.test/androidx.test.runner.AndroidJUnitRunner")
        (args.output / "prepare.log").write_text(result)
        assert "OK (1 test)" in result, "Contact setup failed; inspect private evidence"
        total = re.search(r"INSTRUMENTATION_STATUS: total_chats=(\d+)", result).group(1)
        print(f"PASS {args.count} synthetic contacts verified; {total} total chats", flush=True)
    finally:
        subprocess.run(adb_base + ["shell", "am", "force-stop", package], capture_output=True, timeout=30)
        for process in reversed(processes):
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if port:
            subprocess.run(adb_base + ["reverse", "--remove", f"tcp:{port}"], capture_output=True, timeout=30)
        for reader in readers:
            reader.join(timeout=2)
        for log in logs:
            log.close()
        subprocess.run(adb_base + ["shell", "input", "keyevent", "SLEEP"], capture_output=True, timeout=30)
        release(lock)
    print("Synthetic contacts are now offline; ready for background delivery test", flush=True)


if __name__ == "__main__":
    main()
