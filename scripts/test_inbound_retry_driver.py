#!/usr/bin/env python3
"""Fail-closed checks for the temporary hosted regression driver; no Cargo."""
import importlib.util
import copy
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

    def run_stage(self, cpu, other_failure=False, batched=False, correctness_failure=False):
        data = DRIVER.contract()
        if batched:
            data["batched"]["correctness_tests"] = [
                {"name": "protocol_engine::tests::group_retry_", "expected_count": 2,
                 "exact": False, "marker": None, "ignored": False, "timeout_seconds": 300},
                {"name": "protocol_engine::tests::large_ready_queue", "expected_count": 1,
                 "exact": True, "marker": "ready_queue count=23437", "ignored": False, "timeout_seconds": 300}]
        test = data["batched"]["test"] if batched else data["tests"][-1]
        correctness = data["batched"]["correctness_tests"] if batched else []
        which = "batched" if batched else "optimized"
        source = data["batched"]["source"] if batched else data["after"]
        parent = data["batched"]["base"] if batched else data["before"]
        changed = data["batched"]["changed_paths"] if batched else data["after_changed_paths"]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            out, target = root / "evidence", root / "target"
            out.mkdir(); target.mkdir()
            binary = target / "test-executable"
            binary.write_bytes(b"same immutable executable")
            for name in [*data["fixture_paths"], *data["batched"]["changed_paths"], "chat-protocol/Cargo.lock"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            DRIVER.save(out / "preflight.json", {"inputs": data, "inputs_sha256": DRIVER.sha(DRIVER.INPUTS),
                "mode": which, "deadline_unix": time.time() + 1500})
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            failed = cpu > 5
            summary = "FAILED. 0 passed; 1 failed" if failed else "ok. 1 passed; 0 failed"
            panic = "storage failure" if other_failure else data["batched"]["failure_marker"] if batched else data["before_observation_failure_marker"]
            output = f"running 1 test\ntest {test['name']} ... {test['marker']}{{}}\n"
            output += f"thread 'test' panicked at source.rs:1: {panic}\n" if failed else "ok\n"
            output += f"test result: {summary}; 0 ignored; 100 filtered out;\n"

            def git(_root, *args):
                return {("status", "--porcelain"): "", ("rev-parse", "HEAD"): source,
                    ("rev-parse", "HEAD^"): parent, ("rev-parse", "HEAD^{tree}"): "tree",
                    ("rev-parse", "HEAD:chat-protocol"): "protocol-tree",
                    ("diff", "--name-only", "HEAD^", "HEAD"): "\n".join(changed)}[args]

            executed = []

            def bounded(command, _root, _out, name, timeout, deadline, _target):
                self.assertEqual(_target, target)
                if name == "fetch":
                    self.assertIn("--locked", command)
                    return {"exit_code": 0}, ""
                if name == "build":
                    self.assertTrue(all(flag in command for flag in ("--release", "--locked", "--offline")))
                    return {"exit_code": 0}, json.dumps(artifact)
                index = len(executed)
                self.assertEqual(name, f"test-{index}")
                selected = correctness[index] if index < len(correctness) else test
                expected_command = [str(binary), selected["name"], "--nocapture", "--test-threads=1"]
                if selected["exact"]:
                    expected_command.append("--exact")
                if selected["ignored"]:
                    expected_command.append("--ignored")
                self.assertEqual(command, expected_command)
                executed.append(selected["name"])
                if index < len(correctness):
                    self.assertEqual(timeout, 300)
                    count = selected["expected_count"]
                    failed_count = 1 if correctness_failure else 0
                    status = "FAILED" if correctness_failure else "ok"
                    log = f"running {count} tests\ntest {selected['name']} ... {selected['marker'] or ''}\n"
                    log += f"test result: {status}. {count - failed_count} passed; {failed_count} failed; 0 ignored; 100 filtered out;\n"
                    return {"exit_code": 101 if correctness_failure else 0}, log
                self.assertEqual(timeout, 420 if batched else 300)
                return {"exit_code": 101 if failed else 0}, output

            with mock.patch.object(DRIVER, "git", git), mock.patch.object(DRIVER, "bounded", bounded), \
                 mock.patch.object(DRIVER.subprocess, "check_output", side_effect=lambda cmd, **_: f"{cmd[0]} 1.99.1\n".encode()), \
                 mock.patch.dict(DRIVER.os.environ, {"CARGO_TARGET_DIR": str(target)}), \
                 mock.patch.object(DRIVER, "whole_retry_metrics", return_value={"cpu_percent_one_core": cpu}), \
                 mock.patch.object(DRIVER, "batched_retry_metrics", return_value={"slices": [{"cpu_percent_one_core": cpu}] * 5}), \
                 mock.patch.dict(sys.modules, {"inbound_retry_perf": PERF}), \
                 mock.patch.object(PERF, "sample_failure", return_value={"status": "skipped", "reason": "no perf"}) as sample:
                if correctness_failure:
                    with self.assertRaisesRegex(AssertionError, "Exact test count/result mismatch"):
                        DRIVER.stage(which, root, out, data)
                elif failed:
                    with self.assertRaisesRegex(AssertionError, "not the intended" if other_failure else "unchanged 5% limit"):
                        DRIVER.stage(which, root, out, data)
                else:
                    DRIVER.stage(which, root, out, data)
                self.assertEqual(sample.call_count, int(failed and not other_failure and not batched and not correctness_failure))
            if correctness_failure:
                self.assertEqual(executed, [correctness[0]["name"]])
                self.assertFalse((out / which / "receipt.json").exists())
                results = json.loads((out / which / "test-results.json").read_text())
                self.assertEqual(len(results), 1)
                self.assertFalse(results[0]["accepted"])
            elif not other_failure:
                receipt = json.loads((out / which / "receipt.json").read_text())
                self.assertEqual(receipt["accepted"], not failed)
                self.assertEqual(executed, [t["name"] for t in [*correctness, test]])
                self.assertEqual(len(receipt["tests"]), len(correctness) + 1)
                self.assertTrue(all(t["accepted"] for t in receipt["tests"][:-1]))
                self.assertEqual(receipt["tests"][-1]["counts"]["failed"], int(failed))
                self.assertFalse(receipt["physical_cpu_acceptance"])

    def test_release_pass_runs_exactly_one_test_without_sampling(self):
        self.run_stage(4.0)

    def test_exact_cpu_failure_is_retained_before_optional_sampling(self):
        self.run_stage(80.0)

    def test_other_failure_cannot_trigger_sampling(self):
        self.run_stage(80.0, other_failure=True)

    def test_batched_pass_runs_correctness_before_one_420_second_cpu_test(self):
        self.run_stage(4.0, batched=True)

    def test_batched_cpu_failure_never_runs_a_profile_or_baseline(self):
        self.run_stage(80.0, batched=True)

    def test_batched_correctness_failure_stops_before_cpu_or_sampling(self):
        self.run_stage(4.0, batched=True, correctness_failure=True)

    def test_batched_unrelated_failure_is_not_classified_as_cpu_failure(self):
        self.run_stage(80.0, other_failure=True, batched=True)


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


class BatchedMetricsTests(unittest.TestCase):
    def fixture(self):
        result = {"schema": "iris-batched-retry-cpu/v1", "fixture": "restored-mixed-backlog-v1", "synthetic": True,
            "pending_inbound": 4214, "initially_due_inbound": 102, "fanouts": 24990, "initially_due_fanouts": 10462,
            "ciphertexts": 23437, "remote_sends": 44, "checkpoint_bytes": 350224384, "outer_batch": True,
            "ack_and_flush_in_turn": True, "continuation_delay_ms": 25, "liveness_delay_ms": 2000,
            "settle_seconds": 180, "measurement_windows": 2, "window_seconds": 60, "debug_assertions": False,
            "initially_ready": True, "group_deliveries": 1, "acknowledged_deliveries": 2,
            "acknowledged_setup_deliveries": 1, "acknowledged_group_deliveries": 1, "repeated_search_starts": 0,
            "wire_effect_count": 0, "remaining_inbound": 4214, "remaining_ciphertexts": 23436,
            "remaining_remote_sends": 44, "remaining_fanouts": 24991, "preservation_checked": True,
            "restart_checked": True, "restart_ready": True, "restore_seconds": 1.0, "readiness_seconds": 1.1,
            "first_valid_delivery_seconds": 2.0, "search_starts": 30, "input_generation_changes": 1, "slices": []}
        progress = {"search_starts": 0, "repeated_search_starts": 0, "input_generation_changes": 1,
            "ready_candidates": 10, "engine_attempts": 0, "active_cursor": False, "prepared_result": False}
        result["restart_progress"] = dict(progress)
        for index in range(5):
            before = dict(progress)
            if index < 3:
                progress["search_starts"] += 10
                progress["engine_attempts"] += 100
            cpu = 30.0 if index < 3 else 4.0
            work = cpu * 0.6
            phases = {key: work * 0.01 for key in ("retirement", "group_fanouts", "inbound", "remote_sends", "local_siblings")}
            phases.update(group_inputs=work * 0.5, flush=work * 0.3, decrypt_cursor_nested=work * 0.25)
            result["slices"].append({"stage": "settle" if index < 3 else "measure", "stage_index": index if index < 3 else index - 3,
                "elapsed_seconds": 60.0, "recovery_elapsed_seconds": 60.0 * (index + 1), "cpu_seconds": cpu * 0.6,
                "cpu_percent_one_core": cpu, "work_seconds": work, "phase_work_seconds": phases, "max_turn_seconds": 1.0,
                "turns": 20, "continuations": 20, "acknowledged_deliveries": 2 * int(index == 0), "puts": 1,
                "read_bytes": 350224384, "written_bytes": 4096, "remote_deadline_updates": 1,
                "due_at_start": {"inbound": 0, "fanouts": 0, "remote_sends": 1},
                "progress_before": before, "progress_after": dict(progress)})
        return result

    def render(self, result):
        return "\n".join("batched_retry_slice " + json.dumps(row) for row in result["slices"]) + "\nbatched_retry_cpu " + json.dumps(result)

    def test_complete_slices_and_high_cpu_are_preserved_for_result_classification(self):
        result = self.fixture()
        self.assertEqual(DRIVER.batched_retry_metrics(self.render(result)), result)
        result["slices"][-1].update(cpu_seconds=48.0, cpu_percent_one_core=80.0)
        self.assertEqual(DRIVER.batched_retry_metrics(self.render(result))["slices"][-1]["cpu_percent_one_core"], 80.0)

    def test_missing_misordered_or_unemitted_window_is_rejected(self):
        result = self.fixture()
        cases = [self.render(result).replace("batched_retry_slice ", "unemitted ", 1)]
        missing = copy.deepcopy(result); missing["slices"].pop()
        cases.append(self.render(missing))
        misordered = copy.deepcopy(result); misordered["slices"][-1]["stage_index"] = 0
        cases.append(self.render(misordered))
        for text in cases:
            with self.assertRaises(AssertionError):
                DRIVER.batched_retry_metrics(text)

    def test_invalid_phase_fields_times_and_nested_accounting_are_rejected(self):
        mutations = [lambda p: p.update(unexpected=0.0), lambda p: p.pop("retirement"),
            lambda p: p.update(inbound=-0.1), lambda p: p.update(inbound=float("inf")),
            lambda p: p.update(inbound=True), lambda p: p.update(flush=100.0),
            lambda p: p.update(decrypt_cursor_nested=p["group_inputs"] + 0.1)]
        for mutate in mutations:
            result = self.fixture()
            mutate(result["slices"][0]["phase_work_seconds"])
            with self.assertRaises(AssertionError):
                DRIVER.batched_retry_metrics(self.render(result))

    def test_false_progress_repeated_admission_and_quiet_gap_do_not_pass(self):
        mutations = [lambda r: r.update(first_valid_delivery_seconds=181.0),
            lambda r: r.update(restart_checked=False), lambda r: r.update(acknowledged_deliveries=0),
            lambda r: r.update(acknowledged_setup_deliveries=0),
            lambda r: r.update(acknowledged_group_deliveries=0),
            lambda r: r["slices"][0].update(acknowledged_deliveries=1),
            lambda r: r["slices"][-1].update(puts=0), lambda r: r["slices"][-1].update(remote_deadline_updates=0),
            lambda r: r["slices"][2]["progress_before"].update(search_starts=0),
            lambda r: r["slices"][2]["progress_after"].update(repeated_search_starts=1)]
        for mutate in mutations:
            result = self.fixture(); mutate(result)
            with self.assertRaises(AssertionError):
                DRIVER.batched_retry_metrics(self.render(result))

    def test_batched_contract_is_disabled_without_affecting_old_bounds(self):
        data = DRIVER.contract()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"
            data["batched"]["enabled"] = False
            path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path):
                with self.assertRaisesRegex(AssertionError, "Batched diagnostic disabled"):
                    DRIVER.contract("batched")
            self.assertTrue(all(test["timeout_seconds"] <= 300 for test in data["tests"]))
            self.assertEqual(data["batched"]["test"]["timeout_seconds"], 420)

    def test_batched_contract_admits_reviewed_parent_and_bounded_correctness(self):
        data = DRIVER.contract()
        data["batched"]["enabled"] = True
        data["batched"]["base"] = "a" * 40
        data["batched"]["correctness_tests"] = [dict(data["tests"][3])]
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"
            path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path):
                admitted = DRIVER.contract("batched")
                self.assertNotEqual(admitted["batched"]["base"], admitted["after"])
                self.assertEqual(DRIVER.contract("optimized")["tests"], data["tests"])
                mutations = [lambda d: d["batched"].update(correctness_tests=[]),
                    lambda d: d["batched"]["correctness_tests"][0].update(timeout_seconds=420),
                    lambda d: d["batched"]["correctness_tests"][0].update(ignored=True),
                    lambda d: d["batched"]["correctness_tests"][0].update(expected_count=0),
                    lambda d: d["batched"]["correctness_tests"][0].update(name=d["batched"]["test"]["name"])]
                for mutate in mutations:
                    changed = copy.deepcopy(data)
                    mutate(changed)
                    path.write_text(json.dumps(changed))
                    with self.assertRaises(AssertionError):
                        DRIVER.contract("batched")


if __name__ == "__main__":
    unittest.main()
