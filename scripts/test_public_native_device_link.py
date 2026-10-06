#!/usr/bin/env python3
"""Opt-in physical iPhone -> native linking/history over the public servers.

Reuse the selected development test products and an explicitly selected phone.
Both accounts are isolated; evidence and the native profile remain private.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid


ROOT = Path(__file__).resolve().parent.parent


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "core/target/debug/iris")
    options = parser.parse_args()
    binary = options.binary.resolve()
    if not os.environ.get("IRIS_LINK_TEST_UDID"):
        parser.error("Set IRIS_LINK_TEST_UDID to the explicitly selected physical iPhone")
    if not binary.is_file():
        parser.error("Build the intended native CLI and physical iPhone test products first")
    if any(key in os.environ for key in (
        "IRIS_LINK_TEST_FIPS_SEEDS", "IRIS_LINK_TEST_SEEDLESS",
        "IRIS_FIPS_WEBSOCKET_SEED_URLS", "IRIS_DEMO_RELAYS",
        "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR", "IRIS_CHAT_SAME_HOST_HASHTREE",
        "IRIS_CHAT_FIPS_STATIC_PEERS", "IRIS_CHAT_FIPS_ROUTED_PEERS",
    )):
        parser.error("This gate requires the default public message and FIPS servers")

    directory = ROOT / "work/public-native-device-link" / uuid.uuid4().hex
    directory.mkdir(parents=True, mode=0o700)
    # A long checkout/evidence path exceeds the native service's Unix socket limit.
    profile = Path(tempfile.mkdtemp(prefix="iris-link-"))
    args = [str(binary), "--json", "--data-dir", str(profile)]
    body = "Public native link history " + uuid.uuid4().hex
    result = {
        "passed": False,
        "source": subprocess.check_output(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
        "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "profile": str(profile),
    }
    service = None
    phone = None

    def call(*command):
        completed = subprocess.run(args + list(command), capture_output=True,
                                   text=True, timeout=30)
        parsed = json.loads(completed.stdout)
        if completed.returncode or parsed.get("status") != "ok":
            raise RuntimeError("Native command failed: " + command[0])
        return parsed["data"]

    def start_service():
        nonlocal service
        with (directory / "service.log").open("a") as log:
            offset = log.tell()
            service = subprocess.Popen(args + ["service", "run"], stdout=log,
                                       stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            with (directory / "service.log").open() as log:
                log.seek(offset)
                ready = '\"ready\":true' in log.read()
            if ready:
                return
            if service.poll() is not None:
                break
            time.sleep(.2)
        raise RuntimeError("Native service did not become ready")

    def stop_service():
        if service is None or service.poll() is not None:
            return
        try:
            call("service", "stop")
            service.wait(timeout=15)
        except (ValueError, RuntimeError, subprocess.TimeoutExpired):
            service.terminate()
            service.wait(timeout=15)

    def wait_history(account, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            messages = call("read", account["user_id"]).get("messages", [])
            count = sum(message.get("body") == body for message in messages)
            if count == 1:
                return
            if count > 1:
                raise RuntimeError("Native pre-link history arrived more than once")
            time.sleep(2)
        raise RuntimeError("Native pre-link history did not arrive")

    print("Private evidence:", directory, flush=True)
    try:
        start_service()
        link = call("link", "create")
        spec = directory / "input.json"
        spec.write_text(json.dumps({"run": str(directory), "uri": link["url"], "body": body}))
        with (directory / "phone.log").open("w") as log:
            phone = subprocess.Popen(
                [sys.executable, str(ROOT / "scripts/run_public_link_phone.py"), str(spec)],
                cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 220
        account = None
        while time.monotonic() < deadline:
            state = call("state")
            (directory / "state.json").write_text(json.dumps(state, indent=2))
            account = state.get("account")
            if account and account.get("device_state") == "authorized":
                break
            if phone.poll() is not None:
                raise RuntimeError("Phone stopped before native sign-in")
            time.sleep(2)
        else:
            raise RuntimeError("Native did not sign in")
        print("Native signed in; checking history", flush=True)
        wait_history(account, 90)
        stop_service()
        start_service()
        wait_history(account, 20)
        result["passed"] = True
        print("Pre-link history survives native restart exactly once", flush=True)
    except Exception as error:
        result["error"] = str(error)
        if service is not None and service.poll() is None:
            try:
                (directory / "native-debug.json").write_text(json.dumps(
                    call("debug", "--wait-ms", "0"), indent=2))
            except (ValueError, RuntimeError, subprocess.TimeoutExpired):
                pass
    finally:
        try:
            if phone is not None:
                result["phoneExit"] = phone.wait(timeout=380)
                result["passed"] &= result["phoneExit"] == 0
        except subprocess.TimeoutExpired:
            result.update(passed=False, error="Phone test/restoration did not finish")
            # Allow the helper's finally block to restore the ordinary development app.
            phone.send_signal(2)
            phone.wait(timeout=40)
        finally:
            try:
                stop_service()
            except (OSError, subprocess.TimeoutExpired) as error:
                result.update(passed=False, cleanupError=str(error))
            (directory / "result.json").write_text(json.dumps(result, indent=2))
    print("Public native linking and durable history passed" if result["passed"]
          else "Public native linking/history failed; see private evidence", flush=True)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
