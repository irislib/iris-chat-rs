import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("diagnostic", Path(__file__).with_name("diagnose_ios_pagination.py"))
diagnostic = importlib.util.module_from_spec(spec)
spec.loader.exec_module(diagnostic)


class DiagnosticPreparationTests(unittest.TestCase):
    def test_local_entrypoint_refuses_before_any_external_command(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(diagnostic.subprocess, "check_output") as query:
            with self.assertRaises(SystemExit): diagnostic.main()
            query.assert_not_called()

    def test_three_independent_attempts_and_all_failures_are_retained(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            commands = []
            trace = root / "iris-pagination-timeline-pages-11111111-2222-4333-8444-555555555555.log"
            trace.write_text("iris.timeline_anchor_summary samples=1 truncated=false incomplete=false\n")

            def execute(command, directory, timeout):
                commands.append(command); directory.mkdir()
                return {"exitCode": 65 if directory.name == "iteration-1" else 0,
                        "timedOut": False, "command": command}

            def inventory(_): return {trace.name: trace}

            def retain(before, after, directory):
                return [{"file": trace.name, "summary": trace.read_text().splitlines()[0]}]

            with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}), patch.object(diagnostic, "ROOT", root), \
                    patch.object(diagnostic.subprocess, "check_output", side_effect=["a" * 40 + "\n", "platform=iOS Simulator,id=" + "1" * 36]), \
                    patch.object(diagnostic, "execute", side_effect=execute), \
                    patch.object(diagnostic, "trace_inventory", side_effect=inventory), \
                    patch.object(diagnostic, "retain_new_traces", side_effect=retain):
                self.assertEqual(diagnostic.main(), 1)
            summary = json.loads((root / "work/clipboard/pagination-diagnostic/summary.json").read_text())
            self.assertEqual([x["exitCode"] for x in summary["runs"]], [65, 0, 0])
            self.assertEqual([x["iteration"] for x in summary["runs"]], [1, 2, 3])
            self.assertEqual(len(commands), 5)
            for command in commands[2:]:
                self.assertIn("-only-testing:" + diagnostic.TEST, command)
                self.assertEqual(command[-1], "test-without-building")
                self.assertNotIn("-retry-tests-on-failure", command)

    def test_trace_copy_excludes_prior_runs_and_rejects_alias_or_oversize(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            path = root / "iris-pagination-timeline-pages-11111111-2222-4333-8444-555555555555.log"
            path.write_text("iris.timeline_anchor_summary samples=1 truncated=false incomplete=false\n")
            out = root / "evidence"; out.mkdir()
            self.assertEqual(diagnostic.retain_new_traces({path.name: path}, {path.name: path}, out), [])
            records = diagnostic.retain_new_traces({}, {path.name: path}, out)
            self.assertEqual(records[0]["sha256"], diagnostic.digest(path))
            alias = root / "alias"; alias.symlink_to(path)
            with self.assertRaises(ValueError): diagnostic.retain_new_traces({}, {path.name: alias}, out)
            path.write_bytes(b"x" * (2 * 1024 * 1024 + 1))
            with self.assertRaises(ValueError): diagnostic.retain_new_traces({}, {path.name: path}, out)


if __name__ == "__main__": unittest.main()
