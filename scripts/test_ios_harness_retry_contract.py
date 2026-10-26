#!/usr/bin/env python3

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("run_ios_harness.py")
SPEC = importlib.util.spec_from_file_location("run_ios_harness", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)

PHONE_SPEC = importlib.util.spec_from_file_location(
    "run_public_link_phone", MODULE_PATH.with_name("run_public_link_phone.py"))
assert PHONE_SPEC is not None and PHONE_SPEC.loader is not None
PHONE = importlib.util.module_from_spec(PHONE_SPEC)
with mock.patch.object(sys, "path", [str(MODULE_PATH.parent), *sys.path]):
    PHONE_SPEC.loader.exec_module(PHONE)


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
                simulator=True,
                pre_body_timeout_secs=120,
                pre_test_retries=1,
            )
        self.assertEqual(result.returncode, 124)
        self.assertEqual(run_test.call_count, 2)
        reboot.assert_called_once()

    def test_physical_device_pre_body_failure_waits_and_retries(self):
        failure = subprocess.CompletedProcess(["xcodebuild"], 70, "Device is busy")
        success = subprocess.CompletedProcess(["xcodebuild"], 0, "HARNESS_STATUS: action=probe")
        with mock.patch.object(HARNESS, "run_test", side_effect=[failure, success]) as run_test, mock.patch.object(
            HARNESS, "reboot_simulator"
        ) as reboot, mock.patch.object(HARNESS.time, "sleep") as sleep:
            result = HARNESS.run_test_with_retries(
                "physical-device",
                Path("test.xctestrun"),
                timeout_secs=420,
                simulator=False,
                pre_test_retries=1,
            )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(run_test.call_count, 2)
        reboot.assert_not_called()
        sleep.assert_called_once_with(10)


class PublicLinkPhoneCleanupTests(unittest.TestCase):
    def run_phone(self, *, test_exit=0, test_error=None, cleanup_error=None,
                  listed_device="selected-phone"):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "owned-link-run"
            directory.mkdir()
            spec = directory / "input.json"
            spec.write_text(json.dumps({"run": str(directory), "uri": "test-invite",
                                        "body": "owned pre-link message"}))
            product = directory / "products.xctestrun"
            target = directory / "prepared.xctestrun"
            target.touch()
            calls = []

            def command(argv, **kwargs):
                calls.append((argv, kwargs))
                if argv[2:4] == ["list", "devices"]:
                    Path(argv[argv.index("--json-output") + 1]).write_text(json.dumps({
                        "result": {"devices": [{"hardwareProperties": {
                            "udid": listed_device, "reality": "physical", "platform": "iOS"}}]}}))
                elif cleanup_error:
                    raise cleanup_error
                return subprocess.CompletedProcess(argv, 0)

            with mock.patch.dict(os.environ, {
                "IRIS_LINK_TEST_UDID": "selected-phone",
                "DEVICECTL_CHILD_IRIS_UI_TEST_DATA_DIR": "/ordinary-account",
                "DEVICECTL_CHILD_IRIS_UI_TEST_RESET": "1",
                "DEVICECTL_CHILD_IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION": "1",
            }, clear=True), mock.patch.object(sys, "argv", ["phone.py", str(spec)]), \
                mock.patch.object(PHONE.subprocess, "run", side_effect=command), \
                mock.patch.object(PHONE, "selected_products", return_value=product), \
                mock.patch.object(PHONE, "acquire", return_value=("owned-lock", {})) as acquire, \
                mock.patch.object(PHONE, "release") as release, \
                mock.patch.object(PHONE.harness, "prepare_xctestrun", return_value=target) as prepare, \
                mock.patch.object(PHONE.harness, "run_test", side_effect=test_error,
                                  return_value=subprocess.CompletedProcess([], test_exit)) as run_test:
                try:
                    result = PHONE.main()
                    error = None
                except Exception as caught:
                    result, error = None, caught
            return result, error, calls, acquire, release, prepare, run_test, target.exists()

    def assert_isolated_cleanup(self, result):
        _, _, calls, acquire, _, prepare, run_test, target_exists = result
        self.assertEqual(len(calls), 2)
        argv, options = calls[-1]
        self.assertEqual(argv[:5], ["xcrun", "devicectl", "device", "process", "launch"])
        self.assertEqual(argv[argv.index("--device") + 1], "selected-phone")
        self.assertIn("--terminate-existing", argv)
        self.assertEqual(argv[-1], "fi.siriusbusiness.irischat")
        self.assertEqual(json.loads(argv[argv.index("--environment-variables") + 1]), {
            "IRIS_UI_TEST_RUN_ID": "harness-owned-link-run", "IRIS_DISABLE_NOTIFICATIONS": "1"})
        self.assertFalse(any(key.startswith("DEVICECTL_CHILD_") for key in options["env"]))
        self.assertTrue(options["check"])
        self.assertEqual(options["timeout"], 30)
        acquire.assert_called_once_with("ios-device:selected-phone", stale_after=6 * 60 * 60)
        self.assertEqual(prepare.call_args.args[1]["IRIS_IOS_HARNESS_RUN_ID"], "owned-link-run")
        self.assertEqual(prepare.call_args.args[1]["IRIS_IOS_HARNESS_USE_APP_STORAGE"], "0")
        self.assertEqual(run_test.call_args.args[0], "selected-phone")
        self.assertFalse(target_exists)

    def test_success_relaunches_same_isolated_account(self):
        result = self.run_phone()
        self.assert_isolated_cleanup(result)
        self.assertEqual(result[:2], (0, None))
        result[4].assert_called_once_with("owned-lock")

    def test_test_failure_relaunches_same_isolated_account(self):
        result = self.run_phone(test_exit=1)
        self.assert_isolated_cleanup(result)
        self.assertEqual(result[:2], (1, None))
        result[4].assert_called_once_with("owned-lock")

    def test_exception_relaunches_same_isolated_account(self):
        failure = RuntimeError("test host failed")
        result = self.run_phone(test_error=failure)
        self.assert_isolated_cleanup(result)
        self.assertIs(result[1], failure)
        result[4].assert_called_once_with("owned-lock")

    def test_cleanup_failure_keeps_device_reserved(self):
        failure = subprocess.CalledProcessError(1, ["devicectl"])
        result = self.run_phone(cleanup_error=failure)
        self.assertIs(result[1], failure)
        self.assertEqual(len(result[2]), 2)
        result[4].assert_not_called()

    def test_different_device_is_rejected_without_launch_or_lease(self):
        result = self.run_phone(listed_device="other-phone")
        self.assertIsInstance(result[1], ValueError)
        self.assertEqual(len(result[2]), 1)
        result[3].assert_not_called()
        result[4].assert_not_called()
        result[6].assert_not_called()


if __name__ == "__main__":
    unittest.main()
