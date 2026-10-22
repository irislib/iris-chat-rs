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
PERF_SPEC = importlib.util.spec_from_file_location("inbound_retry_perf", Path(__file__).with_name("inbound_retry_perf.py"))
PERF = importlib.util.module_from_spec(PERF_SPEC)
PERF_SPEC.loader.exec_module(PERF)


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


class OptimizedTests(unittest.TestCase):
    def test_optimized_mode_starts_disabled(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"
            data = DRIVER.contract()
            data["optimized_enabled"] = False
            path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path):
                with self.assertRaisesRegex(AssertionError, "Optimized diagnostic disabled"):
                    DRIVER.contract("optimized")

    def run_stage(self, cpu, other_failure=False):
        data = DRIVER.contract()
        test = data["tests"][-1]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            out, target = root / "evidence", root / "target"
            out.mkdir(); target.mkdir()
            binary = target / "test-executable"
            binary.write_bytes(b"same immutable executable")
            for name in [*data["fixture_paths"], "chat-protocol/Cargo.lock"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            DRIVER.save(out / "preflight.json", {"inputs": data, "inputs_sha256": DRIVER.sha(DRIVER.INPUTS),
                "mode": "optimized", "deadline_unix": time.time() + 1500})
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            failed = cpu > 5
            summary = "FAILED. 0 passed; 1 failed" if failed else "ok. 1 passed; 0 failed"
            panic = "storage failure" if other_failure else data["before_observation_failure_marker"]
            output = f"running 1 test\ntest {test['name']} ... whole_retry_cpu {{}}\n"
            output += f"thread 'test' panicked at source.rs:1: {panic}\n" if failed else "ok\n"
            output += f"test result: {summary}; 0 ignored; 100 filtered out;\n"

            def git(_root, *args):
                return {("status", "--porcelain"): "", ("rev-parse", "HEAD"): data["after"],
                    ("rev-parse", "HEAD^"): data["before"], ("rev-parse", "HEAD^{tree}"): "tree",
                    ("rev-parse", "HEAD:chat-protocol"): "protocol-tree",
                    ("diff", "--name-only", "HEAD^", "HEAD"): "\n".join(data["after_changed_paths"])}[args]

            def bounded(command, _root, _out, name, timeout, deadline, _target):
                self.assertEqual(_target, target)
                if name == "fetch":
                    self.assertIn("--locked", command)
                    return {"exit_code": 0}, ""
                if name == "build":
                    self.assertTrue(all(flag in command for flag in ("--release", "--locked", "--offline")))
                    return {"exit_code": 0}, json.dumps(artifact)
                self.assertEqual(name, "test-0")
                self.assertEqual(timeout, 300)
                self.assertEqual(command, [str(binary), test["name"], "--nocapture", "--test-threads=1", "--exact", "--ignored"])
                return {"exit_code": 101 if failed else 0}, output

            with mock.patch.object(DRIVER, "git", git), mock.patch.object(DRIVER, "bounded", bounded), \
                 mock.patch.object(DRIVER.subprocess, "check_output", side_effect=lambda cmd, **_: f"{cmd[0]} 1.99.1\n".encode()), \
                 mock.patch.dict(DRIVER.os.environ, {"CARGO_TARGET_DIR": str(target)}), \
                 mock.patch.object(DRIVER, "whole_retry_metrics", return_value={"cpu_percent_one_core": cpu}), \
                 mock.patch.dict(sys.modules, {"inbound_retry_perf": PERF}), \
                 mock.patch.object(PERF, "sample_failure", return_value={"status": "skipped", "reason": "no perf"}) as sample:
                if failed:
                    with self.assertRaisesRegex(AssertionError, "not the intended" if other_failure else "unchanged 5% limit"):
                        DRIVER.stage("optimized", root, out, data)
                else:
                    DRIVER.stage("optimized", root, out, data)
                self.assertEqual(sample.call_count, int(failed and not other_failure))
            if not other_failure:
                receipt = json.loads((out / "optimized/receipt.json").read_text())
                self.assertEqual(receipt["accepted"], not failed)
                self.assertEqual(len(receipt["tests"]), 1)
                self.assertEqual(receipt["tests"][0]["counts"]["failed"], int(failed))
                self.assertFalse(receipt["physical_cpu_acceptance"])

    def test_release_pass_runs_exactly_one_test_without_sampling(self):
        self.run_stage(4.0)

    def test_exact_cpu_failure_is_retained_before_optional_sampling(self):
        self.run_stage(80.0)

    def test_other_failure_cannot_trigger_sampling(self):
        self.run_stage(80.0, other_failure=True)


class PerfTests(unittest.TestCase):
    def run_profile(self, installed=True, permitted=True, unwind=True):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "test-binary"
            binary.write_bytes(b"same executable")
            calls = []

            def bounded(command, _root, _out, name, timeout, deadline, target):
                calls.append((name, command))
                if name == "perf-build-options":
                    return {"exit_code": 0}, f"libdw-dwarf-unwind: [ {'on' if unwind else 'OFF'} ]"
                if name == "perf-record-help":
                    return {"exit_code": 129}, "--call-graph --clockid --max-size --no-buildid-cache"
                if name in ("perf-permission-probe", "perf-sampled-test"):
                    self.assertEqual(command[command.index("--") + 1], str(binary) if name == "perf-sampled-test" else sys.executable)
                    self.assertIn("cpu-clock:u", command)
                    self.assertNotIn("-a", command)
                    if not permitted:
                        return {"exit_code": 255}, "Permission denied"
                    Path(command[command.index("-o") + 1]).write_bytes(b"synthetic perf data")
                    return {"exit_code": 101 if name == "perf-sampled-test" else 0}, ""
                self.assertEqual(name, "perf-report")
                self.assertIn("--stats", command)
                return {"exit_code": 0}, "samples and lost-event counts"

            with mock.patch.object(PERF.shutil, "which", return_value="/usr/bin/perf" if installed else None):
                result = PERF.sample_failure([str(binary), "exact-test"], root, root, time.time() + 1500, root, bounded, DRIVER.sha)
            self.assertFalse(result["cpu_acceptance"])
            self.assertTrue(result["no_system_settings_changed"])
            return result, [name for name, _ in calls]

    def test_missing_perf_or_unwind_support_skips_without_workload(self):
        for options in ({"installed": False}, {"unwind": False}):
            result, calls = self.run_profile(**options)
            self.assertEqual(result["status"], "skipped")
            self.assertNotIn("perf-sampled-test", calls)

    def test_denied_permission_skips_without_workload(self):
        result, calls = self.run_profile(permitted=False)
        self.assertEqual(result["status"], "skipped")
        self.assertIn("permission probe failed", result["reason"])
        self.assertNotIn("perf-sampled-test", calls)

    def test_one_process_scoped_sample_uses_same_binary_and_retains_report(self):
        result, calls = self.run_profile()
        self.assertEqual(result["status"], "sampled")
        self.assertEqual(calls.count("perf-sampled-test"), 1)
        self.assertEqual(result["run"]["exit_code"], 101)
        self.assertIn("data_sha256", result)
        self.assertIn("perf-report", calls)


if __name__ == "__main__":
    unittest.main()
