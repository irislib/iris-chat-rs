import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import verify_protocol_checkpoint as driver


class ProtocolCheckpointTests(unittest.TestCase):
    def configured(self):
        data = json.loads(driver.INPUTS.read_text())
        data.update(enabled=True, source="a" * 40, tree="b" * 40, ordinary_passed=191)
        return data

    def contract(self, data):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "inputs.json"
            path.write_text(json.dumps(data))
            with patch.object(driver, "INPUTS", path):
                return driver.contract()

    def test_disabled_or_unbound_source_cannot_start(self):
        data = self.configured()
        for field, value in (("enabled", False), ("source", None), ("tree", None),
                             ("ordinary_passed", None), ("resource_helper_sha256", "0" * 64)):
            changed = copy.deepcopy(data); changed[field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.contract(changed)

    def test_plan_is_whole_suite_then_backlog_then_single_cpu(self):
        data = self.contract(self.configured())
        ordinary, backlog = data["batched"]["correctness_tests"]
        self.assertEqual((ordinary["name"], ordinary["exact"], ordinary["ignored"]), ("", False, False))
        self.assertEqual((ordinary["expected_count"], ordinary["expected_ignored"]), (191, 3))
        self.assertEqual((backlog["name"], backlog["exact"], backlog["ignored"]), (driver.BACKLOG, True, True))
        self.assertEqual([ordinary["timeout_seconds"], backlog["timeout_seconds"], data["batched"]["test"]["timeout_seconds"]], [300, 300, 420])
        self.assertEqual({key: data["batched"][key] for key in ("test", "failure_marker", "schema_version")}, driver.checkpoint.cpu_plan())
        self.assertFalse(data["failed_cpu_evidence"]["reexecute"])

    def test_whole_suite_requires_exact_pass_ignore_and_zero_filter_counts(self):
        test = self.contract(self.configured())["batched"]["correctness_tests"][0]
        valid = "running 194 tests\ntest result: ok. 191 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out;\n"
        self.assertEqual(driver.check_result(valid, test, 0)["passed"], 191)
        for text, code in ((valid, 101), (valid.replace("191 passed", "190 passed"), 0),
                           (valid.replace("3 ignored", "0 ignored"), 0),
                           (valid.replace("0 filtered", "1 filtered"), 0),
                           (valid + valid, 0), (valid.replace("running 194", "running 191"), 0)):
            with self.subTest(text=text, code=code), self.assertRaises(AssertionError):
                driver.check_result(text, test, code)

    def test_failed_focused_stage_never_runs_acceptance_parser(self):
        with patch.object(driver.retry, "stage", side_effect=AssertionError("focused failure")), \
             patch.object(driver.retry, "bounded") as bounded, \
             patch.object(driver.retry, "INPUTS", driver.retry.INPUTS):
            with self.assertRaisesRegex(AssertionError, "focused failure"):
                driver.run("batched", Path("unused"), Path("unused"), self.contract(self.configured()))
            bounded.assert_not_called()


if __name__ == "__main__":
    unittest.main()
