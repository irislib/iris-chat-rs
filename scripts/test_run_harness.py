#!/usr/bin/env python3

import importlib.util
import pathlib
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


if __name__ == "__main__":
    unittest.main()
