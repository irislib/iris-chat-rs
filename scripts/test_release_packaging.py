#!/usr/bin/env python3

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ReleasePublishGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "scripts").mkdir()
        self.guard = self.root / "scripts/test_release_publish_guard"
        shutil.copy2(ROOT / "scripts/test_release_publish_guard", self.guard)
        distributor = self.root / "scripts/distribute"
        distributor.write_text("#!/bin/bash\nexit 1\n")
        distributor.chmod(0o755)
        # Python 3.14 correctly rejects empty discovery; reach the scan through
        # the same nonempty test gate that the real repository uses.
        (self.root / "scripts/test_fixture.py").write_text(
            "import unittest\n"
            "from pathlib import Path\n\n"
            "class FixtureTests(unittest.TestCase):\n"
            "    def test_distribution_entrypoint_exists(self):\n"
            "        self.assertTrue(Path(__file__).with_name('distribute').is_file())\n"
        )
        (self.root / "scripts/distribution_common.sh").write_text("# clean\n")
        (self.root / "RELEASE.md").write_text("Release documentation.\n")
        binaries = self.root / "bin"
        binaries.mkdir()
        # Model hosted runners where Python and shell tools exist but rg does not.
        for name, source in (("python3", sys.executable), ("bash", "/bin/bash"),
                             ("dirname", shutil.which("dirname"))):
            (binaries / name).symlink_to(source)
        self.env = {**os.environ, "PATH": str(binaries)}

    def run_guard(self) -> subprocess.CompletedProcess:
        result = subprocess.run([str(self.guard)], env=self.env, capture_output=True, text=True)
        self.assertIn("Ran 1 test", result.stderr)
        self.assertIn("\nOK\n", result.stderr)
        return result

    def test_clean_inputs_pass_without_ripgrep(self) -> None:
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("command not found", result.stderr)

    def test_forbidden_paths_fail_without_ripgrep(self) -> None:
        for forbidden in ("IRIS_RELEASE_NOSTR_KEY_PATH", "IrisChat-release-latest",
                          "scripts/release --publish"):
            with self.subTest(forbidden=forbidden):
                (self.root / "RELEASE.md").write_text(forbidden + "\n")
                result = self.run_guard()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("forbidden legacy fallback", result.stderr)

    def test_missing_input_fails_closed(self) -> None:
        (self.root / "scripts/distribution_common.sh").unlink()
        result = self.run_guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FileNotFoundError", result.stderr)
        self.assertIn("distribution_common.sh", result.stderr)
        self.assertNotIn("Release publication contract passed", result.stdout)


