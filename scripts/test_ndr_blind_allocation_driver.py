#!/usr/bin/env python3
"""Offline fail-closed tests for the temporary NDR driver. Never invokes Cargo."""
import copy
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_ndr_blind_allocation as DRIVER


def inputs():
    data = json.loads(DRIVER.INPUTS.read_text())
    data["enabled"] = True
    return data


def test_output(test, failed=False, allocations=8):
    count = test["expected_count"]
    output = f"running {count} tests\ntest {test['name'] or 'integration_case'} ... "
    if test["target"] == DRIVER.ALLOCATION_TARGET:
        output += f"blind_search_allocations attempts=10001 allocations={allocations} maximum=16\n"
    if failed:
        output += f"thread 'test' panicked at source.rs:1: blind-search bookkeeping allocated {allocations} times\nFAILED\n"
        summary = "FAILED. 0 passed; 1 failed"
    else:
        output += "ok\n"
        summary = f"ok. {count} passed; 0 failed"
    return output + f"test result: {summary}; 0 ignored; 100 filtered out;\n"


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "inputs.json"
            path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path):
                return DRIVER.contract()

    def test_committed_inputs_are_disabled(self):
        with self.assertRaisesRegex(AssertionError, "disabled"):
            DRIVER.contract()

    def test_exact_reviewed_contract_can_be_enabled(self):
        self.assertEqual(self.verify(inputs()), inputs())

    def test_preflight_records_bound_deadline_and_immutable_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary)
            github_output = out / "github-output"
            with mock.patch.object(DRIVER, "git", side_effect=["", "driver-sha"]), \
                    mock.patch.object(DRIVER.time, "time", return_value=1000), \
                    mock.patch.dict(os.environ, GITHUB_OUTPUT=str(github_output)):
                DRIVER.preflight(out, inputs())
            receipt = json.loads((out / "preflight.json").read_text())
            self.assertEqual(receipt["deadline_unix"], 2500)
            self.assertEqual(receipt["workflow_timeout_seconds"], 1800)
            self.assertEqual(receipt["inputs"], inputs())
            self.assertEqual(receipt["inputs_sha256"], DRIVER.sha(DRIVER.INPUTS))
            self.assertIn(f"after={inputs()['after']}\n", github_output.read_text())

    def test_source_repository_paths_and_selection_fail_closed(self):
        changes = [lambda d: d.update(repository="other/repo"),
            lambda d: d.update(after=d["before"]), lambda d: d.update(after="main"),
            lambda d: d["after_changed_paths"].append(DRIVER.ALLOCATION_PATH),
            lambda d: d["before_changed_paths"].append("../outside"),
            lambda d: d["tests"][0].update(expected_count=0),
            lambda d: d["tests"][1].update(name="other::"),
            lambda d: d["tests"][0].update(timeout_seconds=301),
            lambda d: d["tests"][0].update(ignored=True),
            lambda d: d["tests"][0].update(marker=None)]
        for change in changes:
            data = inputs()
            change(data)
            with self.subTest(data=data), self.assertRaises(AssertionError):
                self.verify(data)


class AllocationTests(unittest.TestCase):
    def setUp(self):
        self.test = inputs()["tests"][0]
        self.red = test_output(self.test, failed=True, allocations=10008)
        self.green = test_output(self.test)

    def test_actual_red_and_green(self):
        metrics, counts = DRIVER.allocation_result(self.red, self.test, 101, True)
        self.assertEqual(metrics, {"attempts": 10001, "allocations": 10008, "maximum": 16})
        self.assertTrue(counts["expected_regression_failure"])
        self.assertEqual(DRIVER.allocation_result(self.green, self.test, 0, False)[1]["passed"], 1)

    def test_compile_failure_zero_tests_and_unexpected_pass_are_not_red(self):
        for text, code in [("error: could not compile nostr-double-ratchet", 101),
                (self.red.replace("running 1 tests", "running 0 tests").replace("1 failed", "0 failed"), 101),
                (self.green, 0), (self.red, -9)]:
            with self.subTest(text=text), self.assertRaises(AssertionError):
                DRIVER.allocation_result(text, self.test, code, True)

    def test_wrong_assertion_or_metric_cannot_count_as_red(self):
        for text in [self.red.replace("bookkeeping allocated 10008 times", "state was mutated"),
                self.red.replace("bookkeeping allocated 10008 times", "bookkeeping allocated 99999 times"),
                self.red.replace("attempts=10001", "attempts=10000"),
                self.red.replace("maximum=16", "maximum=32"),
                self.red.replace("allocations=10008", "allocations=8"),
                self.red + "blind_search_allocations attempts=10001 allocations=10008 maximum=16\n",
                self.red.replace("blind_search_allocations", "missing_metric")]:
            with self.subTest(text=text), self.assertRaises(AssertionError):
                DRIVER.allocation_result(text, self.test, 101, True)

    def test_green_rejects_over_budget_failure_extra_results_and_wrong_count(self):
        for text, code in [(self.red, 101), (self.green, 101), (self.green + self.green, 0),
                (self.green.replace("1 passed", "2 passed"), 0),
                (self.green.replace("allocations=8", "allocations=17"), 0)]:
            with self.subTest(text=text), self.assertRaises(AssertionError):
                DRIVER.allocation_result(text, self.test, code, False)

    def test_boundary_sixteen_allocations_passes(self):
        DRIVER.allocation_result(self.green.replace("allocations=8", "allocations=16"), self.test, 0, False)


