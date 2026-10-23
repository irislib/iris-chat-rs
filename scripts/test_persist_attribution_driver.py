#!/usr/bin/env python3
"""Offline attribution validation and one-build/one-test sequencing; no Cargo."""
import copy
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_persist_attribution as DRIVER
import test_checkpoint_update_driver as CHECKPOINT
import test_inbound_retry_driver as COMMON


def inputs():
    data = json.loads(DRIVER.INPUTS.read_text())
    data["enabled"] = True
    # Stand-in is restricted to offline fixtures until exact source is frozen.
    data["after"] = data["after"] or "a" * 40
    data["batched"]["source"] = data["after"]
    return data


def fixture(cpu=4.0):
    result = CHECKPOINT.v3_receipt(cpu)
    total = 0
    for index, row in enumerate(result["slices"]):
        writes = [{"elapsed_seconds": index * 60 + i, "batch_depth": 0,
            "caller_file": "engine_persistence.rs", "caller_line": 83,
            "requests": [{"file": "engine_retry.rs", "line": 12, "count": 2}]}
            for i in range(row["puts"])]
        total += len(writes)
        row["persistence_trace"] = {"schema": DRIVER.TRACE_SCHEMA, "limit": 64, "overflow": False,
            "successful_puts": len(writes), "total_successful_puts": total, "writes": writes}
    return result


def output(test, metrics, failure=None):
    text = f"running 1 test\ntest {test['name']} ... \n" + COMMON.BatchedMetricsTests().render(metrics) + "\n"
    if failure: text += f"thread 'test' panicked at source.rs:1: {failure}\n"
    state = "FAILED. 0 passed; 1 failed" if failure else "ok. 1 passed; 0 failed"
    return text + f"test result: {state}; 0 ignored; 100 filtered out;\n"


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"; path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path): return DRIVER.contract()

    def test_disabled_or_unbound_source_cannot_admit_run(self):
        for changes in ({"enabled": False}, {"after": None}):
            with self.subTest(changes=changes), self.assertRaises(AssertionError): self.verify(dict(inputs(), **changes))

    def test_exact_single_cpu_and_no_correctness_repeat(self):
        data = self.verify(inputs())
        self.assertEqual(data["batched"]["correctness_tests"], [])
        self.assertEqual(data["batched"]["test"], DRIVER.checkpoint.cpu_plan()["test"])

    def test_changed_source_scope_limits_or_acceptance_claim_are_rejected(self):
        changes = [lambda d: d.update(base="b" * 40), lambda d: d.update(release_cpu_acceptance=True),
            lambda d: d.update(diagnostic_only=False), lambda d: d["batched"]["changed_paths"].append("core/src/lib.rs"),
            lambda d: d["batched"]["correctness_tests"].append(DRIVER.checkpoint.spec("other")),
            lambda d: d["batched"]["test"].update(timeout_seconds=600), lambda d: d["batched"].update(schema_version=2),
            lambda d: d["trace"].update(limit=65), lambda d: d["trace"].update(overflow_allowed=True),
            lambda d: d.update(protocol_lock_sha256="0" * 64)]
        for change in changes:
            data = inputs(); change(data)
            with self.subTest(change=change), self.assertRaises(AssertionError): self.verify(data)


class TraceTests(unittest.TestCase):
    def test_complete_trace_matches_all_five_slice_puts(self):
        result = fixture(); trace = DRIVER.traces(result)
        self.assertEqual(trace["successful_puts"], sum(row["puts"] for row in result["slices"]))
        self.assertEqual(len(trace["slices"]), 5)

    def test_direct_persist_now_caller_may_have_no_requests(self):
        result = fixture(); result["slices"][0]["persistence_trace"]["writes"][0]["requests"] = []
        DRIVER.traces(result)

    def test_rejects_overflow_missing_counts_invalid_locations_and_nonfinite_time(self):
        changes = [lambda t: t.update(overflow=True), lambda t: t.update(limit=65), lambda t: t.update(successful_puts=0),
            lambda t: t.update(total_successful_puts=0), lambda t: t.update(total_successful_puts=True),
            lambda t: t["writes"].pop(), lambda t: t.update(unexpected=1),
            lambda t: t["writes"][0].update(elapsed_seconds=float("nan")),
            lambda t: t["writes"][0].update(elapsed_seconds=-1), lambda t: t["writes"][0].update(batch_depth=-1),
            lambda t: t["writes"][0].update(caller_file="/private/engine.rs"),
            lambda t: t["writes"][0].update(caller_file="..\\engine.rs"),
            lambda t: t["writes"][0].update(caller_line=0),
            lambda t: t["writes"][0]["requests"][0].update(count=0),
            lambda t: t["writes"][0]["requests"][0].update(file="private/engine.rs"),
            lambda t: t["writes"][0]["requests"].append(t["writes"][0]["requests"][0].copy())]
        for change in changes:
            data = fixture(); change(data["slices"][0]["persistence_trace"])
            with self.subTest(change=change), self.assertRaises(AssertionError): DRIVER.traces(data)

    def test_limit_is_global_and_elapsed_time_cannot_go_backwards(self):
        data = fixture(); original = copy.deepcopy(data["slices"][0]["persistence_trace"]["writes"][0])
        trace = data["slices"][0]["persistence_trace"]
        trace["writes"] = [dict(original, elapsed_seconds=i / 100) for i in range(64)]
        trace["successful_puts"] = trace["total_successful_puts"] = data["slices"][0]["puts"] = 64
        total = 64
        for row in data["slices"][1:]:
            total += row["puts"]; row["persistence_trace"]["total_successful_puts"] = total
        with self.assertRaises(AssertionError): DRIVER.traces(data)
        data = fixture(); data["slices"][1]["persistence_trace"]["writes"][0]["elapsed_seconds"] = -0.1
        with self.assertRaises(AssertionError): DRIVER.traces(data)

    def test_result_parser_keeps_v3_workload_and_rejects_a_different_failure(self):
        data = inputs(); test = data["batched"]["test"]
        DRIVER.check_result(output(test, fixture()), test, 0)
        bad = fixture(); bad["acknowledged_setup_deliveries"] = 0
        with self.assertRaises(AssertionError): DRIVER.check_result(output(test, bad), test, 0)
        with self.assertRaises(AssertionError): DRIVER.check_result(output(test, fixture(), "storage failed"), test, 101, data["batched"]["failure_marker"])


