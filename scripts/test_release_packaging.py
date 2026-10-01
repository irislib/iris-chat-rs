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
        return subprocess.run([str(self.guard)], env=self.env, capture_output=True, text=True)

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


if __name__ == "__main__":
    unittest.main()