class SourceAndBuildTests(unittest.TestCase):
    def run_source_proof(self, mutation=None):
        data = inputs()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dependencies = ["rust/Cargo.lock", "rust/Cargo.toml", f"rust/crates/{DRIVER.PACKAGE}/Cargo.toml"]
            for name in [*data["before_changed_paths"], *dependencies]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            facts = {("rev-parse", "HEAD"): data["after"], ("status", "--porcelain"): "",
                ("rev-parse", "HEAD^"): data["before"], ("rev-parse", "HEAD^{tree}"): "tree",
                ("diff", "--name-only", "HEAD^", "HEAD"): "\n".join(data["after_changed_paths"]),
                ("ls-files", "rust/Cargo.lock", "rust/**/Cargo.toml", "rust/Cargo.toml"): "\n".join(dependencies)}
            if mutation:
                facts.update(mutation)
            with mock.patch.object(DRIVER, "git", side_effect=lambda _, *args: facts[args]):
                return DRIVER.source_proof("after", root, data)

    def test_source_proof_binds_exact_ancestry_files_fixture_and_dependencies(self):
        proof = self.run_source_proof()
        self.assertEqual(proof["parent"], inputs()["before"])
        self.assertEqual(proof["commit"], inputs()["after"])
        self.assertEqual(set(proof["changed_file_sha256"]), DRIVER.PRODUCTION_PATHS)
        self.assertEqual(len(proof["dependency_inputs"]), 3)
        self.assertEqual(len(proof["allocation_fixture_sha256"]), 64)

    def test_wrong_source_parent_dirty_checkout_or_paths_stop_before_build(self):
        for mutation in [{("rev-parse", "HEAD"): "other"}, {("rev-parse", "HEAD^"): "other"},
                {("status", "--porcelain"): " M source.rs"},
                {("diff", "--name-only", "HEAD^", "HEAD"): "outside.rs"}]:
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                self.run_source_proof(mutation)

    def test_commands_are_release_offline_and_select_only_reviewed_targets(self):
        command = DRIVER.build_command(inputs()["tests"])
        self.assertEqual(command[:7], ["cargo", "+1.99.1", "test", "--manifest-path", "rust/Cargo.toml", "-p", "nostr-double-ratchet"])
        self.assertTrue({"--locked", "--offline", "--release", "--no-run"}.issubset(command))
        self.assertEqual(command.count("--test"), 3)
        self.assertEqual(command.count("--lib"), 1)
        self.assertEqual(command[command.index("--jobs") + 1], "2")

    def artifacts(self, root):
        rows = []
        for test in inputs()["tests"]:
            binary = root / test["target"]
            binary.write_text("fixed executable")
            rows.append({"reason": "compiler-artifact", "executable": str(binary),
                "target": {"kind": [test["kind"]], "name": test["target"]},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}})
        return rows

    def test_exact_executable_profile_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary).resolve()
            rows = self.artifacts(target)
            actual = DRIVER.test_binaries("\n".join(map(json.dumps, rows)), inputs()["tests"], target)
            self.assertEqual(len(actual), 4)
            self.assertTrue(all(v["sha256"] == DRIVER.sha(Path(v["path"])) for v in actual.values()))

    def test_missing_duplicate_wrong_profile_and_outside_binary_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary).resolve()
            rows = self.artifacts(target)
            changes = [lambda r: r.pop(), lambda r: r.append(r[0]),
                lambda r: r[0]["profile"].update(opt_level="0"),
                lambda r: r[0]["profile"].update(debug_assertions=True),
                lambda r: r[0]["profile"].update(debuginfo=0),
                lambda r: r[0].update(executable=__file__),
                lambda r: r[0]["target"].update(name="different_target")]
            for change in changes:
                bad = copy.deepcopy(rows)
                change(bad)
                with self.subTest(change=change), self.assertRaises(AssertionError):
                    DRIVER.test_binaries("\n".join(map(json.dumps, bad)), inputs()["tests"], target)

    def test_reuse_requires_same_source_target_fixture_dependencies_and_tools(self):
        source = {"parent": "red", "allocation_fixture_sha256": "same", "dependency_inputs": {"lock": "same"}}
        versions = {"cargo": "1.99.1", "rustc": "1.99.1"}
        before = {"accepted": True, "stage": "before", "source_path": "/source", "target_path": "/target",
            "source": dict(source, commit="red"), "toolchain": versions}
        DRIVER.require_reuse(before, source, Path("/source"), Path("/target"), versions)
        changes = [lambda d: d.update(accepted=False), lambda d: d.update(stage="after"),
            lambda d: d.update(source_path="/other"), lambda d: d.update(target_path="/other"),
            lambda d: d["source"].update(commit="other"),
            lambda d: d["source"].update(allocation_fixture_sha256="changed"),
            lambda d: d["source"].update(dependency_inputs={"lock": "changed"}),
            lambda d: d.update(toolchain={"cargo": "other"})]
        for change in changes:
            bad = copy.deepcopy(before)
            change(bad)
            with self.subTest(change=change), self.assertRaises(AssertionError):
                DRIVER.require_reuse(bad, source, Path("/source"), Path("/target"), versions)


