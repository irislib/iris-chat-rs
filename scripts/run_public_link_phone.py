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
import plistlib
import subprocess
import sys

import run_ios_harness as harness
from native_lab import acquire, release


def selected_products() -> Path:
    """Use a reviewed hosted product set without rebuilding or copying it."""
    explicit = os.environ.get("IRIS_LINK_TEST_XCTESTRUN")
    source = Path(explicit) if explicit else harness.find_xctestrun(prefer_simulator=False)
    if source is None or not source.is_absolute() or not source.is_file():
        raise ValueError("Select existing physical iPhone development test products")
    if source.suffix != ".xctestrun":
        raise ValueError("The selected product must be an xctestrun file")
    with source.open("rb") as stream:
        config = plistlib.load(stream)
    target = config.get("IrisChatTests", {})
    if not target or "Debug-iphoneos" not in target.get("TestHostPath", ""):
        raise ValueError("The selected products must contain the physical iPhone unit harness")
    return source


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
    source = selected_products()
    lock, _ = acquire("ios-device:" + selected, stale_after=6 * 60 * 60)
    if lock is None:
        raise RuntimeError("The selected iPhone is reserved by another native test")
    launched = False
    restored = False
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
            env["IRIS_IOS_HARNESS_FIPS_SEEDS_B64"] = ""
        elif os.environ.get("IRIS_LINK_TEST_FIPS_SEEDS"):
            env["IRIS_IOS_HARNESS_FIPS_SEEDS_B64"] = base64.b64encode(
                os.environ["IRIS_LINK_TEST_FIPS_SEEDS"].encode()).decode()
        target = harness.prepare_xctestrun(source, env)
        target.chmod(0o600)
        launched = True
        return harness.run_test(selected, target, timeout_secs=360,
                                pre_body_timeout_secs=90).returncode
    finally:
        try:
            if launched:
                # End the test host in the same isolated account. A normal launch
                # here would open ordinary storage in the development app.
                cleanup_env = {key: value for key, value in os.environ.items()
                               if not key.startswith("DEVICECTL_CHILD_")}
                app_env = {"IRIS_UI_TEST_RUN_ID": "harness-" + directory.name,
                           "IRIS_DISABLE_NOTIFICATIONS": "1"}
                with (directory / "phone-restore.log").open("w") as log:
                    subprocess.run(["xcrun", "devicectl", "device", "process", "launch",
                                    "--device", selected, "--terminate-existing",
                                    "--environment-variables", json.dumps(app_env),
                                    "fi.siriusbusiness.irischat"], stdout=log,
                                   stderr=subprocess.STDOUT, check=True, timeout=30,
                                   env=cleanup_env)
                restored = True
            if target is not None:
                target.unlink(missing_ok=True)
        finally:
            # A failed isolated relaunch needs owner attention; never advertise
            # the phone as free while the test host's state is uncertain.
            if not launched or restored:
                release(lock)


if __name__ == "__main__":
    raise SystemExit(main())
