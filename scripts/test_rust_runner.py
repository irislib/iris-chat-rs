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
        commands = [[
            "test", "--manifest-path", str(ROOT / "core/vendor/webrtc-ice/Cargo.toml"),
            "--locked", "--lib", "agent::agent_idle_regression_test",
        ], [
            "test", "--manifest-path", str(ROOT / "core/vendor/webrtc-ice/Cargo.toml"),
            "--locked", "--lib", "candidate::candidate_equality_test",
        ]]
        for crate in CRATES:
            args = ["--manifest-path", str(ROOT / crate / "Cargo.toml"), "--locked"]
            if nextest:
                commands.append(["nextest", "run", "--no-fail-fast", *args])
                commands.append(["test", "-q", "--doc", *args])
            else:
                commands.append(["test", "-q", *args])
        return commands

    def run_runner(self, nextest, target=None, fail_on=None, approval_relay=None, fail_relay=False):
        with tempfile.TemporaryDirectory(prefix="iris-rust-runner-") as tmp:
            directory = pathlib.Path(tmp)
            binary_dir = directory / "bin"
            binary_dir.mkdir()
            # Isolate PATH so the fallback test cannot discover a real nextest.
            for command in ("dirname", "mktemp", "rm"):
                (binary_dir / command).symlink_to(shutil.which(command))
            python = binary_dir / "python3"
            python.write_text(
                '#!/bin/sh\n'
                'if [ "$1" = "-" ]; then echo 45678; exit; fi\n'
                'printf "%s\\t" "$@" >> "$IRIS_TEST_RELAY_LOG"\n'
                'printf "\\n" >> "$IRIS_TEST_RELAY_LOG"\n'
                'if [ "$2" = "start" ] && [ "$IRIS_TEST_RELAY_FAIL" = "1" ]; then exit 37; fi\n'
            )
            python.chmod(0o755)
            cargo = binary_dir / "cargo"
            cargo.write_text(
                '#!/bin/sh\n'
                '[ "$IRIS_DEVICE_APPROVAL_RELAY_URL" = "$IRIS_TEST_EXPECTED_RELAY" ] || exit 38\n'
                'printf "%s\\t" "$PWD" "$CARGO_TARGET_DIR" "$@" >> "$IRIS_TEST_LOG"\n'
                'printf "\\n" >> "$IRIS_TEST_LOG"\n'
                '[ "$*" != "${IRIS_TEST_FAIL_COMMAND:-}" ] || exit 23\n'
            )
            cargo.chmod(0o755)
            if nextest:
                (binary_dir / "cargo-nextest").symlink_to(cargo)
            log = directory / "commands"
            relay_log = directory / "relay-commands"
            env = dict(
                os.environ, PATH=str(binary_dir), IRIS_TEST_LOG=str(log),
                IRIS_TEST_RELAY_LOG=str(relay_log), IRIS_TEST_RELAY_FAIL=str(int(fail_relay)),
                IRIS_TEST_EXPECTED_RELAY=approval_relay or "ws://127.0.0.1:45678",
            )
            env.pop("CARGO_TARGET_DIR", None)
            env.pop("IRIS_DEVICE_APPROVAL_RELAY_URL", None)
            if approval_relay is not None:
                env["IRIS_DEVICE_APPROVAL_RELAY_URL"] = approval_relay
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
            calls = [line.rstrip("\t").split("\t") for line in log.read_text().splitlines()] if log.exists() else []
            for cwd, actual_target, *_ in calls:
                self.assertEqual(cwd, str(ROOT / "core"))
                self.assertEqual(actual_target, target or str(ROOT / "core/target"))
            if approval_relay:
                self.assertFalse(relay_log.exists(), "must not start or stop a caller-owned relay")
            else:
                relay_calls = [line.rstrip("\t").split("\t") for line in relay_log.read_text().splitlines()]
                self.assertEqual([call[1] for call in relay_calls], ["start", "stop"])
                start, stop = relay_calls
                self.assertEqual(start[start.index("--bind") + 1], "127.0.0.1:45678")
                pid_file = start[start.index("--pid-file") + 1]
                self.assertEqual(stop[stop.index("--pid-file") + 1], pid_file)
                self.assertFalse(pathlib.Path(pid_file).parent.exists(), "fixture directory was not removed")
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

    def test_preserves_explicit_approval_relay_without_managing_it(self):
        result, calls = self.run_runner(True, approval_relay="ws://127.0.0.1:4888")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, self.commands(True))

    def test_failed_relay_readiness_stops_fixture_before_any_tests(self):
        result, calls = self.run_runner(True, fail_relay=True)
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertEqual(calls, [])

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
