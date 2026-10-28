#!/usr/bin/env python3
"""Check release preflight scope and failure propagation without build tools."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class AndroidReleasePreflightTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "scripts").mkdir()
        (self.root / "android").mkdir()
        for name in ("android-release", "build_common.sh"):
            shutil.copy2(ROOT / "scripts" / name, self.root / "scripts" / name)
        (self.root / "android/local.properties").write_text("sdk.dir=unused\n")
        self.log = self.root / "gradle-calls.jsonl"
        gradle = self.root / "android/gradlew"
        gradle.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "with open(os.environ['CALL_LOG'], 'a') as f: f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "sys.exit(int(os.environ['PREFLIGHT_EXIT'] if '--dry-run' in sys.argv else os.environ['BUILD_EXIT']))\n"
        )
        gradle.chmod(0o755)
        self.env = {**os.environ, "CALL_LOG": str(self.log), "PREFLIGHT_EXIT": "0", "BUILD_EXIT": "42",
                    "IRIS_RELEASE_ENV_FILE": str(self.root / "absent.env"),
                    "IRIS_APP_VERSION_NAME": "2026.10.28.2", "IRIS_APP_VERSION_CODE": "2026102802",
                    "IRIS_BUILD_GIT_SHA": "a" * 40, "IRIS_BUILD_TIMESTAMP_UTC": "2026-10-28T00:00:00Z",
                    "IRIS_RELEASE_KEYSTORE_PATH": "unused", "IRIS_RELEASE_KEYSTORE_PASSWORD": "unused",
                    "IRIS_RELEASE_KEY_ALIAS": "unused", "IRIS_RELEASE_KEY_PASSWORD": "unused"}

    def run_release(self, mode):
        result = subprocess.run([str(self.root / "scripts/android-release"), mode],
                                env=self.env, text=True, capture_output=True, timeout=10)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        self.assertFalse((self.root / "dist").exists())
        return result, calls

    def test_preflight_only_evaluates_both_release_graphs_strictly(self):
        result, calls = self.run_release("release-preflight")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0], ["--max-workers=2", "-Dorg.gradle.jvmargs=-Xmx4g -Dfile.encoding=UTF-8",
                                    "--dependency-verification", "strict", "--dry-run",
                                    ":app:bundleRelease", ":app:assembleSelfHosted"])

    def test_failed_preflight_prevents_build(self):
        self.env["PREFLIGHT_EXIT"] = "37"
        result, calls = self.run_release("release-artifacts")
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertIn("--dry-run", calls[0])

    def test_build_uses_identical_launch_and_tasks_after_preflight(self):
        result, calls = self.run_release("release-artifacts")
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertEqual(len(calls), 2)
        self.assertEqual([arg for arg in calls[0] if arg != "--dry-run"], calls[1])


if __name__ == "__main__":
    unittest.main()
