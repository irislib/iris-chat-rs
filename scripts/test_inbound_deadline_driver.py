#!/usr/bin/env python3
"""Offline deadline comparison admission/result checks; never invokes Cargo."""
import copy
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_inbound_deadline as DRIVER


def inputs():
    data = json.loads(DRIVER.INPUTS.read_text())
    data["enabled"] = True
    return data


def output(test, failed=False, scans=265483):
    count = test["expected_count"]
    text = f"running {count} tests\ntest {test['name']} ... {test['marker'] or ''}\n"
    if failed:
        text += f"thread 'test' panicked at source.rs:1: 32 continuations inspected {scans} inbound retry deadlines\nFAILED\n"
        summary = "FAILED. 0 passed; 1 failed"
    else:
        text += "ok\n"
        summary = f"ok. {count} passed; 0 failed"
    return text + f"test result: {summary}; 0 ignored; 100 filtered out;\n"


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"
            path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path):
                return DRIVER.contract()

    def test_disabled_blocks_admission(self):
        with self.assertRaisesRegex(AssertionError, "disabled"):
            self.verify(dict(inputs(), enabled=False))

    def test_enabled_selection_is_twenty_one_without_observation(self):
        data = self.verify(inputs())
        self.assertEqual(sum(t["expected_count"] for t in data["tests"]), 21)
        self.assertIsNone(data["before_observation_test"])

    def test_source_paths_counts_and_observation_fail_closed(self):
        changes = [lambda d: d.update(after=d["before"]), lambda d: d.update(before="main"),
            lambda d: d["fixture_paths"].append("../other"),
            lambda d: d.update(before_observation_test=d["before_test"]),
            lambda d: d["tests"][0].update(timeout_seconds=420),
            lambda d: d["tests"][1].update(expected_count=5),
            lambda d: d["tests"][0].update(ignored=True)]
        for change in changes:
            data = inputs()
            change(data)
            with self.subTest(data=data), self.assertRaises(AssertionError):
                self.verify(data)


class ResultTests(unittest.TestCase):
    def setUp(self):
        self.test = inputs()["tests"][0]

    def test_numeric_upper_bound_failure_is_red(self):
        counts = DRIVER.check_result(output(self.test, True), self.test, 101, DRIVER.FAILURE)
        self.assertTrue(counts["expected_regression_failure"])
        self.assertEqual(counts["deadline_scans"], 265483)
        self.assertEqual(counts["deadline_scan_guard"]["maximum"], 12642)

    def test_compile_zero_tests_wrong_panic_and_low_scan_count_are_not_red(self):
        red = output(self.test, True)
        cases = ["error: could not compile iris-chat-protocol", red.replace("1 failed", "0 failed"),
            red.replace("inbound retry deadlines", "unrelated failure"),
            output(self.test, True, 1), output(self.test, True, 12642), red + red]
        for text in cases:
            with self.subTest(text=text), self.assertRaises(AssertionError):
                DRIVER.check_result(text, self.test, 101, DRIVER.FAILURE)

    def test_green_does_not_invent_scan_measurement(self):
        counts = DRIVER.check_result(output(self.test), self.test, 0)
        self.assertEqual(counts["passed"], 1)
        self.assertIsNone(counts["deadline_scans"])
        self.assertEqual(counts["deadline_scan_guard"]["continuations"], 32)

    def test_unexpected_red_pass_or_killed_process_is_rejected(self):
        for text, code in [(output(self.test), 0), (output(self.test, True), -9)]:
            with self.subTest(code=code), self.assertRaises(AssertionError):
                DRIVER.check_result(text, self.test, code, DRIVER.FAILURE)

    def test_green_prefix_requires_four_actual_passes(self):
        test = inputs()["tests"][1]
        self.assertEqual(DRIVER.check_result(output(test), test, 0)["passed"], 4)
        with self.assertRaises(AssertionError):
            DRIVER.check_result(output(test).replace("4 passed", "3 passed"), test, 0)


