import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile

import verify_retry_decode as driver


class DecodeTests(unittest.TestCase):
    def configured(self):
        admission = json.loads(driver.INPUTS.read_text()); admission["enabled"] = True
        return {**driver.profile.contract(), "decode": admission}

    def test_disabled_or_changed_source_and_helper_binding_stop(self):
        admission = self.configured()["decode"]
        for key, value in (("enabled", False), ("source", "a" * 40), ("profile_helper_sha256", "0" * 64),
                           ("binary_sha256", "0" * 64), ("cpu_acceptance", True)):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as directory:
                changed = copy.deepcopy(admission); changed[key] = value
                path = Path(directory) / "input.json"; path.write_text(json.dumps(changed))
                with patch.object(driver, "INPUTS", path), self.assertRaises(AssertionError):
                    driver.contract()

    def execute(self, failure=None):
        data = self.configured()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); out = root / "out"; output = out / "batched"; output.mkdir(parents=True)
            target = root / "target"; target.mkdir(); binary = target / "binary"; binary.write_bytes(b"same executable")
            upstream = data["decode"]["upstream"]
            prior = {"source": {"commit": data["source"], "tree": data["tree"]},
                     "binary_sha256": data["decode"]["binary_sha256"], "cpu_acceptance": False,
                     "sampling": {"run": {"command": ["perf", "record", "--", str(binary)]}}}
            raw = b"recorded capture only"; stats = b"SAMPLE events: 4192\n"
            prior_bytes = json.dumps(prior).encode()
            compressed = io.BytesIO()
            with zipfile.ZipFile(compressed, "w") as z:
                z.writestr("batched/whole-retry.perf.data", raw)
                z.writestr("profile-result.json", prior_bytes)
                z.writestr("batched/perf-report.log", stats)
                z.writestr("preflight.json", json.dumps({"driver_source": upstream["driver"]}))
            archive = compressed.getvalue()
            digest = lambda b: hashlib.sha256(b).hexdigest()
            upstream.update(zip_bytes=len(archive), zip_sha256=digest(archive), raw_bytes=len(raw),
                            raw_sha256=digest(raw), profile_result_sha256=digest(prior_bytes), statistics_sha256=digest(stats))
            if failure == "raw":
                upstream["raw_sha256"] = "0" * 64
            calls = []
            def bounded(command, _root, _out, name, timeout, *_):
                calls.append((name, command))
                status = {"exit_code": 0, "stop_reason": None}
                if name == "upstream-run":
                    return status, json.dumps({"id": upstream["run"], "head_sha": "wrong" if failure == "run" else upstream["driver"],
                                              "run_attempt": 1, "status": "completed", "conclusion": "failure"})
                if name == "upstream-artifact":
                    return status, json.dumps({"id": upstream["artifact"], "workflow_run": {"id": upstream["run"], "head_sha": upstream["driver"]},
                        "expired": False, "name": f"retry-profile-{upstream['run']}", "size_in_bytes": upstream["zip_bytes"],
                        "digest": "sha256:" + upstream["zip_sha256"]})
                if name == "upstream-download":
                    (target.parent / "retry-profile-input/upstream.zip").write_bytes(b"wrong" if failure == "zip" else archive)
                    return status, ""
                self.assertEqual(name, "perf-callers")
                self.assertEqual(timeout, 60)
                self.assertIn("report", command); self.assertNotIn("--stats", command)
                self.assertNotIn("record", command); self.assertNotIn(str(binary), command)
                self.assertIn("--no-inline", command); self.assertIn("--no-children", command)
                self.assertIn("--show-nr-samples", command); self.assertIn(str(8 * 1024**2), command)
                text = "Aggregated stats only" if failure == "stats_only" else "# Overhead Samples Symbol\n"
                (output / "perf-callers.log").write_text(text)
                if failure == "report":
                    status["exit_code"] = -25
                return status, text
            actual_sha = driver.retry.sha
            def sha(path):
                if path == binary:
                    return "bad" if failure == "binary" else data["decode"]["binary_sha256"]
                return actual_sha(path)
            with patch.object(driver.retry, "bounded", side_effect=bounded), patch.object(driver.retry, "sha", side_effect=sha), \
                 patch.object(driver.retry, "git", return_value=""), patch.object(driver.shutil, "which", return_value="/usr/bin/perf"):
                args = (root, out, data, binary, {}, {}, {}, {}, {"deadline_unix": time.time()+1500}, output, target)
                if failure:
                    with self.assertRaises(AssertionError):
                        driver.decoded(*args)
                else:
                    driver.decoded(*args)
            receipt = json.loads((out / "decode-result.json").read_text())
            self.assertFalse(receipt["cpu_acceptance"])
            self.assertEqual((receipt["test_executions"], receipt["recording_executions"], receipt["permission_changes"]), (0, 0, 0))
            self.assertEqual(receipt["diagnostic_complete"], failure is None)
            return [name for name, _ in calls]

    def test_valid_artifact_and_same_binary_decode_once_without_workload(self):
        calls = self.execute()
        self.assertEqual(calls, ["upstream-run", "upstream-artifact", "upstream-download", "perf-callers"])

    def test_wrong_binary_stops_before_download_or_decode(self):
        self.assertEqual(self.execute("binary"), [])

    def test_wrong_upstream_or_archive_or_raw_stops_before_decode(self):
        for failure in ("run", "zip", "raw"):
            with self.subTest(failure=failure):
                self.assertNotIn("perf-callers", self.execute(failure))

    def test_stats_only_or_report_failure_is_not_success(self):
        for failure in ("stats_only", "report"):
            with self.subTest(failure=failure):
                self.assertEqual(self.execute(failure).count("perf-callers"), 1)


if __name__ == "__main__":
    unittest.main()