class StageTests(unittest.TestCase):
    def run_stage(self, which, compile_failure=False, wrong_assertion=False, fetch_failure=False):
        data = inputs()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            out, target = root / "evidence", root / "target"
            out.mkdir(); target.mkdir()
            source = {"commit": data[which], "parent": data["base"] if which == "before" else data["before"],
                "allocation_fixture_sha256": "same", "dependency_inputs": {"lock": "same"}}
            versions = {tool: f"{tool} 1.99.1 (test)\n" for tool in ("cargo", "rustc")}
            DRIVER.save(out / "preflight.json", {"schema": "ndr-blind-allocation/v1", "inputs": data,
                "inputs_sha256": DRIVER.sha(DRIVER.INPUTS), "toolchain": "1.99.1", "deadline_unix": time.time() + 1500})
            if which == "after":
                (out / "before").mkdir()
                DRIVER.save(out / "before/receipt.json", {"accepted": True, "stage": "before", "source_path": str(root),
                    "target_path": str(target), "source": dict(source, commit=data["before"]), "toolchain": versions})
            rows = SourceAndBuildTests.artifacts(self, target)
            selected = data["tests"] if which == "after" else data["tests"][:1]
            commands = []

            def bounded(command, _root, _out, name, timeout, deadline, _target):
                self.assertEqual(_target, target)
                self.assertEqual(_root, root)
                self.assertGreater(deadline, time.time())
                commands.append((name, command, timeout))
                if name == "fetch":
                    self.assertIn("--locked", command)
                    self.assertNotIn("--offline", command)
                    return {"exit_code": 101 if fetch_failure else 0}, "fetch result"
                if name == "build":
                    self.assertIn("--offline", command)
                    self.assertEqual(timeout, 600)
                    return {"exit_code": 101 if compile_failure else 0}, "\n".join(map(json.dumps, rows if which == "after" else rows[:1]))
                index = int(name.split("-")[1])
                test = selected[index]
                self.assertEqual(timeout, 300)
                self.assertIn("--test-threads=1", command)
                text = test_output(test, which == "before", 10008 if which == "before" else 8)
                if wrong_assertion:
                    text = text.replace("bookkeeping allocated 10008 times", "fixture unavailable")
                return {"exit_code": 101 if which == "before" else 0}, text

            with mock.patch.object(DRIVER, "source_proof", return_value=source), mock.patch.object(DRIVER, "git", return_value=""), \
                    mock.patch.object(DRIVER.subprocess, "check_output", side_effect=lambda cmd, **kw: versions[cmd[0]].encode()), \
                    mock.patch.object(DRIVER, "bounded", bounded), mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(target)):
                if compile_failure or fetch_failure or wrong_assertion:
                    with self.assertRaises(AssertionError):
                        DRIVER.stage(which, root, out, data)
                    self.assertFalse((out / which / "receipt.json").exists())
                    if wrong_assertion:
                        results = json.loads((out / which / "test-results.json").read_text())
                        self.assertFalse(results[0]["accepted"])
                        self.assertIn("error", results[0])
                    else:
                        self.assertFalse(any(name.startswith("test-") for name, _, _ in commands))
                else:
                    DRIVER.stage(which, root, out, data)
                    receipt = json.loads((out / which / "receipt.json").read_text())
                    self.assertTrue(receipt["accepted"])
                    self.assertFalse(receipt["physical_cpu_acceptance"])
                    self.assertEqual(len(receipt["tests"]), len(selected))
                    self.assertEqual(sum(t["counts"]["passed"] for t in receipt["tests"]), 27 if which == "after" else 0)
                    self.assertEqual(sum(t["counts"]["failed"] for t in receipt["tests"]), 0 if which == "after" else 1)
            self.assertEqual([name for name, _, _ in commands].count("fetch"), 0 if which == "after" else 1)

    def test_red_is_one_executed_assertion_failure(self):
        self.run_stage("before")

    def test_green_builds_once_runs_twenty_seven_and_never_fetches(self):
        self.run_stage("after")

    def test_compile_failure_is_not_red_and_no_test_runs(self):
        self.run_stage("before", compile_failure=True)

    def test_fetch_failure_is_not_red_and_no_build_runs(self):
        self.run_stage("before", fetch_failure=True)

    def test_wrong_red_assertion_preserves_failed_partial_result(self):
        self.run_stage("before", wrong_assertion=True)


if __name__ == "__main__":
    unittest.main()
