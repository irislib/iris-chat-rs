#!/usr/bin/env python3

import importlib.util
import pathlib
import subprocess
from unittest.mock import patch
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
RUN_HARNESS = SCRIPT_DIR / "run_harness.py"
SPEC = importlib.util.spec_from_file_location("run_harness", RUN_HARNESS)
assert SPEC is not None
run_harness = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(run_harness)


class InstrumentationResultTests(unittest.TestCase):
    def classify(self, lines: list[str], exit_code: int = 0) -> int:
        return run_harness.classify_instrumentation_result(lines, exit_code)

    def test_normal_success(self) -> None:
        self.assertEqual(self.classify(["INSTRUMENTATION_CODE: -1\n"]), 0)

    def test_negative_status_fails(self) -> None:
        self.assertEqual(
            self.classify(
                [
                    "INSTRUMENTATION_STATUS_CODE: -2\n",
                    "INSTRUMENTATION_CODE: -1\n",
                ]
            ),
            1,
        )

    def test_post_success_process_crashed_is_teardown_success(self) -> None:
        self.assertEqual(
            self.classify(
                [
                    "INSTRUMENTATION_STATUS_CODE: 0\n",
                    "INSTRUMENTATION_RESULT: shortMsg=Process crashed.\n",
                    "INSTRUMENTATION_CODE: 0\n",
                ]
            ),
            0,
        )

    def test_process_crashed_before_success_fails(self) -> None:
        self.assertEqual(
            self.classify(
                [
                    "INSTRUMENTATION_RESULT: shortMsg=Process crashed.\n",
                    "INSTRUMENTATION_CODE: 0\n",
                ]
            ),
            1,
        )

    def test_nonzero_process_exit_fails(self) -> None:
        self.assertEqual(self.classify(["INSTRUMENTATION_CODE: -1\n"], exit_code=42), 42)


class AndroidTargetSafetyTests(unittest.TestCase):
    def test_primary_or_unspecified_user_is_rejected_before_adb(self) -> None:
        for user in ("", "0", "current", "-1"):
            with self.subTest(user=user), patch.object(run_harness.subprocess, "run") as adb:
                with self.assertRaises(SystemExit):
                    run_harness.verify_isolated_android_user("adb", "serial", user)
                adb.assert_not_called()

    def test_ordinary_secondary_user_is_rejected(self) -> None:
        result = subprocess.CompletedProcess([], 0, "Users:\n UserInfo{10:Personal:410} running\n", "")
        with patch.object(run_harness.subprocess, "run", return_value=result) as adb:
            with self.assertRaisesRegex(SystemExit, "not ephemeral"):
                run_harness.verify_isolated_android_user("adb", "serial", "10")
            self.assertEqual(adb.call_count, 1)

    def test_ephemeral_user_must_be_current(self) -> None:
        replies = [subprocess.CompletedProcess([], 0, "UserInfo{10:Push check:510} running\n", ""),
                   subprocess.CompletedProcess([], 0, "0\n", "")]
        with patch.object(run_harness.subprocess, "run", side_effect=replies):
            with self.assertRaisesRegex(SystemExit, "Switch to"):
                run_harness.verify_isolated_android_user("adb", "serial", "10")

    def test_current_ephemeral_user_is_accepted(self) -> None:
        replies = [subprocess.CompletedProcess([], 0, "UserInfo{10:Push check:510} running\n", ""),
                   subprocess.CompletedProcess([], 0, "10\n", "")]
        with patch.object(run_harness.subprocess, "run", side_effect=replies) as adb:
            run_harness.verify_isolated_android_user("adb", "serial", "10")
            self.assertEqual(adb.call_args_list[0].args[0][:3], ["adb", "-s", "serial"])

    def test_mismatched_instrumentation_target_is_rejected(self) -> None:
        result = subprocess.CompletedProcess([], 0,
            "instrumentation:to.iris.chat.test/Runner (target=to.iris.chat.debug)\n", "")
        with patch.object(run_harness.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(SystemExit, "does not target"):
                run_harness.verify_instrumentation_target("adb", "serial", "to.iris.chat.test/Runner", "to.iris.chat.pushcheck")
            run_harness.verify_instrumentation_target("adb", "serial", "to.iris.chat.test/Runner", "to.iris.chat.debug")


if __name__ == "__main__":
    unittest.main()