class StageTests(unittest.TestCase):
    def run_stage(self, which, bad_red=False):
        data, retry = inputs(), DRIVER.retry
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            out, target = root / "evidence", root / "target"
            out.mkdir(); target.mkdir()
            binary = target / "protocol-test"
            binary.write_bytes(b"exact same compiled binary for selected tests")
            for name in [*data["fixture_paths"], "chat-protocol/Cargo.lock"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            pre = {"inputs": data, "inputs_sha256": retry.sha(DRIVER.INPUTS), "mode": "comparison", "deadline_unix": time.time() + 1500}
            retry.save(out / "preflight.json", pre)
            versions = {tool: f"{tool} 1.99.1 (test)\n" for tool in ("cargo", "rustc")}
            if which == "after":
                before = out / "before"
                before.mkdir()
                retry.save(before / "receipt.json", {"accepted": True})
                retry.save(before / "source.json", {"fixtures": {p: retry.sha(root / p) for p in data["fixture_paths"]},
                    "protocol_lock_sha256": retry.sha(root / "chat-protocol/Cargo.lock")})
                retry.save(before / "toolchain.json", versions)
            facts = {("status", "--porcelain"): "", ("rev-parse", "HEAD"): data[which],
                ("rev-parse", "HEAD^"): data["base"] if which == "before" else data["before"],
                ("rev-parse", "HEAD^{tree}"): "tree", ("rev-parse", "HEAD:chat-protocol"): "protocol-tree",
                ("diff", "--name-only", "HEAD^", "HEAD"): "\n".join(data[which + "_changed_paths"])}
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            selected = data["tests"] if which == "after" else data["tests"][:1]
            commands = []

            def bounded(command, _root, _out, name, timeout, deadline, _target):
                commands.append((name, command))
                self.assertEqual(_target, target)
                self.assertGreater(deadline, time.time())
                if name == "fetch":
                    self.assertEqual(which, "before")
                    self.assertIn("--locked", command)
                    return {"exit_code": 0}, "fetched"
                if name == "build":
                    self.assertTrue({"--release", "--locked", "--offline", "--lib", "--no-run"}.issubset(command))
                    self.assertEqual(timeout, 600)
                    return {"exit_code": 0}, json.dumps(artifact)
                index = int(name.split("-")[1])
                self.assertEqual(timeout, 300)
                self.assertEqual(command[0], str(binary))
                return {"exit_code": 101 if which == "before" else 0}, output(selected[index], which == "before", 1 if bad_red else 265483)

            with mock.patch.object(retry, "INPUTS", DRIVER.INPUTS), mock.patch.object(retry, "bounded", bounded), \
                    mock.patch.object(retry, "git", side_effect=lambda _, *args: facts[args]), \
                    mock.patch.object(retry.subprocess, "check_output", side_effect=lambda cmd, **kw: versions[cmd[0]].encode()), \
                    mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(target)), \
                    mock.patch.object(retry, "whole_retry_metrics", side_effect=AssertionError("CPU observation forbidden")):
                if bad_red:
                    with self.assertRaisesRegex(AssertionError, "upper scan limit"):
                        DRIVER.run(which, root, out, data)
                    self.assertFalse((out / which / "receipt.json").exists())
                    checks = json.loads((out / which / "test-results.json").read_text())
                    self.assertFalse(checks[0]["accepted"])
                else:
                    DRIVER.run(which, root, out, data)
                    receipt = json.loads((out / which / "receipt.json").read_text())
                    self.assertTrue(receipt["accepted"])
                    self.assertFalse(receipt["physical_cpu_acceptance"])
                    self.assertIn("no CPU measurement", receipt["cpu_interpretation"])
                    self.assertEqual(sum(t["counts"]["passed"] for t in receipt["tests"]), 21 if which == "after" else 0)
                self.assertEqual(len([name for name, _ in commands if name.startswith("test-")]), len(selected))
                self.assertEqual(len([name for name, _ in commands if name == "build"]), 1)

    def test_release_red_runs_only_scan_assertion(self):
        self.run_stage("before")

    def test_release_green_runs_twenty_one_checks_once_without_cpu(self):
        self.run_stage("after")

    def test_wrong_numeric_red_guard_preserves_failed_partial_result(self):
        self.run_stage("before", bad_red=True)


if __name__ == "__main__":
    unittest.main()
