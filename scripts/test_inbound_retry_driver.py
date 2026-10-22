#!/usr/bin/env python3
"""Fail-closed checks for the temporary hosted regression driver; no Cargo."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("retry_driver", Path(__file__).with_name("verify_inbound_retry.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


class ResultTests(unittest.TestCase):
    def setUp(self):
        self.test = {"name": "protocol_engine::tests::allocation", "expected_count": 1, "marker": "executed allocation"}
        self.passed = "running 1 test\ntest protocol_engine::tests::allocation ... executed allocation\nok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 100 filtered out;\n"
        self.failed = self.passed.replace("\nok\n", "\nthread 'test' panicked at source.rs:1: allocated 9000 bytes copying deferred packets\nFAILED\n").replace("ok. 1 passed; 0 failed", "FAILED. 0 passed; 1 failed")

    def test_exact_pass_and_intended_failure(self):
        self.assertEqual(DRIVER.check_result(self.passed, self.test, 0)["passed"], 1)
        self.assertTrue(DRIVER.check_result(self.failed, self.test, 101, "bytes copying deferred packets")["expected_regression_failure"])

    def test_zero_tests_compile_errors_and_wrong_failure_cannot_count_as_red(self):
        cases = ["error: could not compile iris-chat-protocol", self.failed.replace("running 1 test", "running 0 tests").replace("0 passed; 1 failed", "0 passed; 0 failed"), self.failed.replace("bytes copying deferred packets", "fixture storage unavailable"), self.failed.replace("executed allocation", "")]
        for text in cases:
            with self.subTest(text=text):
                with self.assertRaises(AssertionError):
                    DRIVER.check_result(text, self.test, 101, "bytes copying deferred packets")

    def test_unexpected_pass_or_extra_test_result_is_rejected(self):
        for text, code in [(self.passed, 0), (self.failed + self.failed, 101), (self.failed, -9)]:
            with self.subTest(code=code):
                with self.assertRaises(AssertionError):
                    DRIVER.check_result(text, self.test, code, "bytes copying deferred packets")

    def test_group_requires_actual_five_passes(self):
        test = dict(self.test, expected_count=5)
        with self.assertRaises(AssertionError):
            DRIVER.check_result(self.passed, test, 0)
        text = self.passed.replace("running 1 test", "running 5 tests").replace("1 passed", "5 passed")
        self.assertEqual(DRIVER.check_result(text, test, 0)["passed"], 5)

    def test_disabled_contract_stops_before_source_or_toolchain(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"
            path.write_text('{"enabled": false}')
            with mock.patch.object(DRIVER, "INPUTS", path):
                with self.assertRaisesRegex(AssertionError, "Diagnostic disabled"):
                    DRIVER.contract()


class ResourceTests(unittest.TestCase):
    def run_owned_child(self, monitor_fails):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            real_read = Path.read_text
            real_check = subprocess.check_output

            def read(path, *args, **kwargs):
                return "MemTotal: 8388608 kB\n" if str(path) == "/proc/meminfo" else real_read(path, *args, **kwargs)

            def check(command, *args, **kwargs):
                if monitor_fails and command[0] == "ps":
                    raise subprocess.TimeoutExpired(command, 10)
                return real_check(command, *args, **kwargs)

            with mock.patch.object(Path, "read_text", read), mock.patch.object(DRIVER.subprocess, "check_output", check), mock.patch.object(DRIVER.shutil, "disk_usage", return_value=type("Disk", (), {"free": 10 * DRIVER.GIB})()):
                with self.assertRaises(AssertionError):
                    DRIVER.bounded([sys.executable, "-c", "import time; time.sleep(30)"], root, root, "injected", 0.001 if not monitor_fails else 10, time.time() + 20, root)
            result = json.loads((root / "injected-status.json").read_text())
            self.assertTrue(result["owned_process_reaped"])
            self.assertLess(result["elapsed_seconds"], 12)
            self.assertIsNotNone(result["exit_code"])
            self.assertIsNotNone(result["log_sha256"])
            self.assertIn("TimeoutExpired" if monitor_fails else "time limit", result["stop_reason"])

    def test_timeout_terminates_and_reaps_only_owned_process(self):
        self.run_owned_child(False)

    def test_monitor_error_still_terminates_and_records_owned_process(self):
        self.run_owned_child(True)


if __name__ == "__main__":
    unittest.main()
