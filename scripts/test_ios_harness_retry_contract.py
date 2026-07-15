#!/usr/bin/env python3

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("run_ios_harness.py")
SPEC = importlib.util.spec_from_file_location("run_ios_harness", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class StuckBootstatus:
    returncode = None

    def poll(self):
        return None

    def kill(self):
        self.returncode = -9

    def wait(self, timeout=None):
        return self.returncode


class IOSHarnessRetryContractTests(unittest.TestCase):
    def test_default_pre_body_timeout_is_bounded(self):
        with mock.patch.object(sys, "argv", ["run_ios_harness.py", "--action", "probe"]), mock.patch.dict(
            os.environ,
            {
                "IRIS_IOS_HARNESS_PRE_BODY_TIMEOUT_SECS": "",
                "IRIS_IOS_HARNESS_PRE_TEST_RETRIES": "1",
            },
            clear=False,
        ):
            os.environ.pop("IRIS_IOS_HARNESS_PRE_BODY_TIMEOUT_SECS", None)
            self.assertEqual(HARNESS.parse_args().pre_body_timeout_secs, 120)

    def test_booted_label_does_not_override_failed_bootstatus(self):
        with mock.patch.object(HARNESS.subprocess, "Popen", return_value=StuckBootstatus()), mock.patch.object(
            HARNESS, "simulator_is_booted", return_value=True
        ), mock.patch.dict(
            os.environ,
            {
                "IRIS_IOS_BOOTSTATUS_TIMEOUT_SECS": "0",
                "IRIS_IOS_BOOTSTATUS_FALLBACK_SLEEP_SECS": "0",
            },
        ):
            with self.assertRaises(SystemExit):
                HARNESS.wait_for_simulator_boot("11111111-1111-1111-1111-111111111111")

    def test_booted_label_still_requires_bootstatus(self):
        udid = "11111111-1111-1111-1111-111111111111"
        with mock.patch.object(HARNESS, "simulator_is_booted", return_value=True), mock.patch.object(
            HARNESS, "boot_simulator"
        ) as boot, mock.patch.object(HARNESS, "wait_for_simulator_boot") as wait:
            HARNESS.ensure_simulator_booted(udid)
        boot.assert_not_called()
        wait.assert_called_once_with(udid)

    def test_pre_body_retry_count_is_exact(self):
        failure = subprocess.CompletedProcess(["xcodebuild"], 124, "INSTRUMENTATION_FAILED: pre-body timeout")
        with mock.patch.object(HARNESS, "run_test", side_effect=[failure, failure]) as run_test, mock.patch.object(
            HARNESS, "reboot_simulator"
        ) as reboot:
            result = HARNESS.run_test_with_retries(
                "11111111-1111-1111-1111-111111111111",
                Path("test.xctestrun"),
                timeout_secs=420,
                pre_body_timeout_secs=120,
                pre_test_retries=1,
            )
        self.assertEqual(result.returncode, 124)
        self.assertEqual(run_test.call_count, 2)
        reboot.assert_called_once()


if __name__ == "__main__":
    unittest.main()