class StageTests(unittest.TestCase):
    def exercise(self, *, cpu=4.0, fetch_failure=False, compile_failure=False, trace_failure=False):
        data, retry = inputs(), DRIVER.retry
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); out = root / "evidence"; target = root / "target"
            out.mkdir(); target.mkdir(); binary = target / "only-binary"; binary.write_bytes(b"fixed compiled binary")
            retry.save(out / "preflight.json", {"inputs": data, "inputs_sha256": retry.sha(DRIVER.INPUTS), "mode": "batched", "deadline_unix": time.time() + 1500})
            source = {"commit": data["after"], "diagnostic_only": True, "release_cpu_acceptance": False}
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            commands = []
            def bounded(command, _root, _out, name, timeout, deadline, _target):
                commands.append((name, command)); self.assertEqual(target, _target); self.assertGreater(deadline, time.time())
                if name == "fetch":
                    self.assertIn("--locked", command); self.assertEqual(timeout, 300)
                    return {"exit_code": 101 if fetch_failure else 0}, "fetch"
                if name == "build":
                    self.assertTrue({"--release", "--locked", "--offline", "--lib", "--no-run"}.issubset(command))
                    self.assertEqual(command[-3:-1], ["--jobs", "2"]); self.assertEqual(timeout, 600)
                    return {"exit_code": 101 if compile_failure else 0}, json.dumps(artifact)
                self.assertEqual(name, "test-0"); self.assertEqual(timeout, 420)
                self.assertEqual(command, [str(binary), data["batched"]["test"]["name"], "--nocapture", "--test-threads=1", "--exact", "--ignored"])
                metrics = fixture(cpu)
                if trace_failure: metrics["slices"][0]["persistence_trace"]["overflow"] = True
                return {"exit_code": 101 if cpu > 5 else 0}, output(data["batched"]["test"], metrics, data["batched"]["failure_marker"] if cpu > 5 else None)
            with mock.patch.object(retry, "bounded", bounded), mock.patch.object(DRIVER, "source_proof", return_value=source), \
                mock.patch.object(retry, "git", side_effect=lambda _, *args: data["after"] if args == ("rev-parse", "HEAD") else ""), \
                mock.patch.object(retry.subprocess, "check_output", side_effect=lambda cmd, **kw: f"{cmd[0]} 1.99.1 (test)\n".encode()), \
                mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(target)), \
                mock.patch.object(retry, "whole_retry_metrics", side_effect=AssertionError("No standalone benchmark")):
                if any((fetch_failure, compile_failure, trace_failure, cpu > 5)):
                    with self.assertRaises(AssertionError): DRIVER.run("batched", root, out, data)
                else: DRIVER.run("batched", root, out, data)
            self.assertEqual(sum(name == "fetch" for name, _ in commands), 1)
            self.assertEqual(sum(name == "build" for name, _ in commands), int(not fetch_failure))
            self.assertEqual(sum(name.startswith("test-") for name, _ in commands), int(not (fetch_failure or compile_failure)))
            path = out / "attribution.json"
            self.assertEqual(path.exists(), not (fetch_failure or compile_failure or trace_failure))
            if path.exists():
                result = json.loads(path.read_text())
                self.assertTrue(result["diagnostic_complete"] and result["diagnostic_only"])
                self.assertFalse(result["release_cpu_acceptance"])
                self.assertEqual(result["instrumented_cpu_budget_passed"], cpu <= 5)
                self.assertEqual(result["binary_sha256"], retry.sha(binary))
            self.assertFalse(list(out.rglob("sampling.json")))

    def test_one_fetch_build_and_test_is_diagnostic_even_when_cpu_passes(self): self.exercise()

    def test_complete_cpu_failure_preserves_attribution_without_retry(self): self.exercise(cpu=6.0)

    def test_fetch_and_build_failure_block_test(self):
        self.exercise(fetch_failure=True); self.exercise(compile_failure=True)

    def test_trace_failure_cannot_produce_complete_attribution(self): self.exercise(trace_failure=True)


if __name__ == "__main__": unittest.main()
