import hashlib
import json
import os
import sys
import pathlib
import shutil
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
CRATES = ("core", "chat-protocol", "protocol-ffi")
MEMORY_TEST = "core::protocol::anonymous_reads::memory_tests::populated_account_discovery_under_memory_limit"
CHECKPOINT_TEST = "protocol_engine::tests::checkpoint_backlog_production_counts_are_durable"
CHECKPOINT_MARKER = "checkpoint_backlog fanouts=22702 ciphertexts=19231 checkpoint_bytes=254672896"
CHECKPOINT_RESULT = "test result: ok. 1 passed; 0 failed; 0 ignored;"


def checkpoint_command():
    return [
        "test", "--manifest-path", str(ROOT / "chat-protocol/Cargo.toml"), "--locked",
        "--lib", CHECKPOINT_TEST,
        "--", "--ignored", "--exact", "--test-threads=1", "--nocapture",
    ]


class RustRunnerTests(unittest.TestCase):
    def commands(self, nextest, platform="Darwin"):
        commands = []
        if platform == "Linux":
            commands.append([
                "test", "--manifest-path", str(ROOT / "core/Cargo.toml"), "--locked",
                "--lib", MEMORY_TEST, "--", "--exact", "--nocapture",
            ])
        for crate in CRATES:
            args = ["--manifest-path", str(ROOT / crate / "Cargo.toml"), "--locked"]
            if nextest:
                commands.append(["nextest", "run", "--no-fail-fast", *args])
                commands.append(["test", "-q", "--doc", *args])
            else:
                commands.append(["test", "-q", *args])
            if crate == "chat-protocol":
                commands.append(checkpoint_command())
        return commands

    def run_runner(self, nextest, target=None, fail_on=None, approval_relay=None,
                   fail_relay=False, platform="Darwin", cpu_receipt=None,
                   checkpoint_output="passed"):
        with tempfile.TemporaryDirectory(prefix="iris-rust-runner-") as tmp:
            directory = pathlib.Path(tmp)
            binary_dir = directory / "bin"
            binary_dir.mkdir()
            # Isolate PATH so the fallback test cannot discover a real nextest.
            for command in ("dirname", "mktemp", "rm"):
                (binary_dir / command).symlink_to(shutil.which(command))
            uname = binary_dir / "uname"
            uname.write_text('#!/bin/sh\nprintf "%s\\n" "$IRIS_TEST_PLATFORM"\n')
            uname.chmod(0o755)
            python = binary_dir / "python3"
            python.write_text(
                '#!/bin/sh\n'
                'if [ "$1" = "-" ]; then\n'
                '  if [ "$#" = 1 ]; then echo 45678; exit; fi\n'
                '  exec "$IRIS_TEST_REAL_PYTHON" "$@"\n'
                'fi\n'
                'printf "%s\\t" "$@" >> "$IRIS_TEST_RELAY_LOG"\n'
                'printf "\\n" >> "$IRIS_TEST_RELAY_LOG"\n'
                'if [ "$2" = "start" ] && [ "$IRIS_TEST_RELAY_FAIL" = "1" ]; then exit 37; fi\n'
            )
            python.chmod(0o755)
            cargo = binary_dir / "cargo"
            cargo.write_text(
                f'#!{sys.executable}\n'
                'import json, os, sys\n'
                'env = os.environ\n'
                'if env["IRIS_DEVICE_APPROVAL_RELAY_URL"] != env["IRIS_TEST_EXPECTED_RELAY"]: sys.exit(38)\n'
                'with open(env["IRIS_TEST_LOG"], "a") as log:\n'
                '    log.write("\\t".join([os.getcwd(), env["CARGO_TARGET_DIR"], *sys.argv[1:]]) + "\\n")\n'
                'if sys.argv[1:] == json.loads(env["IRIS_TEST_FAIL_COMMAND"]): sys.exit(23)\n'
                f'if sys.argv[1:] == {checkpoint_command()!r}:\n'
                '    mode = env["IRIS_TEST_CHECKPOINT_OUTPUT"]\n'
                '    if mode != "missing-marker":\n'
                f'        marker = {CHECKPOINT_MARKER!r}\n'
                '        print(marker.replace("ciphertexts=19231", "ciphertexts=19230") if mode == "wrong-counts" else marker)\n'
                '    if mode != "missing-result":\n'
                f'        result = {CHECKPOINT_RESULT!r}\n'
                '        print(result.replace("1 passed", "0 passed") if mode == "zero-tests" else result)\n'
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
                IRIS_TEST_PLATFORM=platform, IRIS_TEST_REAL_PYTHON=sys.executable,
                IRIS_TEST_CHECKPOINT_OUTPUT=checkpoint_output,
            )
            env.pop("CARGO_TARGET_DIR", None)
            env.pop("IRIS_DEVICE_APPROVAL_RELAY_URL", None)
            env.pop("IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT", None)
            if cpu_receipt is not None:
                output = directory / "fixture"
                output.mkdir()
                env["IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT"] = str(output / "native-private-controls.json")
                if cpu_receipt != "missing":
                    child_log = b"Synthetic companion child log\n"
                    sample = {"seconds": 35.0, "cpu_percent_one_core": 0.2,
                              "application_wire_bytes": 0, "connections": 0}
                    cycle = {"seconds": 125.0, "cpu_percent_one_core": 0.4,
                             "application_wire_bytes": 1000000, "connections": 480,
                             "every_author_polled": True, "tls_full_handshakes": 0,
                             "tls_resumed_handshakes": 480}
                    measurements = {"database_bytes": 100 * 1024 * 1024,
                                    "message_rows": 30000, "contacts": 96,
                                    "peer_devices": 288, "own_devices": 3, "groups": 8,
                                    "discovery_subjects": 384, "readiness_round_min": 3,
                                    "mesh_discovery_subjects": 96, "total_discovery_subjects": 480,
                                    "completed_peer_polls": 1440, "fresh_anonymous_authentications": 1441,
                                    "revoked_stale_records": 96,
                                    "targeted_fresh_proof_delivered": True,
                                    "privacy_wire_passed": True,
                                    "quiet_samples": [dict(sample), dict(sample)],
                                    "full_cycle_tls_samples": [dict(cycle), dict(cycle)]}
                    receipt = {"schema": "iris-populated-discovery-idle/v1", "passed": True,
                               "timed_out": False, "exit_code": 0,
                               "log_bytes": len(child_log), "log_sha256": hashlib.sha256(child_log).hexdigest(),
                               "measurements": [measurements]}
                    if cpu_receipt == "failed":
                        receipt["passed"] = False
                    elif cpu_receipt == "wrong-log":
                        receipt["log_sha256"] = "0" * 64
                    elif cpu_receipt == "excess-cpu":
                        measurements["quiet_samples"][0]["cpu_percent_one_core"] = 5.1
                    elif cpu_receipt == "tiny-fixture":
                        measurements["database_bytes"] = 2 * 1024 * 1024
                    elif cpu_receipt == "missing-cycles":
                        del measurements["full_cycle_tls_samples"]
                    elif cpu_receipt == "partial-cycle":
                        measurements["full_cycle_tls_samples"][0]["seconds"] = 70.0
                    elif cpu_receipt == "excess-cycle-cpu":
                        measurements["full_cycle_tls_samples"][0]["cpu_percent_one_core"] = 5.1
                    elif cpu_receipt == "unresumed-cycle":
                        measurements["full_cycle_tls_samples"][0]["tls_full_handshakes"] = 1
                    elif cpu_receipt == "no-cycle-polls":
                        measurements["full_cycle_tls_samples"][0]["tls_resumed_handshakes"] = 0
                    elif cpu_receipt == "missing-mesh-subjects":
                        del measurements["mesh_discovery_subjects"]
                    elif cpu_receipt == "missing-total-subjects":
                        del measurements["total_discovery_subjects"]
                    elif cpu_receipt == "too-few-mesh-subjects":
                        measurements["mesh_discovery_subjects"] = 95
                    elif cpu_receipt == "wrong-total-subjects":
                        measurements["total_discovery_subjects"] = 479
                    elif cpu_receipt == "mesh-not-warmed-up":
                        measurements["completed_peer_polls"] = 1152
                    elif cpu_receipt == "missing-fresh-mesh-auth":
                        measurements["fresh_anonymous_authentications"] = 1440
                    elif cpu_receipt == "ordinary-only-tls-cycle":
                        measurements["full_cycle_tls_samples"][1]["tls_resumed_handshakes"] = 384
                    elif cpu_receipt == "ordinary-only-connection-cycle":
                        measurements["full_cycle_tls_samples"][1]["connections"] = 384
                    (output / "populated-discovery-idle.log").write_bytes(child_log)
                    (output / "populated-discovery-idle.json").write_text(json.dumps(receipt))
            if approval_relay is not None:
                env["IRIS_DEVICE_APPROVAL_RELAY_URL"] = approval_relay
            if target is not None:
                env["CARGO_TARGET_DIR"] = target
            env["IRIS_TEST_FAIL_COMMAND"] = json.dumps(fail_on or [])
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
                self.assertEqual(result.stdout.count(CHECKPOINT_MARKER), 1)
                self.assertEqual(result.stdout.count(CHECKPOINT_RESULT), 1)

    def test_checkpoint_requires_execution_result_and_exact_work_counts(self):
        for nextest in (True, False):
            commands = self.commands(nextest)
            checkpoint_index = commands.index(checkpoint_command())
            for mode in ("zero-tests", "missing-marker", "wrong-counts", "missing-result"):
                with self.subTest(nextest=nextest, checkpoint_output=mode):
                    result, calls = self.run_runner(nextest, checkpoint_output=mode)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertIn("Required protocol backlog checkpoint regression did not execute",
                                  result.stderr)
                    self.assertEqual(calls, commands[:checkpoint_index + 1])

    def test_preserves_explicit_target_directory(self):
        for nextest in (True, False):
            with self.subTest(nextest=nextest):
                result, calls = self.run_runner(nextest, target="custom target")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, self.commands(nextest))

    def test_linux_keeps_causal_memory_proof_visible_with_each_runner(self):
        for nextest in (True, False):
            with self.subTest(nextest=nextest):
                result, calls = self.run_runner(nextest, platform="Linux")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, self.commands(nextest, "Linux"))

    def test_failed_linux_memory_gate_stops_before_other_tests(self):
        for nextest in (True, False):
            command = self.commands(nextest, "Linux")[0]
            with self.subTest(nextest=nextest):
                result, calls = self.run_runner(nextest, platform="Linux", fail_on=command)
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertEqual(calls, [command])

    def test_populated_discovery_runs_nonignored_in_the_core_suite(self):
        source = (ROOT / "core/src/core/tests/populated_discovery_idle.rs").read_text()
        self.assertRegex(source, r"#\[test\]\s+fn populated_account_discovery_stays_quiet_and_targeted_demand_progresses\(\)")
        self.assertNotRegex(source, r"#\[ignore(?:\s|=|\])")
        self.assertIn('include!("tests/populated_discovery_idle.rs");',
                      (ROOT / "core/src/core/tests.rs").read_text())
        # Both ordinary runners select the full core suite once; there is no
        # separate expensive CPU invocation to silently duplicate the fixture.
        for nextest in (True, False):
            self.assertFalse(any("populated_account_discovery_stays_quiet" in arg
                                 for command in self.commands(nextest, "Linux") for arg in command))

    def test_linux_requires_valid_populated_companion_evidence(self):
        for nextest in (True, False):
            for mode in ("passed", "missing", "failed", "wrong-log", "excess-cpu", "tiny-fixture",
                         "missing-cycles", "partial-cycle", "excess-cycle-cpu", "unresumed-cycle", "no-cycle-polls",
                         "missing-mesh-subjects", "missing-total-subjects", "too-few-mesh-subjects", "wrong-total-subjects",
                         "mesh-not-warmed-up", "missing-fresh-mesh-auth", "ordinary-only-tls-cycle",
                         "ordinary-only-connection-cycle"):
                with self.subTest(nextest=nextest, receipt=mode):
                    result, calls = self.run_runner(nextest, platform="Linux", cpu_receipt=mode)
                    if mode == "passed":
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(calls, self.commands(nextest, "Linux"))
                    else:
                        self.assertNotEqual(result.returncode, 0)
                        stages = 3 if nextest else 2
                        self.assertEqual(calls, self.commands(nextest, "Linux")[:stages])

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
                    # The checkpoint wrapper prints Cargo's captured diagnostics
                    # and returns 1; other stages preserve the direct exit status.
                    expected_status = 1 if command == checkpoint_command() else 23
                    self.assertEqual(result.returncode, expected_status, result.stderr)
                    self.assertEqual(calls, commands[:index + 1])


if __name__ == "__main__":
    unittest.main()
