#!/usr/bin/env python3

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class MacosStaplerTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.app = self.root / "Iris Chat.app"
        self.app.mkdir()
        self.state = self.root / "state.json"
        tool = self.bin / "fake-tool"
        tool.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "path = Path(os.environ['IRIS_STAPLER_TEST_STATE'])\n"
            "state = json.loads(path.read_text())\n"
            "name = Path(sys.argv[0]).name\n"
            "args = sys.argv[1:]\n"
            "code = 0\n"
            "if name == 'sleep':\n"
            "    state['sleeps'].append(args)\n"
            "elif name == 'xcrun' and args[:2] == ['notarytool', 'submit']:\n"
            "    state['submissions'] += 1\n"
            "    print(json.dumps({'status': state['notaryStatus'], 'id': 'fixture'}))\n"
            "elif name == 'xcrun' and args[:2] == ['notarytool', 'log']:\n"
            "    print('Notarization fixture rejected')\n"
            "elif name == 'xcrun':\n"
            "    state['calls'].append(args)\n"
            "    code = state['codes'].pop(0)\n"
            "    print('Ticket service output')\n"
            "    print('Ticket service diagnostic', file=sys.stderr)\n"
            "elif name == 'ditto':\n"
            "    Path(args[-1]).touch()\n"
            "elif name != 'codesign':\n"
            "    raise SystemExit('Unexpected fixture command')\n"
            "path.write_text(json.dumps(state))\n"
            "raise SystemExit(code)\n"
        )
        tool.chmod(0o755)
        for name in ("xcrun", "sleep", "codesign", "ditto"):
            (self.bin / name).symlink_to(tool)
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            "IRIS_STAPLER_TEST_STATE": str(self.state),
            "TMPDIR": str(self.root),
        }

    def initialize(self, codes, notary_status="Accepted"):
        self.state.write_text(json.dumps({
            "codes": codes, "calls": [], "sleeps": [],
            "submissions": 0, "notaryStatus": notary_status,
        }))

    def run_helper(self, codes, action="staple"):
        self.initialize(codes)
        result = subprocess.run(
            [str(ROOT / "scripts/macos-stapler"), action, str(self.app)],
            env=self.env, capture_output=True, text=True, timeout=10,
        )
        return result, json.loads(self.state.read_text())

    def test_success_needs_one_call_and_no_delay(self):
        result, state = self.run_helper([0], "validate")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state["calls"], [["stapler", "validate", str(self.app)]])
        self.assertEqual(state["sleeps"], [])

    def test_transient_ticket_lookup_retries_the_same_artifact(self):
        result, state = self.run_helper([68, 0])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state["calls"], [["stapler", "staple", str(self.app)]] * 2)
        self.assertEqual(state["sleeps"], [["5"]])
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr.count("Ticket service output"), 2)
        self.assertIn("retry", result.stderr.lower())

    def test_persistent_service_failure_stays_a_failure(self):
        result, state = self.run_helper([68] * 4)
        self.assertEqual(result.returncode, 68)
        self.assertEqual(len(state["calls"]), 4)
        self.assertEqual(state["sleeps"], [["5"], ["15"], ["30"]])
        self.assertIn("Ticket service diagnostic", result.stderr)

    def test_other_errors_are_not_retried(self):
        for code in (65, 66, 77, 130):
            with self.subTest(code=code):
                result, state = self.run_helper([code], "validate")
                self.assertEqual(result.returncode, code)
                self.assertEqual(len(state["calls"]), 1)
                self.assertEqual(state["sleeps"], [])

    def test_nonretryable_error_after_service_failure_stops(self):
        result, state = self.run_helper([68, 65])
        self.assertEqual(result.returncode, 65)
        self.assertEqual(len(state["calls"]), 2)
        self.assertEqual(state["sleeps"], [["5"]])

    def run_notarization(self, codes, notary_status="Accepted"):
        repo = self.root / "repo"
        scripts = repo / "scripts"
        scripts.mkdir(parents=True)
        for name in ("macos-build", "macos-stapler"):
            shutil.copy2(ROOT / "scripts" / name, scripts / name)
        app = repo / "dist/macos/Iris Chat.app"
        app.mkdir(parents=True)
        self.initialize(codes, notary_status)
        env = {**self.env, "MACOS_NOTARY_PROFILE": "fixture"}
        for key in ("IRIS_ASC_AUTH_KEY_PATH", "IRIS_ASC_AUTH_KEY_ID", "IRIS_ASC_AUTH_KEY_ISSUER_ID"):
            env.pop(key, None)
        # The packager suppresses this function's stdout. Failures must remain
        # visible while retries reuse the signed app and its accepted submission.
        result = subprocess.run(
            [str(scripts / "macos-build"), "macos-notarize"],
            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            text=True, timeout=10,
        )
        return result, json.loads(self.state.read_text()), app

    def test_production_notarization_retries_without_resubmitting(self):
        result, state, app = self.run_notarization([68, 0, 0])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state["submissions"], 1)
        self.assertEqual(state["calls"], [
            ["stapler", "staple", str(app)],
            ["stapler", "staple", str(app)],
            ["stapler", "validate", str(app)],
        ])
        self.assertIn("Ticket service output", result.stderr)

    def test_rejected_notarization_never_reaches_stapling(self):
        result, state, _ = self.run_notarization([], "Invalid")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["submissions"], 1)
        self.assertEqual(state["calls"], [])
        self.assertIn("notarization status was 'Invalid'", result.stderr)

    def test_production_validation_retries_without_restapling(self):
        result, state, app = self.run_notarization([0, 68, 0])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state["submissions"], 1)
        self.assertEqual(state["calls"], [
            ["stapler", "staple", str(app)],
            ["stapler", "validate", str(app)],
            ["stapler", "validate", str(app)],
        ])

    def test_production_persistent_failure_stops_before_validation(self):
        result, state, app = self.run_notarization([68] * 4)
        self.assertEqual(result.returncode, 68)
        self.assertEqual(state["submissions"], 1)
        self.assertEqual(state["calls"], [["stapler", "staple", str(app)]] * 4)
        self.assertIn("Ticket service diagnostic", result.stderr)


if __name__ == "__main__":
    unittest.main()
