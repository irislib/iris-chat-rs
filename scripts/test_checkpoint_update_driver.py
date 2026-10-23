#!/usr/bin/env python3
"""Offline admission, provenance, and RED/GREEN/CPU sequencing; no Cargo."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_checkpoint_update as DRIVER
import test_inbound_retry_driver as COMMON


def inputs():
    data = json.loads(DRIVER.INPUTS.read_text())
    data["enabled"] = True
    return data


def v3_receipt(cpu=4.0):
    result = COMMON.BatchedMetricsTests().fixture()
    result.update(schema="iris-batched-retry-cpu/v3", fixture="restored-mixed-backlog-with-corrupt-mac-inbound-v3",
        inbound_ciphertext_format="canonical-base64-nip44-v2-corrupt-mac", inbound_ciphertext_format_checked=4214,
        inbound_decoded_min_bytes=1091, inbound_decoded_max_bytes=1091)
    for row in result["slices"][-2:]:
        row.update(cpu_percent_one_core=cpu, cpu_seconds=cpu * row["elapsed_seconds"] / 100)
    return result


def output(test, failure=None, cpu=4.0):
    count = test["expected_count"]
    text = f"running {count} tests\ntest {test['name']} ... {test['marker'] or ''}\n"
    if test["name"] == DRIVER.RED_TEST and failure:
        reads = [(offset, min(4096, 135185 - offset)) for offset in range(0, 135185, 4096)]
        text += f"thread 'test' panicked at source.rs:1: {failure}\n  left: {reads}\n right: {DRIVER.READS}\n"
    elif test["name"] == DRIVER.INTEGRITY:
        text += "corrupt_mac_inbound_fixture packets=4214 format_checked=4214 decoded_min=1091 decoded_max=1091 authenticated_failures=1001 skip_limit_failures=3213 positive_controls=2 distinct_ids=4214\n"
        text += "corrupt_mac_inbound_retry packets=16 initially_due=3 rounds=2 saves_on_failed_retry=0 restart_preserved=true\n"
    elif test["ignored"]:
        text += COMMON.BatchedMetricsTests().render(v3_receipt(cpu)) + "\n"
    if failure and test["name"] != DRIVER.RED_TEST:
        text += f"thread 'test' panicked at source.rs:1: {failure}\n"
    status, passed, failed = ("FAILED", count - 1, 1) if failure else ("ok", count, 0)
    return text + f"test result: {status}. {passed} passed; {failed} failed; 0 ignored; 100 filtered out;\n"


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

    def test_exact_six_commit_chain_thirty_two_checks_and_one_cpu(self):
        data = self.verify(inputs())
        self.assertEqual(sum(t["expected_count"] for t in data["tests"]), 32)
        self.assertEqual(data["cpu"]["test"]["timeout_seconds"], 420)

    def test_changed_sources_scope_selection_or_cpu_contract_fail_closed(self):
        changes = [lambda d: d.update(after=d["before"]), lambda d: d["source_chain"][1].update(parent=d["base"]),
            lambda d: d["source_chain"][1].update(changed_paths=["core/src/lib.rs"]),
            lambda d: d["source_chain"][-1].update(commit="main"),
            lambda d: d["source_chain"].pop(), lambda d: d.update(storage_test_module_sha256="0" * 64),
            lambda d: d["tests"][0].update(expected_count=8), lambda d: d["tests"][1].update(ignored=True),
            lambda d: d["tests"].pop(), lambda d: d["cpu"].update(schema_version=2),
            lambda d: d["cpu"]["test"].update(timeout_seconds=600), lambda d: d["cpu"].update(failure_marker="panic")]
        for change in changes:
            data = inputs(); change(data)
            with self.subTest(change=change), self.assertRaises(AssertionError):
                self.verify(data)


class ResultTests(unittest.TestCase):
    def test_red_retains_exact_old_and_new_read_vectors(self):
        test = DRIVER.spec(DRIVER.RED_TEST)
        result = DRIVER.check_result(output(test, DRIVER.FAILURE), test, 101, DRIVER.FAILURE)
        self.assertEqual(result["checkpoint_reads"]["logical_bytes"], 135185)
        self.assertEqual(len(result["checkpoint_reads"]["observed"]), 34)

    def test_wrong_failure_vectors_zero_tests_compile_and_unexpected_pass_are_not_red(self):
        test = DRIVER.spec(DRIVER.RED_TEST)
        red = output(test, DRIVER.FAILURE)
        cases = [(red.replace(DRIVER.FAILURE, "other failure"), 101), (red.replace("(0, 4096)", "(0, 1)"), 101),
            (red.replace("right:", "missing:"), 101), (red.replace("1 failed", "0 failed"), 101),
            ("error: could not compile iris-chat-protocol", 101), (output(test), 0), (red, -9), (red + red, 101)]
        for text, code in cases:
            with self.subTest(text=text[:80]), self.assertRaises(AssertionError):
                DRIVER.check_result(text, test, code, DRIVER.FAILURE)

    def test_fixture_actual_counts_bounds_and_retry_markers(self):
        test = DRIVER.focused_tests()[1]
        result = DRIVER.check_result(output(test), test, 0)
        self.assertEqual(result["inbound_integrity"]["authenticated_failures"], 1001)
        for old, new in [("format_checked=4214", "format_checked=4213"), ("decoded_min=1091", "decoded_min=386"),
            ("decoded_max=1091", "decoded_max=2116"), ("authenticated_failures=1001", "authenticated_failures=0"),
            ("skip_limit_failures=3213", "skip_limit_failures=4214"), ("positive_controls=2", "positive_controls=0"),
            ("distinct_ids=4214", "distinct_ids=1"), ("restart_preserved=true", "restart_preserved=false"),
            ("saves_on_failed_retry=0", "saves_on_failed_retry=1")]:
            with self.subTest(old=old), self.assertRaises(AssertionError):
                DRIVER.check_result(output(test).replace(old, new), test, 0)

    def test_v3_requires_format_and_bounds_without_weakening_v2(self):
        result = v3_receipt()
        render, parse = COMMON.BatchedMetricsTests().render, DRIVER.retry.batched_retry_metrics
        self.assertEqual(parse(render(result), schema_version=3), result)
        self.assertEqual(parse(render(COMMON.BatchedMetricsTests().fixture()))["schema"], "iris-batched-retry-cpu/v2")
        with self.assertRaises(AssertionError):
            parse(render(result))
        changes = [lambda r: r.update(schema="iris-batched-retry-cpu/v2"), lambda r: r.update(inbound_ciphertext_format="malformed"),
            lambda r: r.update(inbound_ciphertext_format_checked=4213), lambda r: r.update(inbound_ciphertext_format_checked=4214.0),
            lambda r: r.update(inbound_decoded_min_bytes=386), lambda r: r.update(inbound_decoded_max_bytes=2116),
            lambda r: r.update(inbound_decoded_min_bytes=True), lambda r: r.update(inbound_decoded_max_bytes=1091.0),
            lambda r: r.update(inbound_decoded_min_bytes=1092), lambda r: r.update(acknowledged_setup_deliveries=0),
            lambda r: r.update(restart_checked=False), lambda r: r["slices"][3].update(expired_control_retirements=0)]
        for change in changes:
            candidate = copy.deepcopy(result); change(candidate)
            with self.subTest(change=change), self.assertRaises(AssertionError):
                parse(render(candidate), schema_version=3)


class SourceTests(unittest.TestCase):
    def test_section_requires_one_exact_boundary(self):
        contents = b"production\n" + DRIVER.SECTION + b"test bytes\n}\n"
        self.assertEqual(DRIVER.module_digest(contents), DRIVER.module_digest(contents.replace(b"production", b"new implementation")))
        self.assertNotEqual(DRIVER.module_digest(contents), DRIVER.module_digest(contents.replace(b"test bytes", b"different test")))
        for candidate in [b"no section", contents + contents]:
            with self.assertRaises(AssertionError):
                DRIVER.module_digest(candidate)

    def proof(self, wrong_parent=False, changed_path=False, changed_fixture=False):
        data, retry = inputs(), DRIVER.retry
        module = DRIVER.SECTION + b"\nfixed test module\n}\n"
        data["storage_test_module_sha256"] = DRIVER.module_digest(module)
        chain = data["source_chain"]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / DRIVER.STORAGE; path.parent.mkdir(parents=True); path.write_bytes(b"production\n" + module)
            lock = root / "chat-protocol/Cargo.lock"; lock.write_text("fixed dependencies")
            def git(_, *args):
                if args[0] == "rev-list":
                    item = next(x for x in chain if x["commit"] == args[-1])
                    parent = data["base"] if wrong_parent and item == chain[-1] else item["parent"]
                    return item["commit"] + " " + parent
                if args[0] == "diff":
                    item = next(x for x in chain if x["commit"] == args[-1])
                    return "outside.rs" if changed_path and item == chain[1] else "\n".join(item["changed_paths"])
                return {"HEAD": data["after"], "HEAD^": chain[-1]["parent"]}.get(args[-1], "tree")
            def source(_, commit, path):
                if path == DRIVER.STORAGE:
                    return module + (b"changed" if changed_fixture and commit == chain[1]["commit"] else b"")
                return b"exact committed source"
            with mock.patch.object(retry, "git", git), mock.patch.object(DRIVER, "source_bytes", source):
                return DRIVER.source_proof("after", root, data)

    def test_full_chain_and_regression_bytes_are_preserved_in_proof(self):
        result = self.proof()
        self.assertEqual(len(result["reviewed_chain"]), 6)
        self.assertEqual(result["changed_paths"], ["RELEASE.md"])

    def test_wrong_parent_path_or_intermediate_fixture_fails(self):
        for options in [dict(wrong_parent=True), dict(changed_path=True), dict(changed_fixture=True)]:
            with self.subTest(options=options), self.assertRaises(AssertionError):
                self.proof(**options)


class StageTests(unittest.TestCase):
    def exercise(self, which="after", *, failure_index=None, compile_failure=False, red_mode="expected", cpu=4.0):
        data, retry = inputs(), DRIVER.retry
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); out = root / "evidence"; target = root / "target"
            out.mkdir(); target.mkdir()
            binary = target / "one-green-binary"; binary.write_bytes(b"immutable compiled test executable")
            retry.save(out / "preflight.json", {"inputs": data, "inputs_sha256": retry.sha(DRIVER.INPUTS), "mode": "comparison", "deadline_unix": time.time() + 1500})
            versions = {tool: f"{tool} 1.99.1 (test)\n" for tool in ("rustc", "cargo")}
            source = {"commit": data[which], "fixtures": {"module": "same"}, "protocol_lock_sha256": "same"}
            if which == "after":
                before = out / "before"; before.mkdir()
                retry.save(before / "receipt.json", {"accepted": True}); retry.save(before / "source.json", source); retry.save(before / "toolchain.json", versions)
            artifact = {"reason": "compiler-artifact", "executable": str(binary), "target": {"name": "iris_chat_protocol"},
                "profile": {"test": True, "opt_level": "3", "debuginfo": 1, "debug_assertions": False}}
            selected = [DRIVER.spec(DRIVER.RED_TEST)] if which == "before" else [*data["tests"], data["cpu"]["test"]]
            commands = []
            def bounded(command, _root, _out, name, timeout, deadline, _target):
                commands.append((name, command)); self.assertEqual(_target, target); self.assertGreater(deadline, time.time())
                if name == "fetch":
                    self.assertEqual(which, "before"); self.assertIn("--locked", command)
                    return {"exit_code": 0}, "fetched"
                if name == "build":
                    self.assertTrue({"--release", "--locked", "--offline", "--lib", "--no-run"}.issubset(command))
                    self.assertEqual(command[-3:-1], ["--jobs", "2"]); self.assertEqual(timeout, 600)
                    return {"exit_code": 101 if compile_failure else 0}, json.dumps(artifact)
                index = int(name.split("-")[1]); test = selected[index]
                self.assertEqual(command[0], str(binary)); self.assertEqual(timeout, 420 if test["ignored"] else 300)
                failure = None
                if which == "before":
                    failure = {"expected": DRIVER.FAILURE, "wrong": "unrelated failure", "pass": None}[red_mode]
                elif index == failure_index:
                    failure = "focused check failed"
                elif test["ignored"] and cpu > 5:
                    failure = data["cpu"]["failure_marker"]
                return {"exit_code": 101 if failure else 0}, output(test, failure, cpu)
            with mock.patch.object(retry, "bounded", bounded), mock.patch.object(DRIVER, "source_proof", return_value=source), \
                mock.patch.object(retry, "git", side_effect=lambda _, *args: data[which] if args == ("rev-parse", "HEAD") else ""), \
                mock.patch.object(retry.subprocess, "check_output", side_effect=lambda cmd, **kw: versions[cmd[0]].encode()), \
                mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(target)), \
                mock.patch.object(retry, "whole_retry_metrics", side_effect=AssertionError("Standalone CPU forbidden")):
                failure_expected = compile_failure or failure_index is not None or red_mode != "expected" or cpu > 5
                if failure_expected:
                    with self.assertRaises(AssertionError):
                        DRIVER.run(which, root, out, data)
                else:
                    DRIVER.run(which, root, out, data)
            executions = [c for name, c in commands if name.startswith("test-")]
            cpu_commands = [c for c in executions if c[1] == data["cpu"]["test"]["name"]]
            expected_cpu = int(which == "after" and failure_index is None and not compile_failure)
            self.assertEqual(len(cpu_commands), expected_cpu)
            self.assertEqual(len([name for name, _ in commands if name == "build"]), 1)
            if compile_failure:
                self.assertFalse(executions)
            elif failure_index is not None:
                self.assertEqual(len(executions), failure_index + 1)
                self.assertFalse((out / which / "receipt.json").exists())
            elif which == "after":
                receipt = json.loads((out / which / "receipt.json").read_text())
                self.assertEqual(receipt["test_binary_sha256"], retry.sha(binary))
                self.assertEqual(sum(t["counts"]["passed"] for t in receipt["tests"][:-1]), 32)
                self.assertEqual(receipt["accepted"], cpu <= 5)
                self.assertEqual(receipt["unprofiled_cpu_failure"], cpu > 5)
                self.assertEqual(len(executions), 13)
                self.assertFalse((out / which / "sampling.json").exists())
            return commands

    def test_green_refuses_missing_or_unaccepted_red_receipt_before_build(self):
        data, retry = inputs(), DRIVER.retry
        for red_accepted in (None, False):
            with self.subTest(red_accepted=red_accepted), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                retry.save(root / "preflight.json", {"inputs": data, "inputs_sha256": retry.sha(DRIVER.INPUTS), "mode": "comparison", "deadline_unix": time.time() + 1500})
                if red_accepted is not None:
                    (root / "before").mkdir()
                    retry.save(root / "before/receipt.json", {"accepted": red_accepted})
                with mock.patch.object(retry, "bounded") as bounded, \
                    mock.patch.object(retry, "git", side_effect=lambda _, *args: data["after"] if args == ("rev-parse", "HEAD") else ""):
                    with self.assertRaises((AssertionError, FileNotFoundError)):
                        DRIVER.run("after", root, root, data)
                    bounded.assert_not_called()

    def test_expected_red_runs_one_regression_without_cpu(self):
        self.exercise("before")

    def test_unexpected_red_pass_and_wrong_failure_cannot_reach_cpu(self):
        for mode in ("pass", "wrong"):
            self.exercise("before", red_mode=mode)

    def test_compile_failure_cannot_reach_cpu_on_either_source(self):
        for which in ("before", "after"):
            self.exercise(which, compile_failure=True)

    def test_every_focused_green_failure_stops_before_cpu(self):
        for index in range(len(DRIVER.focused_tests())):
            with self.subTest(index=index):
                self.exercise(failure_index=index)

    def test_all_thirty_two_green_checks_precede_one_cpu_same_binary(self):
        self.exercise()

    def test_cpu_failure_retains_all_functional_success_without_sampling(self):
        self.exercise(cpu=6.1)


if __name__ == "__main__":
    unittest.main()
