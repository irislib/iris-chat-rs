#!/usr/bin/env python3
"""Offline repair admission, intended RED and bounded GREEN sequencing."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_repair_retry as D
import test_checkpoint_update_driver as CHECKPOINT
import test_persist_attribution_driver as TRACE


def inputs():
    data = json.loads(D.INPUTS.read_text()); data["enabled"] = True
    return data


def source(which, data):
    expected = data["dependencies"][which]
    packages = {D.receive.NDR: {"version": expected["ndr_version"], "source": D.receive.REGISTRY,
        "checksum": expected["ndr_checksum"]}, D.receive.CODEC: {"version": "0.0.168",
        "source": D.receive.REGISTRY, "checksum": D.receive.CODEC_CHECKSUM}}
    return {"commit": data[which], "fixtures": D.FIXTURES, "dependencies": {**expected, "packages": packages},
        "protocol_lock_sha256": expected["lock_sha256"]}


def output(test, failure=None, cpu=4.0, bad_trace=False):
    if test["ignored"]:
        metrics = TRACE.fixture(cpu)
        if bad_trace: metrics["slices"][0]["persistence_trace"]["overflow"] = True
        return TRACE.output(test, metrics, failure)
    if test["name"] != D.RED_TEST:
        return CHECKPOINT.output(test, failure, cpu)
    state = "FAILED. 0 passed; 1 failed" if failure else "ok. 1 passed; 0 failed"
    text = f"running 1 test\ntest {test['name']} ... \n"
    if failure:
        text += f"thread '{test['name']}' panicked at src/protocol_engine/{failure}:\nassertion `left == right` failed\n  left: [(1, false), (1, false)]\n right: [(0, true), (1, false)]\n"
    return text + f"test result: {state}; 0 ignored; 100 filtered out;\n"


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "input.json"; path.write_text(json.dumps(data))
            with mock.patch.object(D, "INPUTS", path): return D.contract()

    def test_disabled_current_source_blocks_admission(self):
        self.assertFalse(json.loads(D.INPUTS.read_text())["enabled"])
        with self.assertRaisesRegex(AssertionError, "disabled"):
            self.verify(dict(inputs(), enabled=False))

    def test_exact_52_checks_same_v3_and_prior_37(self):
        data = self.verify(inputs())
        self.assertEqual(sum(t["expected_count"] for t in data["tests"]), 52)
        self.assertEqual(data["tests"][:14], D.receive.focused_tests())
        self.assertEqual(data["cpu"], D.checkpoint.cpu_plan())
        self.assertEqual(data["before"], D.COMMITS[2])

    def test_source_fixture_test_dependency_or_threshold_changes_fail_closed(self):
        changes = [lambda d: d.update(after="main"), lambda d: d.update(before=D.COMMITS[0]),
            lambda d: d["source_chain"][1].update(parent=d["base"]),
            lambda d: d["source_chain"][-1]["changed_paths"].append("core/src/lib.rs"),
            lambda d: d["source_chain"].pop(), lambda d: d["fixtures"].update({D.TEST_PATH: "0" * 64}),
            lambda d: d["tests"][14].update(expected_count=4), lambda d: d["tests"].pop(),
            lambda d: d["cpu"]["test"].update(timeout_seconds=600), lambda d: d["cpu"].update(schema_version=2),
            lambda d: d["dependencies"]["after"].update(lock_sha256="0" * 64),
            lambda d: d["trace"].update(limit=65)]
        for change in changes:
            data = inputs(); change(data)
            with self.subTest(change=change), self.assertRaises(AssertionError): self.verify(data)

    def test_workflow_reuses_comparison_resource_and_execution_bounds(self):
        workflow = (D.INPUTS.parent.parent / ".github/workflows/ci.yml").read_text()
        job = workflow.split("  inbound-deadline:\n")[1].split("  ndr-blind-allocation:\n")[0]
        for literal in ["inputs.scope == 'repair-retry'", "verify_repair_retry.py", "timeout-minutes: 30",
            "CARGO_BUILD_JOBS: '2'", "CARGO_PROFILE_RELEASE_OPT_LEVEL: '3'", "CARGO_PROFILE_RELEASE_DEBUG: '1'",
            "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS: 'false'", "before --source iris-chat-rs", "after --source iris-chat-rs"]:
            self.assertIn(literal, job)
        self.assertEqual(job.count("uses: actions/cache/restore@v6"), 1)


class ResultTests(unittest.TestCase):
    def test_exact_final_red_assertion_after_positive_controls(self):
        test = D.checkpoint.spec(D.RED_TEST)
        result = D.check_result(output(test, D.FAILURE), test, 101, D.FAILURE)
        self.assertEqual(result["repair_checkpoints"], {"observed": [(1, False), (1, False)], "expected": [(0, True), (1, False)]})

    def test_compile_pass_wrong_location_wrong_tuple_and_multiple_panics_are_not_red(self):
        test = D.checkpoint.spec(D.RED_TEST); good = output(test, D.FAILURE)
        cases = [("compilation failed", 101), (output(test), 0),
            (good.replace(":122:5", ":121:5"), 101), (good.replace("[(1, false), (1, false)]", "[(0, true), (1, false)]"), 101),
            (good.replace("[(0, true), (1, false)]", "[(0, true), (0, true)]"), 101),
            (good + "second thread panicked at elsewhere", 101), (good, -9)]
        for text, code in cases:
            with self.subTest(text=text), self.assertRaises(AssertionError): D.check_result(text, test, code, D.FAILURE)

    def test_cpu_requires_full_v3_work_and_complete_bounded_trace(self):
        test = D.checkpoint.cpu_plan()["test"]
        result = D.check_result(output(test), test, 0)
        self.assertGreater(result["persistence_attribution"]["successful_puts"], 0)
        for text in [output(test, bad_trace=True), output(test).replace('"pending_inbound": 4214', '"pending_inbound": 1')]:
            with self.assertRaises(AssertionError): D.check_result(text, test, 0)


class DependencyTests(unittest.TestCase):
    def test_exact_transition_and_source_lock_registry_mismatch(self):
        data = inputs(); before, after = source("before", data), source("after", data)
        self.assertTrue(D.dependency_transition(before, after, data)["accepted"])
        for change in [lambda s: s.update(commit=data["before"]), lambda s: s.update(protocol_lock_sha256="0" * 64),
            lambda s: s["dependencies"].update(manifest_sha256="0" * 64),
            lambda s: s["dependencies"]["packages"][D.receive.NDR].update(checksum="0" * 64),
            lambda s: s["dependencies"]["packages"][D.receive.NDR].update(source="git+unreviewed")]:
            candidate = copy.deepcopy(after); change(candidate)
            with self.subTest(change=change), self.assertRaises(AssertionError): D.dependency_transition(before, candidate, data)

    def test_locked_parser_requires_manifest_registry_versions_and_hashes(self):
        for which in ("before", "after"):
            for mutation in (None, "checksum", "version", "source", "extra", "manifest", "hash"):
                data = inputs(); expected = source(which, data)["dependencies"]["packages"]
                manifest = {name: row["version"] for name, row in expected.items()}
                rows = [{"name": name, **row} for name, row in expected.items()]
                if mutation in ("checksum", "source"): rows[0][mutation] = "wrong"
                elif mutation == "version": rows[0]["version"] = "0.0.1"
                elif mutation == "extra": rows.append(dict(rows[0], name=D.receive.NDR + "-pairwise"))
                elif mutation == "manifest": manifest[D.receive.NDR] = "0.0.1"
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary); directory = root / "chat-protocol"; directory.mkdir()
                    m = "[dependencies]\n" + "\n".join(f'{name} = "{value}"' for name, value in manifest.items())
                    lock = "\n".join("[[package]]\n" + "\n".join(f'{key} = "{value}"' for key, value in row.items()) for row in rows)
                    (directory / "Cargo.toml").write_text(m); (directory / "Cargo.lock").write_text(lock)
                    data["dependencies"][which].update(manifest_sha256=hashlib.sha256(m.encode()).hexdigest(), lock_sha256=hashlib.sha256(lock.encode()).hexdigest())
                    if mutation == "hash": data["dependencies"][which]["lock_sha256"] = "0" * 64
                    with self.subTest(which=which, mutation=mutation):
                        if mutation:
                            with self.assertRaises(AssertionError): D.dependencies(root, which, data)
                        else: self.assertEqual(D.dependencies(root, which, data)["packages"], expected)


class StageTests(unittest.TestCase):
    def exercise(self, which="after", *, failure_index=None, compile_failure=False, fetch_failure=False,
            bad_dependency=False, red_mode="expected", cpu=4.0, bad_trace=False):
        data, retry = inputs(), D.retry
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); out = root / "evidence"; target = root / "target"
            out.mkdir(); target.mkdir(); binary = target / "one-binary"; binary.write_bytes(b"one fixed compiled test executable")
            retry.save(out / "preflight.json", {"inputs": data, "inputs_sha256": retry.sha(D.INPUTS), "mode": "comparison", "deadline_unix": time.time() + 1500})
            versions = {tool: f"{tool} 1.99.1 (test)\n" for tool in ("rustc", "cargo")}
            current = source(which, data)
            if bad_dependency: current["protocol_lock_sha256"] = "0" * 64
            if which == "after":
                (out / "before").mkdir(); retry.save(out / "before/receipt.json", {"accepted": True})
                retry.save(out / "before/source.json", source("before", data)); retry.save(out / "before/toolchain.json", versions)
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            selected = [D.checkpoint.spec(D.RED_TEST)] if which == "before" else [*data["tests"], data["cpu"]["test"]]
            commands = []
            def bounded(command, _root, _out, name, timeout, deadline, _target):
                commands.append((name, command)); self.assertEqual(_target, target); self.assertGreater(deadline, time.time())
                if name == "fetch":
                    self.assertEqual(command, ["cargo", "+1.99.1", "fetch", "--manifest-path", "chat-protocol/Cargo.toml", "--locked"])
                    self.assertEqual(timeout, 300)
                    return {"exit_code": 101 if fetch_failure else 0}, "fetch result"
                if name == "build":
                    self.assertTrue({"--release", "--locked", "--offline", "--lib", "--no-run"}.issubset(command))
                    self.assertEqual(command[-3:-1], ["--jobs", "2"]); self.assertEqual(timeout, 600)
                    return {"exit_code": 101 if compile_failure else 0}, json.dumps(artifact)
                index = int(name.split("-")[1]); test = selected[index]
                self.assertEqual(command[0], str(binary)); self.assertEqual(timeout, 420 if test["ignored"] else 300)
                failure = None
                if which == "before": failure = {"expected": D.FAILURE, "wrong": "wrong.rs:1:1", "pass": None}[red_mode]
                elif index == failure_index: failure = "focused check failed"
                elif test["ignored"] and cpu > 5: failure = data["cpu"]["failure_marker"]
                return {"exit_code": 101 if failure else 0}, output(test, failure, cpu, bad_trace)
            with mock.patch.object(retry, "bounded", bounded), mock.patch.object(D, "source_proof", return_value=current), \
                mock.patch.object(retry, "git", side_effect=lambda _, *args: data[which] if args == ("rev-parse", "HEAD") else ""), \
                mock.patch.object(retry.subprocess, "check_output", side_effect=lambda cmd, **kw: versions[cmd[0]].encode()), \
                mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(target)), \
                mock.patch.object(retry, "whole_retry_metrics", side_effect=AssertionError("Standalone CPU forbidden")):
                failure_expected = any((compile_failure, fetch_failure, bad_dependency, failure_index is not None, red_mode != "expected", cpu > 5, bad_trace))
                if failure_expected:
                    with self.assertRaises(AssertionError): D.run(which, root, out, data)
                else: D.run(which, root, out, data)
            needs_fetch = which == "before" or data["dependencies"]["before"]["lock_sha256"] != data["dependencies"]["after"]["lock_sha256"]
            self.assertEqual(sum(name == "fetch" for name, _ in commands), int(needs_fetch and not bad_dependency))
            self.assertEqual(sum(name == "build" for name, _ in commands), int(not bad_dependency and not fetch_failure))
            executions = [c for name, c in commands if name.startswith("test-")]
            cpu_commands = [c for c in executions if c[1] == data["cpu"]["test"]["name"]]
            self.assertEqual(len(cpu_commands), int(which == "after" and failure_index is None and not (bad_dependency or compile_failure or fetch_failure)))
            if bad_dependency or compile_failure or fetch_failure: self.assertFalse(executions)
            elif failure_index is not None: self.assertEqual(len(executions), failure_index + 1)
            elif which == "after" and not bad_trace:
                receipt = json.loads((out / which / "receipt.json").read_text())
                self.assertEqual(receipt["test_binary_sha256"], retry.sha(binary))
                self.assertEqual(sum(t["counts"]["passed"] for t in receipt["tests"][:-1]), 52)
                self.assertEqual(receipt["accepted"], cpu <= 5)
                self.assertEqual(receipt["unprofiled_cpu_failure"], cpu > 5)
                self.assertEqual(len(executions), 24)
            self.assertFalse((out / which / "sampling.json").exists())

    def test_expected_red_is_one_test_without_cpu(self): self.exercise("before")

    def test_unexpected_red_pass_or_wrong_failure_stop(self):
        for mode in ("pass", "wrong"): self.exercise("before", red_mode=mode)

    def test_fetch_and_compile_failures_stop_before_tests(self):
        self.exercise("before", fetch_failure=True)
        for which in ("before", "after"): self.exercise(which, compile_failure=True)

    def test_bad_dependency_blocks_green_build(self): self.exercise(bad_dependency=True)

    def test_every_focused_green_failure_blocks_cpu(self):
        for index in range(len(D.focused_tests())):
            with self.subTest(index=index): self.exercise(failure_index=index)

    def test_one_green_build_52_checks_and_single_cpu_same_binary(self): self.exercise()

    def test_cpu_failure_keeps_successes_without_retry_or_profile(self): self.exercise(cpu=5.15)

    def test_incomplete_trace_blocks_acceptance_without_retry(self): self.exercise(bad_trace=True)


if __name__ == "__main__":
    unittest.main()