class HomebrewPackagingTests(unittest.TestCase):
    def test_temporary_tap_waits_for_git_housekeeping(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            seed = root / "seed"
            assets = root / "assets"
            assets.mkdir()
            for target in ("aarch64-apple-darwin", "x86_64-apple-darwin",
                           "x86_64-unknown-linux-gnu"):
                (assets / f"iris-v9.8.7-{target}.tar.gz").write_bytes(target.encode())

            config = root / "gitconfig"
            config.write_text("[gc]\n\tauto = 1\n\tautoDetach = true\n"
                              "[maintenance]\n\tautoDetach = true\n")
            env = {**os.environ, "GIT_CONFIG_GLOBAL": str(config), "GIT_CONFIG_NOSYSTEM": "1"}

            def git(*args, **kwargs):
                return subprocess.run(
                    ["git", "-c", "gc.autoDetach=false", "-c", "maintenance.autoDetach=false", *args],
                    env=env, capture_output=True, check=True, **kwargs,
                )

            git("init", "-q", "-b", "master", str(seed))
            (seed / "README").write_text("Preserve the existing tap.\n")
            git("-C", str(seed), "add", "README")
            git("-C", str(seed), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "Seed tap")
            # Git samples loose-object bucket 17 for gc.auto. Ensure this tiny
            # fixture triggers real maintenance after the generated formula commit.
            added = 0
            for number in range(10000):
                data = f"loose object {number}".encode()
                digest = hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()
                if digest.startswith("17"):
                    git("-C", str(seed), "hash-object", "-w", "--stdin", input=data)
                    added += 1
                    if added == 5:
                        break
            self.assertEqual(added, 5)

            trace = root / "trace.json"
            result = subprocess.run(
                [str(ROOT / "packaging/homebrew/create_tap.sh"), "--version", "v9.8.7",
                 "--release-base-url", "https://example.invalid/assets", "--assets-dir", str(assets),
                 "--output-dir", str(root / "tap.git"), "--seed-repo", str(seed)],
                env={**env, "GIT_TRACE2_EVENT": str(trace)}, capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            maintenance = [event for event in map(json.loads, trace.read_text().splitlines())
                           if event.get("event") == "child_start"
                           and "maintenance" in event.get("argv", [])]
            self.assertTrue(maintenance, "fixture must exercise Git automatic maintenance")
            for event in maintenance:
                self.assertNotIn("--detach", event["argv"])
            git("--git-dir", str(root / "tap.git"), "fsck", "--strict")
            formula = git("--git-dir", str(root / "tap.git"), "show", "master:Formula/iris.rb")
            self.assertIn(b'class Iris < Formula', formula.stdout)


class ReleaseUpdaterVerificationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.receipt = self.root / "calls.jsonl"
        self.cli = self.root / "iris"
        self.cli.write_text(f"#!{sys.executable}\n" + '''
import json
import os
import sys
from pathlib import Path

data = Path(sys.argv[sys.argv.index("--data-dir") + 1])
app = "--app" in sys.argv
record = {
    "arguments": sys.argv[1:],
    "data_dir": str(data),
    "existing_files": sorted(path.name for path in data.iterdir()),
    "override_names": sorted(name for name in os.environ
                             if name.startswith(("IRIS_UPDATE_", "IRIS_FIPS_"))),
    "unrelated_setting": os.environ.get("IRIS_CHECK_FIXTURE_KEEP"),
}
with Path(os.environ["CHECK_FIXTURE_RECEIPT"]).open("a") as output:
    output.write(json.dumps(record) + "\\n")
(data / "warm-cache").write_text("must not reach the next check")
tag = os.environ.get("CHECK_FIXTURE_TAG", "v2026.10.1.7")
prefix = "iris-chat-" if app else "iris-"
print(json.dumps({"tag": tag, "verified": True,
                  "source": "hashtree-nostr-blossom", "asset": prefix + tag + "-fixture"}))
''')
        self.cli.chmod(0o755)
        self.env = {
            **os.environ,
            "CHECK_FIXTURE_RECEIPT": str(self.receipt),
            "IRIS_UPDATE_TEST_REFERENCE": "test-reference",
            "IRIS_FIPS_TEST_ENDPOINT": "test-endpoint",
            "IRIS_CHECK_FIXTURE_KEEP": "preserved",
        }

    def run_check(self, **environment: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(ROOT / "scripts/check-release-updater.py"),
             "--cli", str(self.cli), "--tag", "v2026.10.1.7"],
            env={**self.env, **environment}, capture_output=True, text=True, timeout=10,
        )

    def calls(self) -> list[dict]:
        return [json.loads(line) for line in self.receipt.read_text().splitlines()]

    def test_removes_both_override_families_but_keeps_unrelated_settings(self) -> None:
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertEqual(call["override_names"], [])
            self.assertEqual(call["unrelated_setting"], "preserved")

    def test_cli_and_app_each_start_with_an_independent_empty_directory(self) -> None:
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        self.assertNotEqual(calls[0]["data_dir"], calls[1]["data_dir"])
        for index, call in enumerate(calls):
            self.assertEqual(call["existing_files"], [])
            self.assertEqual(call["arguments"], ["--data-dir", call["data_dir"],
                             "update", "check", "--source", "hashtree", "--json"]
                             + (["--app"] if index else []))
            self.assertFalse(Path(call["data_dir"]).exists())

    def test_wrong_release_still_fails_closed_and_cleans_its_directory(self) -> None:
        result = self.run_check(CHECK_FIXTURE_TAG="v2026.10.1.6")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("different or unverified release", result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 1)
        self.assertFalse(Path(calls[0]["data_dir"]).exists())


if __name__ == "__main__":
    unittest.main()
