import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
CRATES = ("core", "chat-protocol", "protocol-ffi")


class RustRunnerTests(unittest.TestCase):
    def commands(self, nextest):
        commands = []
        for crate in CRATES:
            args = ["--manifest-path", str(ROOT / crate / "Cargo.toml"), "--locked"]
            if nextest:
                commands.append(["nextest", "run", *args])
                commands.append(["test", "-q", "--doc", *args])
            else:
                commands.append(["test", "-q", *args])
        return commands

    def run_runner(self, nextest, target=None, fail_on=None):
        with tempfile.TemporaryDirectory(prefix="iris-rust-runner-") as tmp:
            directory = pathlib.Path(tmp)
            binary_dir = directory / "bin"
            binary_dir.mkdir()
            # Isolate PATH so the fallback test cannot discover a real nextest.
            (binary_dir / "dirname").symlink_to(shutil.which("dirname"))
            cargo = binary_dir / "cargo"
            cargo.write_text(
                '#!/bin/sh\n'
                'printf "%s\\t" "$PWD" "$CARGO_TARGET_DIR" "$@" >> "$IRIS_TEST_LOG"\n'
                'printf "\\n" >> "$IRIS_TEST_LOG"\n'
                '[ "$*" != "${IRIS_TEST_FAIL_COMMAND:-}" ] || exit 23\n'
            )
            cargo.chmod(0o755)
            if nextest:
                (binary_dir / "cargo-nextest").symlink_to(cargo)
            log = directory / "commands"
            env = dict(os.environ, PATH=str(binary_dir), IRIS_TEST_LOG=str(log))
            env.pop("CARGO_TARGET_DIR", None)
            if target is not None:
                env["CARGO_TARGET_DIR"] = target
            env["IRIS_TEST_FAIL_COMMAND"] = " ".join(fail_on or [])
            result = subprocess.run(
                [shutil.which("bash"), str(ROOT / "scripts/test_rust.sh")],
                cwd=directory,
                env=env,
                capture_output=True,
                text=True,
            )
            calls = [line.rstrip("\t").split("\t") for line in log.read_text().splitlines()]
            for cwd, actual_target, *_ in calls:
                self.assertEqual(cwd, str(ROOT / "core"))
                self.assertEqual(actual_target, target or str(ROOT / "core/target"))
            return result, [call[2:] for call in calls]

    def test_runs_all_crates_and_doc_tests_with_each_runner(self):
        for nextest in (True, False):
            with self.subTest(nextest=nextest):
                result, calls = self.run_runner(nextest)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, self.commands(nextest))

    def test_preserves_explicit_target_directory(self):
        for nextest in (True, False):
            with self.subTest(nextest=nextest):
                result, calls = self.run_runner(nextest, target="custom target")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, self.commands(nextest))

    def test_stops_and_propagates_failure_from_every_stage(self):
        for nextest in (True, False):
            commands = self.commands(nextest)
            for index, command in enumerate(commands):
                with self.subTest(nextest=nextest, stage=index):
                    result, calls = self.run_runner(nextest, fail_on=command)
                    self.assertEqual(result.returncode, 23, result.stderr)
                    self.assertEqual(calls, commands[:index + 1])


if __name__ == "__main__":
    unittest.main()
