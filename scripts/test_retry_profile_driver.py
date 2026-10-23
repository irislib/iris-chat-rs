import copy
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

import verify_retry_profile as driver


class ProfileTests(unittest.TestCase):
    def configured(self):
        data = json.loads(driver.INPUTS.read_text())
        data["enabled"] = True
        return data

    def contract(self, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "inputs.json"
            path.write_text(json.dumps(data))
            for name in data["helpers"]:
                (path.parent / name).write_bytes(driver.INPUTS.with_name(name).read_bytes())
            with patch.object(driver, "INPUTS", path):
                return driver.contract()

    def test_disabled_changed_source_helper_or_workload_cannot_start(self):
        data = self.configured()
        mutations = [("enabled", False), ("source", "a" * 40), ("tree", "b" * 40),
                     ("cpu_acceptance", True), ("sampling", {**data["sampling"], "frequency_hz": 999})]
        for key, value in mutations:
            changed = copy.deepcopy(data); changed[key] = value
            with self.subTest(key=key), self.assertRaises(AssertionError):
                self.contract(changed)
        changed = copy.deepcopy(data); changed["helpers"]["inbound_retry_perf.py"] = "0" * 64
        with self.assertRaises(AssertionError):
            self.contract(changed)
        valid = self.contract(data)
        self.assertEqual(valid["batched"]["correctness_tests"], [])
        self.assertEqual(valid["tests"], [])
        self.assertFalse(valid["failed_cpu_evidence"]["reexecute_unprofiled"])

    def profile(self, denied=False, report_failure=False, at_limit=False, report_probe_failure=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "test"; binary.write_bytes(b"exact binary")
            calls = []
            def bounded(command, _root, _out, name, timeout, _deadline, _target):
                calls.append((name, command, timeout))
                status = {"exit_code": 0, "stop_reason": None}
                if name == "perf-build-options":
                    return status, "libdw-dwarf-unwind: [ on ]"
                if name == "perf-record-help":
                    return status, "--call-graph --clockid --max-size --no-buildid-cache"
                if name in ("perf-permission-probe", "perf-sampled-test"):
                    self.assertEqual(command[command.index("-F") + 1], "199")
                    self.assertIn("cpu-clock:u", command); self.assertNotIn("-a", command)
                    if denied:
                        return {**status, "exit_code": 255}, "denied"
                    Path(command[command.index("-o") + 1]).write_bytes(b"raw capture")
                    if name == "perf-sampled-test":
                        self.assertEqual(timeout, 420)
                        self.assertEqual(command[command.index("--") + 1], str(binary))
                        status["exit_code"] = 101
                else:
                    self.assertIn("--no-inline", command); self.assertIn("--no-children", command)
                    self.assertIn("--stats", command); self.assertIn(str(8 * 1024**2), command)
                    with (root / f"{name}.log").open("wb") as stream:
                        if at_limit and name == "perf-report":
                            stream.truncate(8 * 1024**2)
                        else:
                            stream.write(b"sample and loss counts")
                    if report_failure and name == "perf-report":
                        status["exit_code"] = -25
                    if report_probe_failure and name == "perf-report-probe":
                        status["exit_code"] = 129
                return status, ""
            with patch.object(driver.perf.shutil, "which", return_value="/usr/bin/perf"):
                result = driver.perf.sample_failure([str(binary), "exact"], root, root,
                    time.time() + 1500, root, bounded, driver.retry.sha, **self.configured()["sampling"])
            return result, calls

    def test_one_199hz_capture_preserves_limits_and_raw_on_report_failure(self):
        result, calls = self.profile()
        self.assertEqual(result["status"], "sampled")
        self.assertFalse(result["cpu_acceptance"])
        self.assertEqual(sum(name == "perf-sampled-test" for name, *_ in calls), 1)
        self.assertEqual(result["maximum_data_bytes"], 32 * 1024**2)
        result, _ = self.profile(report_failure=True)
        self.assertIn("report_error", result); self.assertIn("data_sha256", result)
        result, _ = self.profile(at_limit=True)
        self.assertEqual(result["status"], "incomplete"); self.assertIn("data_sha256", result)

    def test_permission_denial_prevents_workload(self):
        result, calls = self.profile(denied=True)
        self.assertEqual(result["status"], "skipped")
        self.assertNotIn("perf-sampled-test", [name for name, *_ in calls])
        result, calls = self.profile(report_probe_failure=True)
        self.assertEqual(result["status"], "skipped")
        self.assertNotIn("perf-sampled-test", [name for name, *_ in calls])

    def test_report_file_limit_is_enforced_by_the_child(self):
        _, calls = self.profile()
        command = next(command for name, command, _ in calls if name == "perf-report")
        # Exercise the exact generated limiter, replacing only perf with a
        # deterministic writer; no profiling or application workload is run.
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report"
            writer = [*command[:4], driver.perf.sys.executable, "-c",
                      "import sys; sys.stdout.buffer.write(b'x'*(8*1024**2+1)); sys.stdout.flush()"]
            with path.open("wb") as stream:
                status = driver.subprocess.run(writer, stdout=stream, stderr=stream, timeout=5)
            self.assertNotEqual(status.returncode, 0)
            self.assertEqual(path.stat().st_size, 8 * 1024**2)

    def test_sampled_pass_and_cpu_failure_never_qualify_release(self):
        data = self.contract(self.configured())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); binary = root / "binary"; binary.write_bytes(b"exact")
            setting = root / "paranoid"; setting.write_text("2")
            driver.retry.save(root / "perf-permission.json", {"configured": True, "original": "4"})
            for percent, code, wrong_failure in ((4.0, 0, False), (6.0, 101, False), (6.0, 101, True)):
                test = data["batched"]["test"]
                text = f"running 1 test\ntest {test['name']} ... batched_retry_cpu {{}}\n"
                if code:
                    text += "thread 'test' panicked at fixture.rs:1: "
                    text += "other assertion failed\n" if wrong_failure else data["batched"]["failure_marker"] + "\n"
                text += "test result: " + ("FAILED. 0 passed; 1 failed" if code else "ok. 1 passed; 0 failed") + "; 0 ignored;\n"
                (root / "sampled.log").write_text(text)
                sampling = {"status": "sampled", "report": {"exit_code": 0, "stop_reason": None},
                    "run": {"exit_code": code, "stop_reason": None}, "test_log": "sampled.log"}
                with patch.object(driver, "PARANOID", setting), patch.object(driver.os, "geteuid", return_value=1001), \
                     patch.object(driver.perf, "sample_failure", return_value=sampling) as sample, \
                     patch.object(driver.retry, "batched_retry_metrics", return_value={"slices": [{"cpu_percent_one_core": percent}] * 5}), \
                     patch.object(driver.retry, "git", return_value=""):
                    args = (root, root, data, binary, {}, {}, {}, {}, {"deadline_unix": time.time() + 1500}, root, root)
                    if wrong_failure:
                        with self.assertRaises(AssertionError):
                            driver.sampled(*args)
                    else:
                        driver.sampled(*args)
                    sample.assert_called_once()
                receipt = json.loads((root / "profile-result.json").read_text())
                self.assertFalse(receipt["cpu_acceptance"])
                self.assertFalse(receipt["physical_cpu_acceptance"])
                self.assertEqual(receipt["diagnostic_complete"], not wrong_failure)

    def test_permission_original_saved_before_change_and_restored_after_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); setting = root / "paranoid"; setting.write_text("4")
            def change(command, **_):
                state = json.loads((root / "perf-permission.json").read_text())
                self.assertEqual(state["original"], "4")
                self.assertEqual(command[:3], ["sudo", "-n", "sysctl"])
                setting.write_text(command[-1].split("=")[1])
            with patch.object(driver, "PARANOID", setting), patch.object(driver.os, "geteuid", return_value=1001), \
                 patch.object(driver.subprocess, "run", side_effect=change):
                driver.permission(root)
                self.assertEqual(setting.read_text(), "2")
                driver.permission(root, restore=True)
                self.assertEqual(setting.read_text(), "4")
                self.assertTrue(json.loads((root / "perf-permission.json").read_text())["restored"])
            with patch.object(driver, "PARANOID", setting), patch.object(driver.os, "geteuid", return_value=1001), \
                 patch.object(driver.subprocess, "run", side_effect=RuntimeError("sudo failed")):
                with self.assertRaises(RuntimeError):
                    driver.permission(root)
                self.assertEqual(json.loads((root / "perf-permission.json").read_text())["original"], "4")

    def test_build_only_hook_runs_profile_once_and_never_qualification_tests(self):
        data = self.contract(self.configured())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); out = root / "out"; out.mkdir()
            target = root / "target"; target.mkdir()
            binary = target / "protocol"; binary.write_bytes(b"exact")
            artifact = {"reason": "compiler-artifact", "executable": str(binary),
                "target": {"name": "iris_chat_protocol"}, "profile": {"test": True,
                "opt_level": "3", "debug_assertions": False, "debuginfo": 1}}
            driver.retry.save(out / "preflight.json", {"inputs": data,
                "inputs_sha256": driver.retry.sha(driver.retry.INPUTS), "mode": "batched", "deadline_unix": time.time() + 1500})
            for build_code in (1, 0):
                calls = []
                def bounded(command, _root, _out, name, timeout, *_):
                    calls.append(name)
                    self.assertIn(name, ("fetch", "build"))
                    return {"exit_code": build_code if name == "build" else 0}, json.dumps(artifact)
                def git(_root, *args):
                    return "" if args == ("status", "--porcelain") else data["source"]
                with patch.object(driver.retry, "bounded", side_effect=bounded), \
                     patch.object(driver.retry, "git", side_effect=git), \
                     patch.object(driver.retry.subprocess, "check_output", side_effect=lambda cmd, **_: f"{cmd[0]} 1.99.1\n".encode()), \
                     patch.dict(driver.retry.os.environ, {"CARGO_TARGET_DIR": str(target)}), \
                     patch.object(driver, "sampled") as sampled:
                    if build_code:
                        with self.assertRaisesRegex(AssertionError, "Compilation failed"):
                            driver.retry.stage("batched", root, out, data,
                                source_prover=lambda *_: {"commit": data["source"]}, diagnostic_runner=sampled)
                        sampled.assert_not_called()
                    else:
                        driver.retry.stage("batched", root, out, data,
                            source_prover=lambda *_: {"commit": data["source"]}, diagnostic_runner=sampled)
                        sampled.assert_called_once()
                        self.assertEqual(sampled.call_args.args[0], binary)
                    self.assertEqual(calls, ["fetch", "build"])
                if build_code:
                    import shutil
                    shutil.rmtree(out / "batched")


if __name__ == "__main__":
    unittest.main()
