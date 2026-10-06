#!/usr/bin/env python3
"""Phone half of the opt-in iris-chat public device-link browser test.

Requires an explicitly selected physical iPhone and existing development test
products. The test creates an isolated account; it never opens ordinary storage.
"""
from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import subprocess
import sys

import run_ios_harness as harness
from native_lab import acquire, release


def main():
    os.umask(0o077)
    spec = json.loads(Path(sys.argv[1]).read_text())
    directory = Path(spec["run"]).resolve()
    selected = os.environ.get("IRIS_LINK_TEST_UDID")
    if not selected or not directory.is_dir():
        raise ValueError("Set IRIS_LINK_TEST_UDID to the explicitly selected physical iPhone")
    devices_path = directory / "devices.json"
    subprocess.run(["xcrun", "devicectl", "list", "devices", "--quiet",
                    "--json-output", str(devices_path)], check=True, timeout=30)
    matches = [device for device in json.loads(devices_path.read_text())["result"]["devices"]
               if device.get("hardwareProperties", {}).get("udid") == selected
               and device.get("hardwareProperties", {}).get("reality") == "physical"
               and device.get("hardwareProperties", {}).get("platform") == "iOS"]
    if len(matches) != 1:
        raise ValueError("The selected identifier must match exactly one paired physical iPhone")
    source = harness.find_xctestrun(prefer_simulator=False)
    if source is None:
        raise ValueError("Build the physical iPhone development test products first")
    lock, _ = acquire("ios-device:" + selected, stale_after=6 * 60 * 60)
    if lock is None:
        raise RuntimeError("The selected iPhone is reserved by another native test")
    launched = False
    target = None
    try:
        env = {
            "IRIS_IOS_HARNESS_ACTION": "link_browser_with_history_from_args",
            "IRIS_IOS_HARNESS_RUN_ID": directory.name,
            "IRIS_IOS_HARNESS_USE_APP_STORAGE": "0",
            "IRIS_DISABLE_NOTIFICATIONS": "1",
            "IRIS_PERF_LOG": "1",
        }
        for key, value in {"DEVICE_INPUT": spec["uri"], "MESSAGE": spec["body"],
                           "DISPLAY_NAME": "Public link test"}.items():
            env["IRIS_IOS_HARNESS_" + key + "_B64"] = base64.b64encode(value.encode()).decode()
        if os.environ.get("IRIS_LINK_TEST_SEEDLESS") == "1":
            env["IRIS_FIPS_WEBSOCKET_SEED_URLS"] = ""
        elif os.environ.get("IRIS_LINK_TEST_FIPS_SEEDS"):
            env["IRIS_FIPS_WEBSOCKET_SEED_URLS"] = os.environ["IRIS_LINK_TEST_FIPS_SEEDS"]
        target = harness.prepare_xctestrun(source, env)
        target.chmod(0o600)
        launched = True
        return harness.run_test(selected, target, timeout_secs=360,
                                pre_body_timeout_secs=90).returncode
    finally:
        try:
            if launched:
                with (directory / "phone-restore.log").open("w") as log:
                    subprocess.run(["xcrun", "devicectl", "device", "process", "launch",
                                    "--device", selected, "--terminate-existing",
                                    "fi.siriusbusiness.irischat"], stdout=log,
                                   stderr=subprocess.STDOUT, check=True, timeout=30)
            if target is not None:
                target.unlink(missing_ok=True)
        finally:
            release(lock)


if __name__ == "__main__":
    raise SystemExit(main())
