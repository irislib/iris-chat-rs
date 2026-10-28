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
            "stage = 'INPUTS_EXIT' if ':irisReleaseInputs' in sys.argv else 'RESOURCE_EXIT' if ':app:processReleaseResources' in sys.argv else 'PREFLIGHT_EXIT' if '--dry-run' in sys.argv else 'BUILD_EXIT'\n"
            "if stage == 'INPUTS_EXIT':\n"
            "    assert os.environ['IRIS_ANDROID_DEPENDENCY_PHASE'] == 'preflight'\n"
            "    assert not os.path.exists(os.environ['IRIS_ANDROID_DEPENDENCY_REPORT'])\n"
            "sys.exit(int(os.environ[stage]))\n"
        )
        gradle.chmod(0o755)
        self.env = {**os.environ, "CALL_LOG": str(self.log), "INPUTS_EXIT": "0", "RESOURCE_EXIT": "0", "PREFLIGHT_EXIT": "0", "BUILD_EXIT": "42",
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

    def test_preflight_resolves_dependencies_and_resources_before_both_graphs(self):
        result, calls = self.run_release("release-preflight")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 3)
        self.assertIn(":irisReleaseInputs", calls[0])
        self.assertIn("--no-parallel", calls[0])
        self.assertNotIn("--dry-run", calls[0])
        self.assertEqual(calls[1][-4:], ["-x", ":app:generateRustBindings",
                                       ":app:processReleaseResources", ":app:processSelfHostedResources"])
        self.assertNotIn("--dry-run", calls[1])
        self.assertEqual(calls[2][-3:], ["--dry-run", ":app:bundleRelease", ":app:assembleSelfHosted"])
        for call in calls:
            self.assertEqual(call[:4], ["--max-workers=2", "-Dorg.gradle.jvmargs=-Xmx4g -Dfile.encoding=UTF-8",
                                       "--dependency-verification", "strict"])
            self.assertNotIn("--write-locks", call)
            self.assertNotIn("--write-verification-metadata", call)

    def test_failed_preflight_prevents_build(self):
        for count, stage in enumerate(("INPUTS_EXIT", "RESOURCE_EXIT", "PREFLIGHT_EXIT"), 1):
            with self.subTest(stage=stage):
                self.log.unlink(missing_ok=True)
                self.env[stage] = "37"
                result, calls = self.run_release("release-artifacts")
                self.assertEqual(result.returncode, 37, result.stderr)
                self.assertEqual(len(calls), count)
                self.env[stage] = "0"

    def test_build_uses_identical_launch_and_tasks_after_preflight(self):
        result, calls = self.run_release("release-artifacts")
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertEqual(len(calls), 4)
        self.assertEqual([arg for arg in calls[2] if arg != "--dry-run"], calls[3])
        self.assertNotIn("-x", calls[3])


if __name__ == "__main__":
    unittest.main()
