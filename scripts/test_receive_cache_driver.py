#!/usr/bin/env python3
"""Offline contracts and mocked execution ordering; no Cargo or network."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_receive_cache as DRIVER
import test_checkpoint_update_driver as CHECKPOINT


def inputs():
    data = json.loads(DRIVER.INPUTS.read_text())
    data["enabled"] = True
    return data


def output(test, failure=None, cpu=4.0):
    if test["name"] != DRIVER.RED_TEST:
        return CHECKPOINT.output(test, failure, cpu)
    state = "FAILED. 0 passed; 1 failed" if failure else "ok. 1 passed; 0 failed"
    text = f"running 1 test\ntest {test['name']} ... \n"
    if failure:
        text += f"thread '{test['name']}' panicked at src/protocol_engine/{failure}:\nassertion `left == right` failed\n  left: (2, 2)\n right: (0, 1)\n"
    return text + f"test result: {state}; 0 ignored; 100 filtered out;\n"


def source(which, data):
    dep = {**data["dependencies"][which], "packages": {
        DRIVER.NDR: {"version": "0.0.172" if which == "before" else "0.0.173", "source": DRIVER.REGISTRY,
            "checksum": DRIVER.NDR_CHECKSUMS[which]},
        DRIVER.CODEC: {"version": "0.0.168", "source": DRIVER.REGISTRY, "checksum": DRIVER.CODEC_CHECKSUM}}}
    return {"commit": data[which], "fixtures": DRIVER.FIXTURES, "dependencies": dep, "protocol_lock_sha256": dep["lock_sha256"]}


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "input.json"; path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, "INPUTS", path):
                return DRIVER.contract()

    def test_disabled_input_cannot_admit_work(self):
        self.assertFalse(json.loads(DRIVER.INPUTS.read_text())["enabled"])
        with self.assertRaisesRegex(AssertionError, "disabled"):
            self.verify(dict(inputs(), enabled=False))

    def test_exact_37_and_unchanged_v3_cpu(self):
        data = self.verify(inputs())
        self.assertEqual(sum(t["expected_count"] for t in data["tests"]), 37)
        self.assertEqual(data["tests"][:12], DRIVER.checkpoint.focused_tests())
        self.assertEqual(data["cpu"], DRIVER.checkpoint.cpu_plan())

    def test_changed_chain_fixture_selection_dependencies_or_cpu_fail_closed(self):
        changes = [lambda d: d.update(after="main"), lambda d: d["source_chain"][1].update(parent=d["base"]),
            lambda d: d["source_chain"][-1]["changed_paths"].append("core/src/lib.rs"),
            lambda d: d["source_chain"].pop(), lambda d: d["fixtures"].update({DRIVER.TEST_PATH: "0" * 64}),
            lambda d: d["tests"][-2].update(expected_count=3), lambda d: d["tests"].pop(),
            lambda d: d["cpu"]["test"].update(timeout_seconds=600), lambda d: d["cpu"].update(schema_version=2),
            lambda d: d["dependencies"]["after"].update(lock_sha256=None),
            lambda d: d.update(dependencies=dict(before=d["dependencies"]["before"], after=d["dependencies"]["before"]))]
        for change in changes:
            data = inputs(); change(data)
            with self.subTest(change=change), self.assertRaises(AssertionError):
                self.verify(data)


class ResultTests(unittest.TestCase):
    def test_exact_final_red_assertion(self):
        test = DRIVER.checkpoint.spec(DRIVER.RED_TEST)
        result = DRIVER.check_result(output(test, DRIVER.FAILURE), test, 101, DRIVER.FAILURE)
        self.assertEqual(result["receive_calls"], dict(observed_repeated=2, observed_changed_envelope=2,
            expected_repeated=0, expected_changed_envelope=1))

    def test_compile_pass_wrong_location_wrong_tuple_and_multiple_panics_are_not_red(self):
        test = DRIVER.checkpoint.spec(DRIVER.RED_TEST); good = output(test, DRIVER.FAILURE)
        for text, code in [("compilation failed", 101), (output(test), 0),
            (good.replace(":69:5", ":68:5"), 101), (good.replace("(2, 2)", "(1, 2)"), 101),
            (good.replace("(0, 1)", "(0, 0)"), 101), (good + "second thread panicked at elsewhere", 101)]:
            with self.subTest(text=text), self.assertRaises(AssertionError):
                DRIVER.check_result(text, test, code, DRIVER.FAILURE)


class DependencyTests(unittest.TestCase):
    def test_exact_transition_and_reject_source_lock_or_registry_mismatch(self):
        data = inputs(); before, after = source("before", data), source("after", data)
        self.assertTrue(DRIVER.dependency_transition(before, after, data)["accepted"])
        for change in [lambda s: s.update(commit=data["before"]), lambda s: s.update(protocol_lock_sha256="0" * 64),
            lambda s: s["dependencies"].update(manifest_sha256="0" * 64),
            lambda s: s["dependencies"]["packages"][DRIVER.NDR].update(checksum="0" * 64),
            lambda s: s["dependencies"]["packages"][DRIVER.CODEC].update(checksum="0" * 64)]:
            candidate = copy.deepcopy(after); change(candidate)
            with self.subTest(change=change), self.assertRaises(AssertionError):
                DRIVER.dependency_transition(before, candidate, data)

    def test_actual_locked_parser_requires_exact_manifest_registry_and_versions(self):
        for which in ("before", "after"):
            for mutation in (None, "checksum", "version", "source", "extra", "manifest", "hash"):
                data = inputs(); expected = source(which, data)["dependencies"]["packages"]
                manifest = {"dependencies": {name: row["version"] for name, row in expected.items()}}
                rows = [{"name": name, **row} for name, row in expected.items()]
                if mutation in ("checksum", "source"):
                    rows[0][mutation] = "wrong"
                elif mutation == "version": rows[0]["version"] = "0.0.1"
                elif mutation == "extra": rows.append(dict(rows[0], name=DRIVER.NDR + "-pairwise"))
                elif mutation == "manifest": manifest["dependencies"][DRIVER.NDR] = "0.0.1"
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary); p = root / "chat-protocol"; p.mkdir()
                    m = "[dependencies]\n" + "\n".join(f'{name} = "{value}"' for name, value in manifest["dependencies"].items())
                    lock = "\n".join("[[package]]\n" + "\n".join(f'{key} = "{value}"' for key, value in row.items()) for row in rows)
                    (p / "Cargo.toml").write_text(m); (p / "Cargo.lock").write_text(lock)
                    data["dependencies"][which] = dict(manifest_sha256=hashlib.sha256(m.encode()).hexdigest(), lock_sha256=hashlib.sha256(lock.encode()).hexdigest())
                    if mutation == "hash": data["dependencies"][which]["lock_sha256"] = "0" * 64
                    with self.subTest(which=which, mutation=mutation):
                        if mutation:
                            with self.assertRaises(AssertionError): DRIVER.dependencies(root, which, data)
                        else:
                            self.assertEqual(DRIVER.dependencies(root, which, data)["packages"], expected)


class StageTests(unittest.TestCase):
    def exercise(self, which="after", *, failure_index=None, compile_failure=False, fetch_failure=False,
            bad_dependency=False, no_dependency_hook=False, red_mode="expected", cpu=4.0):
        data, retry = inputs(), DRIVER.retry
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); out = root / "evidence"; target = root / "target"
            out.mkdir(); target.mkdir(); binary = target / "one-green-binary"; binary.write_bytes(b"one fixed compiled test executable")
            retry.save(out / "preflight.json", {"inputs": data, "inputs_sha256": retry.sha(DRIVER.INPUTS), "mode": "comparison", "deadline_unix": time.time() + 1500})
            versions = {tool: f"{tool} 1.99.1 (test)\n" for tool in ("rustc", "cargo")}
            current = source(which, data)
            if bad_dependency: current["protocol_lock_sha256"] = "0" * 64
            if which == "after":
                (out / "before").mkdir(); retry.save(out / "before/receipt.json", {"accepted": True})
                retry.save(out / "before/source.json", source("before", data)); retry.save(out / "before/toolchain.json", versions)
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            selected = [DRIVER.checkpoint.spec(DRIVER.RED_TEST)] if which == "before" else [*data["tests"], data["cpu"]["test"]]
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
                if which == "before": failure = {"expected": DRIVER.FAILURE, "wrong": "wrong.rs:1:1", "pass": None}[red_mode]
                elif index == failure_index: failure = "focused check failed"
                elif test["ignored"] and cpu > 5: failure = data["cpu"]["failure_marker"]
                return {"exit_code": 101 if failure else 0}, output(test, failure, cpu)
            hook = None if no_dependency_hook else DRIVER.dependency_transition
            with mock.patch.object(retry, "bounded", bounded), mock.patch.object(DRIVER, "source_proof", return_value=current), \
                mock.patch.object(DRIVER, "dependency_transition", hook), \
                mock.patch.object(retry, "git", side_effect=lambda _, *args: data[which] if args == ("rev-parse", "HEAD") else ""), \
                mock.patch.object(retry.subprocess, "check_output", side_effect=lambda cmd, **kw: versions[cmd[0]].encode()), \
                mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(target)), \
                mock.patch.object(retry, "whole_retry_metrics", side_effect=AssertionError("Standalone CPU forbidden")):
                failure_expected = any((compile_failure, fetch_failure, bad_dependency, no_dependency_hook,
                    failure_index is not None, red_mode != "expected", cpu > 5))
                if failure_expected:
                    with self.assertRaises(AssertionError): DRIVER.run(which, root, out, data)
                else: DRIVER.run(which, root, out, data)
            blocked = bad_dependency or no_dependency_hook
            self.assertEqual(sum(name == "fetch" for name, _ in commands), int(not blocked))
            self.assertEqual(sum(name == "build" for name, _ in commands), int(not blocked and not fetch_failure))
            executions = [c for name, c in commands if name.startswith("test-")]
            cpu_commands = [c for c in executions if c[1] == data["cpu"]["test"]["name"]]
            self.assertEqual(len(cpu_commands), int(which == "after" and failure_index is None and not (blocked or compile_failure or fetch_failure)))
            if blocked or compile_failure or fetch_failure: self.assertFalse(executions)
            elif failure_index is not None: self.assertEqual(len(executions), failure_index + 1)
            elif which == "after":
                receipt = json.loads((out / which / "receipt.json").read_text())
                self.assertEqual(receipt["test_binary_sha256"], retry.sha(binary))
                self.assertEqual(sum(t["counts"]["passed"] for t in receipt["tests"][:-1]), 37)
                self.assertEqual(receipt["accepted"], cpu <= 5)
                self.assertEqual(receipt["unprofiled_cpu_failure"], cpu > 5)
                self.assertEqual(len(executions), 15)
            self.assertFalse((out / which / "sampling.json").exists())

    def test_expected_red_is_one_test_and_one_fetch_without_cpu(self): self.exercise("before")

    def test_unexpected_red_pass_or_wrong_failure_stop(self):
        for mode in ("pass", "wrong"): self.exercise("before", red_mode=mode)

    def test_fetch_and_compile_failures_stop_before_tests_on_both_sources(self):
        for which in ("before", "after"):
            self.exercise(which, fetch_failure=True); self.exercise(which, compile_failure=True)

    def test_bad_dependencies_or_unchanged_default_hook_block_green_fetch_and_build(self):
        self.exercise(bad_dependency=True); self.exercise(no_dependency_hook=True)

    def test_every_focused_green_failure_blocks_cpu(self):
        for index in range(len(DRIVER.focused_tests())):
            with self.subTest(index=index): self.exercise(failure_index=index)

    def test_one_green_fetch_then_offline_build_37_checks_and_single_cpu_same_binary(self): self.exercise()

    def test_cpu_failure_keeps_37_successes_without_retry_or_profile(self): self.exercise(cpu=6.1)


if __name__ == "__main__":
    unittest.main()
