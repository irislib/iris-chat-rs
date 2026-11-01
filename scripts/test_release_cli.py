"""Exercise CLI packaging without compiling or downloading build inputs."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class LinuxCliReleaseTests(unittest.TestCase):
    def test_hosted_linux_build_and_install_check_cover_both_architectures(self):
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text().split("\n  cli:\n", 1)[1]
        for runner, target, platform in (
            ("ubuntu-24.04", "x86_64-unknown-linux-gnu", "linux/amd64"),
            ("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu", "linux/arm64"),
        ):
            entry = workflow.split(f"- runner: {runner}\n", 1)[1].split("    runs-on:", 1)[0].split("          - runner:", 1)[0]
            self.assertIn(f"target: {target}", entry)
            self.assertIn(f"docker_platform: {platform}", entry)
        self.assertIn('IRIS_CLI_LINUX_DOCKER_BUILD=1 ./scripts/cli-release "$target"', workflow)
        self.assertIn('if [[ "$target" == *-unknown-linux-gnu ]]; then', workflow)
        self.assertIn('--archive "$archive" --platform "${{ matrix.docker_platform }}"', workflow)

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / "chat"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / "core").mkdir()
        for name in ("cli-release", "build_common.sh", "docker_stage_common.sh"):
            shutil.copy2(ROOT / "scripts" / name, self.root / "scripts" / name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "docker.json"
        docker = self.bin / "docker"
        docker.write_text(f"#!{sys.executable}\n" + '''import json, os, pathlib, sys
args = sys.argv[1:]
pathlib.Path(os.environ['CLI_TEST_CALLS']).write_text(json.dumps(args))
if os.environ.get('CLI_TEST_BUILD_FAIL'):
    sys.exit(23)
target = next(a.split('=', 1)[1] for a in args if a.startswith('CARGO_TARGET_DIR='))
root = pathlib.Path(os.environ['CLI_TEST_ROOT'])
relative = target.split('/target/', 1)[1]
binary = root / 'target' / relative / 'release/iris'
binary.parent.mkdir(parents=True)
binary.write_text('#!/bin/sh\\nprintf "iris fixture\\n"\\n')
binary.chmod(0o755)
''')
        docker.chmod(0o755)
        uname = self.bin / "uname"
        uname.write_text('#!/bin/sh\ncase "$1" in -s) echo "$CLI_TEST_OS";; -m) echo "$CLI_TEST_ARCH";; esac\n')
        uname.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        CLI_TEST_ROOT=str(self.root), CLI_TEST_CALLS=str(self.calls),
                        CLI_TEST_OS="Linux", CLI_TEST_ARCH="aarch64",
                        IRIS_CLI_LINUX_DOCKER_BUILD="1", IRIS_RELEASE_ENV_FILE="/dev/null",
                        IRIS_APP_VERSION_NAME="2026.11.1", IRIS_BUILD_GIT_SHA="a" * 12,
                        IRIS_BUILD_TIMESTAMP_UTC="2026-11-01T00:00:00Z")
        for key in ("IRIS_CLI_LINUX_DOCKER_PLATFORM", "IRIS_DOCKER_PLATFORM"):
            self.env.pop(key, None)

    def build(self, target, **environment):
        return subprocess.run(["bash", str(self.root / "scripts/cli-release"), target],
                              env={**self.env, **environment}, capture_output=True, text=True)

    def test_each_linux_archive_uses_matching_platform_and_separate_cache(self):
        for target, platform, cache in (
            ("aarch64-unknown-linux-gnu", "linux/arm64", "linux-arm64"),
            ("x86_64-unknown-linux-gnu", "linux/amd64", "linux-x64"),
        ):
            with self.subTest(target=target):
                result = self.build(target)
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = json.loads(self.calls.read_text())
                self.assertEqual(calls[calls.index("--platform") + 1], platform)
                self.assertIn(f"IRIS_CLI_TARGET={target}", calls)
                self.assertTrue(any(a.endswith(f"target/cli-release/{cache}") for a in calls))
                self.assertIn('rustc -vV', calls[-1])
                self.assertIn('$IRIS_CLI_TARGET', calls[-1])
                self.assertIn('--locked', calls[-1])
                with tarfile.open(self.root / f"dist/cli/iris-{target}.tar.gz") as archive:
                    self.assertTrue({"iris/iris", "iris/install.sh", "iris/README.txt"} <= set(archive.getnames()))
                    self.assertTrue(archive.getmember("iris/iris").mode & 0o111)

    def test_conflicting_platform_override_fails_before_build(self):
        for override in ("IRIS_CLI_LINUX_DOCKER_PLATFORM", "IRIS_DOCKER_PLATFORM"):
            with self.subTest(override=override):
                result = self.build("aarch64-unknown-linux-gnu", **{override: "linux/amd64"})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("platform", result.stderr)
                self.assertFalse(self.calls.exists())

    def test_failed_build_does_not_package_old_binary(self):
        result = self.build("aarch64-unknown-linux-gnu", CLI_TEST_BUILD_FAIL="1")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertFalse((self.root / "dist/cli").exists())

    def test_wrong_container_compiler_is_rejected_before_dependency_install(self):
        self.assertEqual(self.build("aarch64-unknown-linux-gnu").returncode, 0)
        command = json.loads(self.calls.read_text())[-1]
        compiler = self.bin / "rustc"
        compiler.write_text('#!/bin/sh\nprintf "host: x86_64-unknown-linux-gnu\\n"\n')
        compiler.chmod(0o755)
        result = subprocess.run(["bash", "-c", command], cwd=self.root / "core",
                                env={**self.env, "IRIS_CLI_TARGET": "aarch64-unknown-linux-gnu"},
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Container compiler does not match", result.stderr)
        self.assertNotIn("pinned-apt.sh", result.stderr)

    def test_linux_arm_archive_is_inspected_not_executed_on_macos_arm(self):
        result = self.build("aarch64-unknown-linux-gnu", CLI_TEST_OS="Darwin", CLI_TEST_ARCH="arm64")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("iris fixture", result.stdout)


if __name__ == "__main__":
    unittest.main()
