"""Exercise the focused runner with real fake-command processes, never Rust builds."""
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("test-rust-focused")
LOADER = importlib.machinery.SourceFileLoader("rust_focused", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
FOCUSED = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(FOCUSED)

FAKE_TOOL = r'''
import json, os, pathlib, signal, subprocess, sys, time
root = pathlib.Path(os.environ["FIXTURE_ROOT"])
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as output:
    output.write(json.dumps({"tool": name, "args": args,
        "target": os.environ.get("CARGO_TARGET_DIR"),
        "jobs": os.environ.get("CARGO_BUILD_JOBS")}) + "\n")
if name == "git":
    if (root / "delay-source-read").exists(): time.sleep(0.5)
    if args[0] == "rev-parse": print("1" * 40)
    else: sys.stdout.buffer.write(b"core/Cargo.toml\0core/source.rs\0scripts/test-rust-focused\0")
elif name == "rustc":
    print("rustc fixture\nhost: fixture-host")
elif name == "cargo" and args == ["-V"]:
    print("cargo fixture")
elif name == "cargo" and args[0] == "fmt":
    if os.environ.get("FAKE_SCENARIO") == "format-failure":
        print("formatting diff", flush=True)
        print("formatting stderr", file=sys.stderr, flush=True)
        sys.exit(3)
elif name == "cargo":
    scenario = os.environ.get("FAKE_SCENARIO", "")
    if scenario == "build-failure":
        print("compiler diagnostic", flush=True)
        print("compiler stderr", file=sys.stderr, flush=True)
        sys.exit(7)
    if scenario == "sleep":
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        def finish(signum, frame):
            child.wait(timeout=2)
            sys.exit(128 + signum)
        signal.signal(signal.SIGTERM, finish)
        (root / "ready.json").write_text(json.dumps([os.getpid(), child.pid]))
        print("build waiting", flush=True)
        time.sleep(60)
    target = pathlib.Path(os.environ["CARGO_TARGET_DIR"])
    target.mkdir(parents=True, exist_ok=True)
    binary = target / "fixture-tests"
    binary.write_text((root / "binary-template").read_text())
    binary.chmod(0o755)
    print(json.dumps({"reason": "compiler-artifact", "profile": {"test": True},
        "manifest_path": args[args.index("--manifest-path") + 1], "executable": str(binary)}))
else:
    raise AssertionError(name)
'''

FAKE_BINARY = r'''
import json, os, pathlib, subprocess, sys
root = pathlib.Path(os.environ["FIXTURE_ROOT"])
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as output:
    output.write(json.dumps({"tool": "binary", "args": args}) + "\n")
name = args[0]
print("test diagnostic: " + name, flush=True)
if name.startswith("tests::nested_"):
    child_name = {
        "tests::nested_good": "tests::one",
        "tests::nested_child_failure": "tests::failure",
        "tests::nested_child_zero": "tests::missing",
        "tests::nested_child_ignored": "tests::ignored",
        "tests::nested_outer_zero": "tests::one",
    }[name]
    child = subprocess.run([sys.executable, sys.argv[0], child_name, *args[1:]],
                           capture_output=True, text=True, check=False)
    print("owned child exit: " + str(child.returncode), flush=True)
    print("stdout:\n" + child.stdout, flush=True)
    print("stderr:\n" + child.stderr, flush=True)
    # Deliberately return zero even for a failed child: the runner must inspect
    # every result rather than trusting only the successful outer exit/summary.
    passed = 0 if name == "tests::nested_outer_zero" else 1
    print(f"test result: ok. {passed} passed; 0 failed; 0 ignored; 99 filtered out;")
elif name == "tests::no_summary":
    pass
elif name == "tests::missing":
    print("test result: ok. 0 passed; 0 failed; 0 ignored; 99 filtered out;")
elif name == "tests::ignored" and "--include-ignored" not in args:
    print("test result: ok. 0 passed; 0 failed; 1 ignored; 99 filtered out;")
elif name == "tests::failure":
    print("test result: FAILED. 0 passed; 1 failed; 0 ignored;")
    sys.exit(101)
else:
    if os.environ.get("FAKE_SCENARIO") == "slow-final-source":
        (root / "delay-source-read").touch()
    if name == "tests::mutates_source":
        (root / "core/source.rs").write_text("changed during test")
    print("test result: ok. 1 passed; 0 failed; 0 ignored; 99 filtered out;")
'''


@unittest.skipUnless(os.name == "posix", "runner requires POSIX process groups")
class FocusedRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / "scripts").mkdir()
        self.script = self.root / "scripts/test-rust-focused"
        shutil.copyfile(SCRIPT, self.script)
        (self.root / "core").mkdir()
        (self.root / "core/Cargo.toml").write_text("fixture manifest")
        (self.root / "core/source.rs").write_text("fixture source")
        self.target = self.root / "existing-target"
        self.target.mkdir()
        self.output = self.root / "result"
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        for name in ("git", "cargo", "rustc"):
            tool = self.bin_dir / name
            tool.write_text(f"#!{sys.executable}\n" + FAKE_TOOL)
            tool.chmod(0o755)
        (self.root / "binary-template").write_text(f"#!{sys.executable}\n" + FAKE_BINARY)
        self.env = {**os.environ, "PATH": str(self.bin_dir), "FIXTURE_ROOT": str(self.root),
                    "CARGO_TARGET_DIR": str(self.target)}

    def command(self, *options):
        return [sys.executable, str(self.script), "--crate", "core", "--output", str(self.output), *options]

    def run_fixture(self, *options):
        return subprocess.run(self.command(*options), env=self.env, capture_output=True, text=True, timeout=10)

    def receipt(self):
        return json.loads((self.output / "result.json").read_text())

    def calls(self):
        return [json.loads(line) for line in (self.root / "calls.jsonl").read_text().splitlines()]

    def test_multiple_exact_tests_share_one_build_and_keep_provenance(self):
        result = self.run_fixture("--test", "tests::one", "--tests", "tests::two\ntests::three")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        formats = [call for call in calls if call["tool"] == "cargo" and call["args"][0] == "fmt"]
        self.assertEqual(len(formats), 1)
        self.assertEqual(formats[0]["args"], ["fmt", "--manifest-path", str(self.root / "core/Cargo.toml"), "--check"])
        builds = [call for call in calls if call["tool"] == "cargo" and call["args"][0] == "test"]
        self.assertEqual(len(builds), 1)
        self.assertLess(calls.index(formats[0]), calls.index(builds[0]))
        self.assertEqual(builds[0]["args"], ["test", "--manifest-path", str(self.root / "core/Cargo.toml"),
            "--locked", "--lib", "--no-run", "--message-format=json-render-diagnostics", "--jobs", "2", "--offline"])
        self.assertEqual(builds[0]["target"], str(self.target))
        self.assertEqual(builds[0]["jobs"], "2")
        tests = [call["args"] for call in calls if call["tool"] == "binary"]
        self.assertEqual(tests, [["tests::" + name, "--exact", "--nocapture", "--test-threads=1"]
                                 for name in ("one", "two", "three")])
        receipt = self.receipt()
        self.assertTrue(receipt["passed"])
        self.assertEqual(receipt["source"]["commit"], "1" * 40)
        self.assertEqual(receipt["binary"]["sha256"], FOCUSED.sha256(self.target / "fixture-tests"))
        self.assertIn("rustc fixture", receipt["rustc"])
        self.assertIn("cargo fixture", receipt["cargo"])
        self.assertIn("test diagnostic: tests::three", result.stdout)
        for run in receipt["runs"]:
            self.assertEqual(run["log_sha256"], FOCUSED.sha256(run["log"]))
            self.assertEqual(run["exit_code"], 0)
            self.assertTrue(run["closure"]["closed"])

    def test_online_release_and_ignored_execution_are_explicit(self):
        result = self.run_fixture("--test", "tests::ignored", "--online", "--release", "--include-ignored")
        self.assertEqual(result.returncode, 0, result.stderr)
        build = next(call for call in self.calls() if call["tool"] == "cargo" and call["args"][0] == "test")
        self.assertNotIn("--offline", build["args"])
        self.assertIn("--release", build["args"])
        self.assertIn("--locked", build["args"])
        self.assertTrue(self.receipt()["passed"])

    def test_zero_match_and_ignored_only_are_failures(self):
        for name in ("missing", "ignored"):
            with self.subTest(name=name):
                self.output = self.root / name
                result = self.run_fixture("--test", "tests::" + name)
                self.assertEqual(result.returncode, 1)
                self.assertIn("did not execute exactly one", result.stderr)
                self.assertFalse(self.receipt()["passed"])
                self.assertEqual(self.receipt()["runs"][-1]["exit_code"], 0)

    def test_nested_child_and_outer_success_keep_both_summaries_and_logs(self):
        result = self.run_fixture("--test", "tests::nested_good")
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = self.receipt()
        self.assertTrue(receipt["passed"])
        run = receipt["runs"][-1]
        self.assertEqual(run["test_summary"], [["ok", "1", "0", "0"]] * 2)
        self.assertEqual(run["exit_code"], 0)
        self.assertTrue(run["closure"]["closed"])
        self.assertEqual(run["log_sha256"], FOCUSED.sha256(run["log"]))
        self.assertIn("owned child exit: 0", result.stdout)
        self.assertIn("test diagnostic: tests::one", Path(run["log"]).read_text())
        self.assertEqual([call["args"][0] for call in self.calls() if call["tool"] == "binary"],
                         ["tests::nested_good", "tests::one"])

    def test_nested_bad_child_or_outer_and_missing_summaries_are_rejected(self):
        for name, bad_index, bad_summary in (
            ("nested_child_failure", 0, ["FAILED", "0", "1", "0"]),
            ("nested_child_zero", 0, ["ok", "0", "0", "0"]),
            ("nested_child_ignored", 0, ["ok", "0", "0", "1"]),
            ("nested_outer_zero", 1, ["ok", "0", "0", "0"]),
            ("no_summary", None, None),
        ):
            with self.subTest(name=name):
                self.output = self.root / name
                result = self.run_fixture("--test", "tests::" + name)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn("did not execute exactly one", result.stderr)
                receipt = self.receipt()
                self.assertFalse(receipt["passed"])
                run = receipt["runs"][-1]
                self.assertEqual(run["exit_code"], 0, "summary rejection must not rely on process failure")
                self.assertTrue(run["closure"]["closed"])
                if bad_index is None:
                    self.assertEqual(run["test_summary"], [])
                else:
                    self.assertEqual(len(run["test_summary"]), 2)
                    self.assertEqual(run["test_summary"][bad_index], bad_summary)
                    self.assertEqual(run["test_summary"][1 - bad_index], ["ok", "1", "0", "0"])

    def test_test_failure_preserves_full_diagnostic_and_exit_code(self):
        result = self.run_fixture("--test", "tests::failure")
        self.assertEqual(result.returncode, 1)
        receipt = self.receipt()
        self.assertFalse(receipt["passed"])
        self.assertEqual(receipt["runs"][-1]["exit_code"], 101)
        self.assertIn("test diagnostic: tests::failure", (self.output / "test-001.log").read_text())

    def test_build_failure_keeps_diagnostics_without_running_binary(self):
        self.env["FAKE_SCENARIO"] = "build-failure"
        result = self.run_fixture("--test", "tests::one")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.receipt()["runs"][-1]["exit_code"], 7)
        self.assertEqual((self.output / "build.log").read_text(), "compiler diagnostic\ncompiler stderr\n")
        self.assertFalse(any(call["tool"] == "binary" for call in self.calls()))

    def test_format_failure_stops_before_compilation_and_keeps_provenance(self):
        self.env["FAKE_SCENARIO"] = "format-failure"
        result = self.run_fixture("--test", "tests::one")
        self.assertEqual(result.returncode, 1)
        receipt = self.receipt()
        self.assertFalse(receipt["passed"])
        self.assertEqual(receipt["source"]["commit"], "1" * 40)
        self.assertIn("format failed with exit 3", receipt["error"])
        self.assertEqual(receipt["runs"][-1]["exit_code"], 3)
        self.assertTrue(receipt["runs"][-1]["closure"]["closed"])
        log = self.output / "format.log"
        self.assertEqual(log.read_text(), "formatting diff\nformatting stderr\n")
        self.assertEqual(receipt["runs"][-1]["log_sha256"], FOCUSED.sha256(log))
        self.assertFalse((self.output / "build.log").exists())
        self.assertFalse((self.target / "fixture-tests").exists())
        self.assertFalse(any(call["tool"] == "binary" or
                             (call["tool"] == "cargo" and call["args"][0] == "test")
                             for call in self.calls()))

    def test_dry_run_never_invokes_commands_or_creates_output(self):
        self.env["PATH"] = ""
        result = self.run_fixture("--test", "tests::one", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        plan = json.loads(result.stdout)
        self.assertEqual(plan["target_dir"], str(self.target))
        self.assertEqual(plan["format_command"], ["cargo", "fmt", "--manifest-path",
                                               str(self.root / "core/Cargo.toml"), "--check"])
        self.assertFalse(self.output.exists())
        self.assertFalse((self.root / "calls.jsonl").exists())

    def test_invalid_names_and_nonfinite_limits_fail_before_commands(self):
        for options in (("--test", "--ignored"), ("--test", "x;echo"),
                        ("--test", "same", "--test", "same"),
                        ("--test", "ok", "--timeout", "nan"),
                        ("--test", "ok", "--min-free-gib", "inf"),
                        ("--test", "ok", "--max-growth-gib", "nan")):
            with self.subTest(options=options):
                self.assertEqual(self.run_fixture(*options).returncode, 2)
        self.assertFalse((self.root / "calls.jsonl").exists())

    def test_no_implicit_worktree_target(self):
        del self.env["CARGO_TARGET_DIR"]
        result = self.run_fixture("--test", "tests::one")
        self.assertEqual(result.returncode, 2)
        self.assertIn("established cache", result.stderr)
        self.assertFalse((self.root / "calls.jsonl").exists())

    def test_source_mutation_cannot_produce_pass(self):
        result = self.run_fixture("--test", "tests::mutates_source")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.receipt()["error"], "source changed during the focused run")

    def test_timeout_in_final_source_check_cannot_produce_pass(self):
        self.env["FAKE_SCENARIO"] = "slow-final-source"
        result = self.run_fixture("--test", "tests::one", "--timeout", "0.8")
        self.assertEqual(result.returncode, 1)
        receipt = self.receipt()
        self.assertEqual(receipt["error"], "focused run timed out")
        self.assertTrue(all(run["exit_code"] == 0 for run in receipt["runs"]))
        self.assertFalse(receipt["passed"])

    def test_timeout_closes_build_process_group_and_keeps_receipt(self):
        self.env["FAKE_SCENARIO"] = "sleep"
        result = self.run_fixture("--test", "tests::one", "--timeout", "0.8")
        self.assertEqual(result.returncode, 1)
        receipt = self.receipt()
        self.assertEqual(receipt["error"], "focused run timed out")
        self.assertTrue(receipt["runs"][-1]["closure"]["closed"], receipt)
        for pid in json.loads((self.root / "ready.json").read_text()):
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)

    def test_external_sigterm_closes_build_process_group_and_keeps_receipt(self):
        self.env["FAKE_SCENARIO"] = "sleep"
        process = subprocess.Popen(self.command("--test", "tests::one"), env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 5
            while not (self.root / "ready.json").exists() and time.monotonic() < deadline:
                self.assertIsNone(process.poll())
                time.sleep(0.01)
            self.assertTrue((self.root / "ready.json").exists())
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=5)
            self.assertEqual(process.returncode, 1)
            receipt = self.receipt()
            self.assertIn("SIGTERM", receipt["error"])
            self.assertTrue(receipt["runs"][-1]["closure"]["closed"], receipt)
            for pid in json.loads((self.root / "ready.json").read_text()):
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
        finally:
            ready = self.root / "ready.json"
            if ready.exists():
                group = json.loads(ready.read_text())[0]
                try:
                    os.killpg(group, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)

    def test_disk_guards_stop_actual_child_and_record_usage(self):
        for floor, growth, expected in ((900, None, "free-space floor"), (None, 100, "cumulative filesystem growth")):
            with self.subTest(expected=expected):
                self.output.mkdir(exist_ok=True)
                reads = iter([1000, 1000, 1000, 800])
                def disk_usage(_):
                    return shutil._ntuple_diskusage(2000, 1000, next(reads, 800))
                with patch.object(FOCUSED.shutil, "disk_usage", side_effect=disk_usage):
                    runner = FOCUSED.Runner(self.output, self.target, 5, floor, growth)
                    label = "floor" if floor else "growth"
                    with self.assertRaisesRegex(RuntimeError, expected):
                        runner.run(label, [sys.executable, "-c", "import time; time.sleep(60)"], os.environ.copy())
                    self.assertTrue(runner.runs[-1]["closure"]["closed"])
                    self.assertEqual(runner.lowest_free, 800)

    def test_cleanup_probe_error_still_records_exit_and_log(self):
        self.output.mkdir()
        runner = FOCUSED.Runner(self.output, self.target, 5)
        with patch.object(FOCUSED, "close_group", side_effect=OSError("cleanup probe failed")):
            with self.assertRaisesRegex(RuntimeError, "live process group"):
                runner.run("probe", [sys.executable, "-c", "print('complete')"], os.environ.copy())
        self.assertEqual(runner.runs[0]["closure"]["error"], "cleanup probe failed")
        self.assertEqual(runner.runs[0]["exit_code"], 0)
        self.assertIn("log_sha256", runner.runs[0])


if __name__ == "__main__":
    unittest.main()
