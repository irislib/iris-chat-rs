#!/usr/bin/env python3
"""The hardware gate must reject missing evidence as well as expensive idling."""
import copy
from contextlib import contextmanager, redirect_stderr, redirect_stdout
import io
import json
import os
import socket
import subprocess
import plistlib
import sys
import unittest
from unittest.mock import Mock, patch
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import physical_ble_idle as physical

from physical_ble_idle import (
    Gate, evaluate_metrics, internet_addresses, require_android_success, require_isolated_instrumentation,
    reserve_phone, evaluate_saved_run,
    evaluate_background_reply,
)


def fixture(cpu=(1.8, 2.1), writes=(12, 200)):
    return [{"testIdentifier": "FipsBlePhysicalUITests/testSendAndReceiveReceiptOverFipsBle()",
             "testRuns": [{"metrics": [
                 {"displayName": "Clock Monotonic Time", "unitOfMeasurement": "s",
                  "identifier": "com.apple.dt.XCTMetric_Clock.time.monotonic",
                  "measurements": [60.0, 60.0]},
                 {"displayName": "CPU Time (irischat)", "unitOfMeasurement": "s", "measurements": list(cpu),
                  "identifier": "com.apple.dt.XCTMetric_CPU-fi.siriusbusiness.irischat.time"},
                 {"displayName": "Disk Logical Writes", "unitOfMeasurement": "kB",
                  "identifier": "com.apple.dt.XCTMetric_Disk-fi.siriusbusiness.irischat.logical_writes",
                  "measurements": list(writes)},
             ]}]}]


def save_capture(directory):
    (directory / "metrics.log").write_text(json.dumps(fixture()))
    (directory / "test-summary.log").write_text(json.dumps({
        "result": "Passed", "totalTestCount": 1, "passedTests": 1,
        "failedTests": 0, "skippedTests": 0, "testFailures": [],
        "startTime": 1000, "finishTime": 1240,
    }))
    (directory / "receiver.log").write_text(
        "INSTRUMENTATION_CODE: -1\nINSTRUMENTATION_STATUS: seen=true\n"
        "INSTRUMENTATION_STATUS: accepted=true\nINSTRUMENTATION_STATUS: message=probe\n")
    (directory / "network-checks.jsonl").write_text("\n".join(
        json.dumps({"time": t, "ok": True, "addresses": "", "ipv4Routes": "", "ipv6Routes": ""})
        for t in range(999, 1245, 5)))
    (directory / "physical-idle.xctestrun").write_bytes(plistlib.dumps({"IrisChatUITests": {
        "EnvironmentVariables": {"IRIS_FIPS_PHYSICAL_RUN_ID": "run",
                                 "IRIS_FIPS_PHYSICAL_MESSAGE": "probe",
                                 "IRIS_FIPS_IDLE_METRICS": "1", "IRIS_FIPS_IDLE_SECONDS": "60"},
    }}))
    (directory / "result.json").write_text(json.dumps({"ok": False, "testRunId": "run",
        "error": "Old display-name parser rejected valid measurements",
        "restoration": {"wifi_on": True, "mobile_data": True,
                        "bluetooth_unchanged": True, "ios_normal_launch": True,
                        "android_preferences": True, "android_permissions": True, "android_account_stopped": True,
                        "android_account_preserved": True}}))


class PhysicalIdleGateTests(unittest.TestCase):
    @contextmanager
    def background_stub(self, supported=True, stale=False):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run", sample_seconds=60,
                background=True, android_serial="offline-fixture", max_cpu_percent=5, max_write_mib_per_minute=5))
            gate.iphone = "offline-fixture"; gate.saved_account.before = {"owner": "fixture-owner"}
            clock, calls, commands = [0.0], [], []
            android = Mock(returncode=0); android.poll.return_value = None
            ios = Mock(returncode=0)
            def ios_poll():
                if clock[0] < 80:
                    return None
                log = gate.out / "ios-test.log"
                if "LEAVING" not in log.read_text():
                    with log.open("a") as output: output.write("IRIS_FIPS_BACKGROUND_LEAVING=175000\n")
                return 0
            ios.poll.side_effect = ios_poll
            def popen(argv, **kwargs):
                calls.append(argv)
                if len(calls) == 1:
                    token = argv[argv.index("background_release_token") + 1]
                    if supported:
                        kwargs["stdout"].write("INSTRUMENTATION_STATUS: background_release_support=private-file-v1\n"
                                              f"INSTRUMENTATION_STATUS: background_release_token={token}\n")
                    kwargs["stdout"].write("INSTRUMENTATION_STATUS: fips_ble_ready=true\n")
                    process = android
                else:
                    kwargs["stdout"].write("IRIS_FIPS_BACKGROUND_ENTERED=100000\n")
                    process = ios
                kwargs["stdout"].flush()
                return process
            def command(argv, label, timeout=30):
                commands.append((clock[0], argv, label, timeout))
                if stale and label == "background-release-absent":
                    raise RuntimeError("Owned release file already exists")
                return ""
            with patch.object(gate, "preflight"), patch.object(gate, "adb"), \
                    patch.object(gate, "offline", return_value=True), patch.object(gate, "command", side_effect=command), \
                    patch.object(gate, "action", return_value={"npub": "npub1fixture"}), \
                    patch.object(gate, "configure", return_value=Path(temp) / "fixture.xctestrun"), \
                    patch.object(gate, "action_command", side_effect=lambda _, extra: ["offline-receiver", *extra]), \
                    patch("physical_ble_idle.subprocess.Popen", side_effect=popen), \
                    patch("physical_ble_idle.time.monotonic", side_effect=lambda: clock[0]), \
                    patch("physical_ble_idle.time.sleep", side_effect=lambda seconds: clock.__setitem__(0, clock[0] + seconds)), \
                    patch("physical_ble_idle.evaluate_capture", return_value={"ok": True}), redirect_stdout(io.StringIO()):
                yield gate, calls, commands, android

    def test_host_loop_releases_exact_token_after_observed_background_only(self):
        with self.background_stub() as (gate, calls, commands, android):
            gate.execute()
            release = [row for row in commands if row[2] == "background-release"]
            self.assertEqual(len(release), 1)
            self.assertEqual(release[0][0], 40)
            token = calls[0][calls[0].index("background_release_token") + 1]
            self.assertEqual(release[0][1], ["adb", "-s", "offline-fixture", "shell", "run-as", physical.PACKAGE,
                                           "touch", f"files/ble-background-release/{token}.ready"])
            self.assertTrue(gate.result["backgroundBarrier"]["released"])
            android.wait.assert_called_once_with(timeout=165)

    def test_old_android_fixture_is_rejected_before_ios_launch(self):
        with self.background_stub(supported=False) as (gate, calls, commands, _):
            with self.assertRaisesRegex(RuntimeError, "lacks the exact background barrier"):
                gate.execute()
            self.assertEqual(len(calls), 1)
            self.assertFalse(commands)

    def test_stale_release_file_is_not_overwritten_or_claimed_owned(self):
        with self.background_stub(stale=True) as (gate, _, commands, _):
            with self.assertRaisesRegex(RuntimeError, "already exists"):
                gate.execute()
            self.assertFalse(any(row[2] == "background-release" for row in commands))
            self.assertFalse(gate.background_release.release_attempted)

    def barrier(self):
        barrier = physical.BackgroundRelease("12345678-1234-4abc-8def-1234567890ab")
        support = ("INSTRUMENTATION_STATUS: background_release_support=private-file-v1\n"
                   f"INSTRUMENTATION_STATUS: background_release_token={barrier.token}\n")
        barrier.acknowledge(support + "INSTRUMENTATION_STATUS: fips_ble_ready=true\n")
        barrier.start(0)
        return barrier, support

    def test_barrier_requires_exact_supported_token_before_readiness(self):
        barrier, support = self.barrier()
        self.assertTrue(barrier.acknowledged)
        ready = "INSTRUMENTATION_STATUS: fips_ble_ready=true\n"
        for output in [ready, ready + support, support + support + ready,
                       support.replace(barrier.token, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa") + ready,
                       support.replace("private-file-v1", "unsupported") + ready]:
            with self.subTest(output=output), self.assertRaises(RuntimeError):
                physical.BackgroundRelease(barrier.token).acknowledge(output)

    def test_streamed_ack_fragments_wait_for_complete_newline_records(self):
        barrier = physical.BackgroundRelease("12345678-1234-4abc-8def-1234567890ab")
        prefix = "INSTRUMENTATION_STATUS: background_release_support=private-file-v1\n"
        token = f"INSTRUMENTATION_STATUS: background_release_token={barrier.token}"
        self.assertFalse(barrier.acknowledge(prefix + token[:-5]))
        self.assertFalse(barrier.acknowledge(prefix + token + "\nINSTRUMENTATION_STATUS: fips_ble_ready=tr"))
        self.assertTrue(barrier.acknowledge(prefix + token + "\nINSTRUMENTATION_STATUS: fips_ble_ready=true\n"))

    def test_streamed_background_fragment_starts_clock_only_after_newline(self):
        barrier, _ = self.barrier()
        partial = "IRIS_FIPS_BACKGROUND_ENTERED=1792537"
        self.assertFalse(barrier.observe(partial, 5))
        self.assertIsNone(barrier.observed_at)
        completed = partial + "565930\n"
        self.assertFalse(barrier.observe(completed, 10))
        self.assertEqual(barrier.entered_ms, 1792537565930)
        self.assertFalse(barrier.observe(completed, 49.999))
        self.assertTrue(barrier.observe(completed, 50))

    def test_malformed_completed_and_incomplete_final_protocol_records_fail_closed(self):
        barrier, support = self.barrier()
        for output in [support.replace("=private-file-v1", "broken") + "INSTRUMENTATION_STATUS: fips_ble_ready=true\n",
                       support + "INSTRUMENTATION_STATUS: fips_ble_ready=tr\n"]:
            with self.assertRaises(RuntimeError):
                physical.BackgroundRelease(barrier.token).acknowledge(output)
        for output in ["IRIS_FIPS_BACKGROUND_ENTERED=123oops\n", "IRIS_FIPS_BACKGROUND_ENTEREDbroken\n",
                       "IRIS_FIPS_BACKGROUND_ENTERED=123"]:
            with self.assertRaises(RuntimeError):
                barrier.observe(output, 10, completed=True)
        with self.assertRaises(RuntimeError):
            barrier.acknowledge(support + "INSTRUMENTATION_STATUS: fips_ble_ready=true", final=True)

    def test_barrier_rejects_unsafe_noncanonical_or_nonrandom_tokens(self):
        for token in ["../some-file", "", "12345678-1234-1abc-8def-1234567890ab",
                      "12345678-1234-4ABC-8DEF-1234567890AB", "1234567812344abc8def1234567890ab"]:
            with self.subTest(token=token), self.assertRaises(ValueError):
                physical.BackgroundRelease(token)

    def test_barrier_waits_forty_monotonic_seconds_from_observed_entry(self):
        barrier, _ = self.barrier()
        log = "IRIS_FIPS_BACKGROUND_ENTERED=100000\n"
        self.assertFalse(barrier.observe(log, 10))
        self.assertFalse(barrier.observe(log, 49.999))
        self.assertTrue(barrier.observe(log, 50))
        barrier.mark_released(50.2)
        self.assertFalse(barrier.observe(log + "IRIS_FIPS_BACKGROUND_LEAVING=175000\n", 100, completed=True))
        self.assertEqual(barrier.evidence()["releaseDelaySeconds"], 40)
        self.assertEqual(barrier.relative_file, f"files/ble-background-release/{barrier.token}.ready")

    def test_barrier_fails_missing_repeated_malformed_and_early_leaving_markers(self):
        for log in ["", "IRIS_FIPS_BACKGROUND_ENTERED=bad\n",
                    "IRIS_FIPS_BACKGROUND_ENTERED=100000\n" * 2,
                    "IRIS_FIPS_BACKGROUND_LEAVING=175000\n",
                    "IRIS_FIPS_BACKGROUND_ENTERED=100000\nIRIS_FIPS_BACKGROUND_LEAVING=175000\n"]:
            barrier, _ = self.barrier()
            with self.subTest(log=log), self.assertRaises(RuntimeError):
                barrier.observe(log, 240)

    def test_barrier_rejects_late_host_or_early_process_completion(self):
        for now, completed in [(46, False), (10, True)]:
            barrier, _ = self.barrier()
            log = "IRIS_FIPS_BACKGROUND_ENTERED=100000\n"
            barrier.observe(log, 0)
            with self.subTest(now=now), self.assertRaises(RuntimeError):
                barrier.observe(log, now, completed=completed)

    def test_cleanup_removes_only_an_attempted_owned_release_file(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run", android_serial="offline-fixture"))
            gate.background_release, _ = self.barrier()
            with patch.object(gate.saved_account, "restore"), patch.object(gate, "command") as command:
                gate.restore()
                command.assert_not_called()
                gate.background_release.release_attempted = True
                gate.restore()
            command.assert_called_once_with(["adb", "-s", "offline-fixture", "shell", "run-as", physical.PACKAGE,
                "rm", "-f", gate.background_release.relative_file], "background-release-cleanup", 5)

    def test_default_and_fractional_receiver_lifetime_round_up_without_shortening(self):
        self.assertEqual(physical.receiver_lifetime(60, True),
                         {"idleHoldMs": 305000, "backgroundSeconds": 75, "postIosWaitSeconds": 165})
        self.assertEqual(physical.receiver_lifetime(60, False),
                         {"idleHoldMs": 230000, "backgroundSeconds": 0, "postIosWaitSeconds": 90})
        self.assertEqual(physical.receiver_lifetime(60.0001, True)["idleHoldMs"], 305001)
        self.assertEqual(physical.receiver_lifetime(40.1234, False)["idleHoldMs"], 170371)

    def test_receiver_lifetime_matches_apk_cap_and_rejects_unsupported_requests(self):
        self.assertEqual(physical.receiver_lifetime(120, False)["idleHoldMs"], 410000)
        background_maximum = (420 - 75 - 50) / 3
        self.assertEqual(physical.receiver_lifetime(background_maximum, True)["idleHoldMs"], 420000)
        for seconds, background in [(background_maximum + 0.001, True), (120, True), (9.9, False),
                                    (120.1, False), (float("nan"), True), (float("inf"), False)]:
            with self.subTest(seconds=seconds), self.assertRaises(ValueError):
                physical.receiver_lifetime(seconds, background)

    def test_unsupported_lifetime_rejected_before_device_preflight(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run", sample_seconds=120, background=True))
            with patch.object(gate, "preflight") as preflight:
                with self.assertRaisesRegex(ValueError, "420-second"):
                    gate.execute()
            preflight.assert_not_called()

    def test_unsupported_cli_request_does_not_create_a_gate_or_touch_phones(self):
        argv = ["physical_ble_idle.py", "--iphone", "offline-fixture", "--android-serial", "offline-fixture",
                "--xctestrun", "/unused", "--artifact-dir", "/unused", "--sample-seconds", "120", "--background"]
        with patch.object(sys, "argv", argv), patch("physical_ble_idle.Gate") as gate, \
                redirect_stderr(io.StringIO()) as error:
            with self.assertRaises(SystemExit) as raised:
                physical.main()
        self.assertEqual(raised.exception.code, 2)
        self.assertIn("420-second", error.getvalue())
        gate.assert_not_called()

    def test_default_cli_sample_and_resource_budgets_remain_unchanged(self):
        argv = ["physical_ble_idle.py", "--iphone", "offline-fixture", "--android-serial", "offline-fixture",
                "--xctestrun", "/unused", "--artifact-dir", "/unused"]
        with patch.object(sys, "argv", argv), patch("physical_ble_idle.Gate") as gate, redirect_stdout(io.StringIO()):
            gate.return_value.result = {"ok": True}
            self.assertEqual(physical.main(), 0)
        args = gate.call_args.args[0]
        self.assertEqual((args.sample_seconds, args.max_cpu_percent, args.max_write_mib_per_minute), (60, 5, 5))

    def test_resource_budget_boundaries_are_still_five_percent_and_five_mib_per_minute(self):
        at_limit = fixture(cpu=(3, 3), writes=(5 * 1024**2, 5 * 1024**2))
        at_limit[0]["testRuns"][0]["metrics"][2]["unitOfMeasurement"] = "B"
        self.assertTrue(evaluate_metrics(at_limit, 60, 5, 5)["ok"])
        above_limit = copy.deepcopy(at_limit)
        above_limit[0]["testRuns"][0]["metrics"][1]["measurements"][0] += 0.001
        above_limit[0]["testRuns"][0]["metrics"][2]["measurements"][0] += 1
        self.assertEqual(len(evaluate_metrics(above_limit, 60, 5, 5)["failures"]), 2)

    def test_background_receiver_lifetime_covers_background_and_measurements(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run", sample_seconds=60,
                background=True, max_cpu_percent=5, max_write_mib_per_minute=5))
            gate.iphone = "offline-fixture"
            gate.saved_account.before = {"owner": "fixture-owner"}
            android = Mock(returncode=0); android.poll.return_value = None
            ios = Mock(returncode=0); ios.poll.return_value = 0
            calls = []
            def popen(argv, **kwargs):
                calls.append(argv)
                if len(calls) == 1:
                    kwargs["stdout"].write("INSTRUMENTATION_STATUS: fips_ble_ready=true\n")
                    kwargs["stdout"].flush()
                    return android
                return ios
            with patch.object(gate, "preflight"), patch.object(gate, "adb"), \
                    patch.object(gate, "offline", return_value=True), patch.object(gate, "command"), \
                    patch.object(gate, "action", return_value={"npub": "npub1fixture"}), \
                    patch.object(gate, "configure", return_value=Path(temp) / "fixture.xctestrun"), \
                    patch.object(gate, "action_command", side_effect=lambda _, extra: ["offline-receiver", *extra]), \
                    patch("physical_ble_idle.BackgroundRelease", return_value=Mock(token="offline-token")), \
                    patch("physical_ble_idle.subprocess.Popen", side_effect=popen), \
                    patch("physical_ble_idle.evaluate_capture", return_value={"ok": True}):
                gate.execute()
            self.assertEqual(calls[0][calls[0].index("idle_hold_ms") + 1], "305000")
            android.wait.assert_called_once_with(timeout=165)

    def test_background_proof_rejects_receipt_after_foreground_and_missing_evidence(self):
        with TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "ios-test.log").write_text(
                "IRIS_FIPS_BACKGROUND_ENTERED=100000\nIRIS_FIPS_BACKGROUND_LEAVING=175000\n")
            def receiver(sent=145000, received=150000):
                (root / "receiver.log").write_text(
                    "INSTRUMENTATION_CODE: -1\nINSTRUMENTATION_STATUS: background_reply_received=true\n"
                    "INSTRUMENTATION_STATUS: background_reply=reply\n"
                    f"INSTRUMENTATION_STATUS: background_sent_ms={sent}\n"
                    f"INSTRUMENTATION_STATUS: background_received_ms={received}\n")
            receiver()
            self.assertTrue(evaluate_background_reply(root, "reply")["backgroundReplyReceivedWhileHidden"])
            for sent, received in [(110000, 120000), (120185, 122000), (145000, 176000), (150000, 145000)]:
                receiver(sent, received)
                with self.assertRaises(ValueError):
                    evaluate_background_reply(root, "reply")
            receiver()
            (root / "ios-test.log").write_text("")
            with self.assertRaises(ValueError):
                evaluate_background_reply(root, "reply")

    def test_configure_requires_current_device_from_saved_roster(self):
        with TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source.xctestrun"
            app = root / "TestApp.app"
            bundle = root / "IrisChatUITests.xctest"
            for path in (app, bundle):
                path.mkdir()
                (path / "Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable": "test", "CFBundleVersion": "1"}))
                (path / "test").write_bytes(b"test product")
            source.write_bytes(plistlib.dumps({"IrisChatUITests": {
                "TestBundlePath": str(bundle), "UITargetAppPath": str(app),
                "EnvironmentVariables": {"IRIS_UI_TEST_DATA_DIR": "/ordinary-data"},
            }}))
            gate = Gate(SimpleNamespace(artifact_dir=root / "run", xctestrun=source, sample_seconds=60))
            device = "a" * 64
            gate.saved_account.before = {"devices": [device]}
            for invalid in ["", "b" * 64, device + "\n"]:
                with self.assertRaises(ValueError):
                    gate.configure("npub1fixture", invalid)
            configured = plistlib.loads(gate.configure("npub1fixture", device).read_bytes())
            environment = configured["IrisChatUITests"]["EnvironmentVariables"]
            self.assertEqual(environment["IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX"], device)
            self.assertEqual(environment["IRIS_FIPS_PHYSICAL_RUN_ID"], gate.run_id)
            self.assertNotIn("IRIS_UI_TEST_DATA_DIR", environment)

    def test_quiet_app_passes_and_reports_each_minute(self):
        result = evaluate_metrics(fixture(), 60, 5, 5)
        self.assertTrue(result["ok"])
        self.assertEqual(result["cpuPercentOneCore"], [3, 3.5])

    def test_previous_cpu_and_write_churn_fails(self):
        result = evaluate_metrics(fixture((7.66, 6.10), (90000, 85000)), 60, 5, 5)
        self.assertFalse(result["ok"])
        self.assertEqual(len(result["failures"]), 4)

    def test_metrics_match_real_xcode_app_suffixed_output(self):
        metrics = fixture((2.350145, 1.797713), (110.592, 106.496))
        metrics[0]["testRuns"][0]["metrics"][0]["measurements"] = [59.041051322, 59.044853785]
        result = evaluate_metrics(metrics, 60, 5, 5)
        self.assertTrue(result["ok"])
        self.assertAlmostEqual(result["cpuPercentOneCore"][0], 3.980527, places=5)
        self.assertAlmostEqual(result["diskWriteMiBPerMinute"][1], 0.103205, places=5)

    def test_one_bad_minute_is_not_hidden_by_average(self):
        self.assertFalse(evaluate_metrics(fixture((0, 6)), 60, 5, 5)["ok"])

    def test_missing_truncated_duplicate_and_nonfinite_data_fail_closed(self):
        cases = [[], fixture(), fixture(), fixture(), fixture(), fixture(), fixture()]
        cases[1][0]["testRuns"][0]["metrics"].pop()
        cases[2][0]["testRuns"][0]["metrics"][0]["measurements"] = [0.1, 0.1]
        cases[3][0]["testRuns"][0]["metrics"][1]["measurements"] = [float("nan"), 1]
        cases[4][0]["testRuns"][0]["metrics"][2]["measurements"] = [-1, 1]
        cases[5][0]["testRuns"].append(copy.deepcopy(cases[5][0]["testRuns"][0]))
        cases[6][0]["testRuns"][0]["metrics"][1]["measurements"] = [1]
        for value in cases:
            with self.subTest(value=value), self.assertRaises(ValueError):
                evaluate_metrics(value, 60, 5, 5)

    def test_unknown_units_are_not_silently_treated_as_seconds(self):
        value = fixture()
        value[0]["testRuns"][0]["metrics"][1]["unitOfMeasurement"] = "ticks"
        with self.assertRaises(ValueError):
            evaluate_metrics(value, 60, 5, 5)

    def test_only_verified_virtual_dummy_and_loopback_are_allowed(self):
        loopback = "1: lo inet 127.0.0.1/8 scope host lo\n1: lo inet6 ::1/128 scope host"
        dummy = "2: dummy0 inet6 fe80::1/64 scope link"
        self.assertEqual(internet_addresses(loopback + "\n" + dummy, True), [])
        self.assertTrue(internet_addresses(dummy, False))
        for address in ["3: wlan0 inet 192.0.2.1/24 scope global", "4: rmnet0 inet6 2001:db8::1/64 scope global",
                        "3: wlan0 inet6 fe80::1/64 scope link", "unexpected inet data"]:
            self.assertTrue(internet_addresses(address, True))

    def test_android_exit_zero_with_instrumentation_failure_is_not_success(self):
        with self.assertRaises(RuntimeError):
            require_android_success("INSTRUMENTATION_CODE: -1\nFAILURES!!!", {"seen": "true"})
        with self.assertRaises(RuntimeError):
            require_android_success("INSTRUMENTATION_CODE: -1", {"seen": "true"})
        require_android_success("INSTRUMENTATION_CODE: -1\nINSTRUMENTATION_STATUS: seen=true", {"seen": "true"})

    def test_failed_measurement_still_restores_radio_settings(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run"))
            gate.snapshot = {"wifi_on": "1", "mobile_data": "0", "bluetooth_on": "1"}
            commands = []
            def adb(*args):
                commands.append(args)
                return gate.snapshot.get(args[-1], "")
            with patch.object(gate, "adb", side_effect=adb):
                gate.restore()
            self.assertFalse(gate.result["ok"])
            self.assertTrue(all(gate.result["restoration"].values()))
            self.assertIn(("svc", "wifi", "enable"), commands)
            self.assertIn(("svc", "data", "disable"), commands)
            self.assertNotIn(("svc", "bluetooth", "disable"), commands)

    def test_runner_cannot_target_the_personal_app(self):
        prefix = "instrumentation:to.iris.chat.blegate.test/androidx.test.runner.AndroidJUnitRunner "
        require_isolated_instrumentation(prefix + "(target=to.iris.chat.blegate)")
        with self.assertRaises(ValueError):
            require_isolated_instrumentation(prefix + "(target=to.iris.chat)")

    def test_release_wrapper_can_lend_its_reservation_but_other_runs_cannot(self):
        owner = {"pid": 123, "host": socket.gethostname()}
        with patch("physical_ble_idle.acquire", return_value=(None, owner)), \
                patch("physical_ble_idle.ancestor_pids", return_value={123}):
            self.assertIsNone(reserve_phone("android:test"))
        with patch("physical_ble_idle.acquire", return_value=(None, owner)), \
                patch("physical_ble_idle.ancestor_pids", return_value={456}):
            with self.assertRaises(RuntimeError):
                reserve_phone("android:test")

    def test_release_gate_rejects_missing_phone_before_running_other_gates(self):
        root = Path(__file__).resolve().parent.parent
        result = subprocess.run([str(root / "scripts/test-release-gate"), "--physical-ble-idle", "--skip-fast"],
                                env=dict(os.environ, IRIS_CHAT_LAB_IOS_DEVICE=""),
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Select the physical iPhone explicitly" if sys.platform == "darwin"
                      else "Physical Bluetooth idle gate requires macOS", result.stderr)

    def test_saved_capture_can_be_checked_without_touching_phones_or_original_result(self):
        with TemporaryDirectory() as temp:
            directory = Path(temp)
            save_capture(directory)
            original = (directory / "result.json").read_bytes()
            with patch("physical_ble_idle.subprocess.run", side_effect=AssertionError("No device commands")):
                result = evaluate_saved_run(directory, 5, 5)
            self.assertTrue(result["ok"])
            self.assertEqual(result["evaluationMode"], "saved-capture")
            self.assertEqual((directory / "result.json").read_bytes(), original)

    def test_incomplete_or_failed_hardware_evidence_cannot_pass(self):
        for kind in ("skipped", "wrong_probe", "network_gap", "ip_route", "missing_restore", "failed_restore", "wrong_run", "missing_account_restore"):
            with self.subTest(kind=kind), TemporaryDirectory() as temp:
                directory = Path(temp)
                save_capture(directory)
                if kind == "skipped":
                    path = directory / "test-summary.log"
                    data = json.loads(path.read_text()); data["skippedTests"] = 1
                    path.write_text(json.dumps(data))
                elif kind == "wrong_probe":
                    path = directory / "receiver.log"
                    path.write_text(path.read_text().replace("message=probe", "message=other"))
                elif kind in ("network_gap", "ip_route"):
                    path = directory / "network-checks.jsonl"
                    lines = path.read_text().splitlines()
                    if kind == "network_gap":
                        lines = lines[:2] + lines[-2:]
                    else:
                        data = json.loads(lines[2]); data["ipv6Routes"] = "default via fe80::1"
                        lines[2] = json.dumps(data)
                    path.write_text("\n".join(lines))
                else:
                    path = directory / "result.json"
                    data = json.loads(path.read_text())
                    if kind == "missing_restore": data["restoration"].pop("mobile_data")
                    if kind == "missing_account_restore": data["restoration"].pop("android_account_preserved")
                    if kind == "failed_restore": data["restoration"]["mobile_data"] = False
                    if kind == "wrong_run": data["testRunId"] = "another-run"
                    path.write_text(json.dumps(data))
                with self.assertRaises((ValueError, RuntimeError)):
                    evaluate_saved_run(directory, 5, 5)

    def test_termination_failure_still_restores_radios_and_records_failure(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run"))
            gate.snapshot = {"wifi_on": "1", "mobile_data": "0", "bluetooth_on": "1"}
            gate.result["ok"] = True
            with patch("physical_ble_idle.stop", side_effect=RuntimeError("Could not stop process")), \
                    patch.object(gate, "adb", side_effect=lambda *args: gate.snapshot.get(args[-1], "")):
                gate.restore()
            self.assertFalse(gate.result["ok"])
            self.assertTrue(gate.result["restoration"]["wifi_on"])
            self.assertTrue(gate.result["restoration"]["mobile_data"])

    def test_failed_radio_restore_fails_gate_and_still_attempts_other_cleanup(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run"))
            gate.snapshot = {"wifi_on": "1", "mobile_data": "0", "bluetooth_on": "1"}
            gate.result["ok"] = True
            def adb(*args):
                if args == ("svc", "wifi", "enable"):
                    raise RuntimeError("Device disconnected")
                return gate.snapshot.get(args[-1], "")
            with patch.object(gate, "adb", side_effect=adb):
                gate.restore()
            result = json.loads((gate.out / "result.json").read_text())
            self.assertFalse(result["ok"])
            self.assertFalse(result["restoration"]["wifi_on"])
            self.assertTrue(result["restoration"]["mobile_data"])
            self.assertTrue(result["restoration"]["android_cleanup"])

    def test_normal_ios_launch_must_survive_not_just_be_accepted(self):
        for dies in (False, True):
            with self.subTest(dies=dies), TemporaryDirectory() as temp:
                gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run"))
                gate.ios = Mock(); gate.ios.poll.return_value = 0
                gate.iphone = "owned-fixture"
                gate.result["ok"] = True
                clock, launches = [0.0], []
                executable = "file:///private/var/containers/Bundle/Application/fixture/IrisChat.app/IrisChat"
                def command(argv, label, timeout=30, *, env=None):
                    if "launch" in argv:
                        launches.append(argv)
                        result = {"process": {"processIdentifier": 123, "executable": executable}}
                    else:
                        self.assertEqual(argv[argv.index("--search") + 1], "IrisChat")
                        alive = not dies or clock[0] < 3
                        result = {"runningProcesses": [{"processIdentifier": 123, "executable": executable}] if alive else []}
                    if "--json-output" not in argv:
                        return ""
                    output = Path(argv[argv.index("--json-output") + 1])
                    result["deviceIdentifier"] = "owned-coredevice-fixture"
                    output.write_text(json.dumps({"info": {"outcome": "success"}, "result": result}))
                    return ""
                with patch.object(gate, "command", side_effect=command), \
                        patch.object(gate.saved_account, "restore") as account_restore, \
                        patch("physical_ble_idle.time.monotonic", side_effect=lambda: clock[0]), \
                        patch("physical_ble_idle.time.sleep", side_effect=lambda seconds: clock.__setitem__(0, clock[0] + seconds)):
                    gate.restore()
                self.assertEqual(len(launches), 1, "A dead app must not be relaunched to hide the failure")
                self.assertEqual(gate.restored["ios_normal_launch"], not dies)
                self.assertEqual(gate.result["ok"], not dies)
                account_restore.assert_called_once()
                if not dies:
                    self.assertGreaterEqual(clock[0], 15)

    def test_ios_survival_rejects_wrong_restarted_and_malformed_process_evidence(self):
        executable = "file:///private/var/containers/Bundle/Application/fixture/IrisChat.app/IrisChat"
        process = {"processIdentifier": 123, "executable": executable}
        cases = {
            "missing-pid": ({"executable": executable}, None),
            "boolean-pid": ({**process, "processIdentifier": True}, None),
            "wrong-app": ({**process, "executable": executable.replace("IrisChat.app", "Other.app")}, None),
            "extension": ({**process, "executable": executable + "/PlugIns/NotificationService.appex/NotificationService"}, None),
            "restart": (process, [{**process, "processIdentifier": 124}]),
            "changed-executable": (process, [{**process, "executable": executable.replace("fixture/", "another/")}]),
            "duplicate-pid": (process, [process, process]),
            "missing-inventory": (process, None),
            "invalid-inventory-record": (process, [{"name": "IrisChat"}]),
            "failed-json-outcome": (process, [process]),
            "malformed-json": (process, [process]),
            "wrong-device": (process, [process]),
        }
        for name, (launched, inventory) in cases.items():
            with self.subTest(name=name), TemporaryDirectory() as temp:
                calls = []
                def command(argv, label, timeout=30, *, env=None):
                    calls.append(argv)
                    result = {"process": launched} if "launch" in argv else {"runningProcesses": inventory}
                    result["deviceIdentifier"] = ("other-fixture" if name == "wrong-device" and "launch" not in argv
                                                  else "owned-coredevice-fixture")
                    outcome = "failure" if name == "failed-json-outcome" else "success"
                    path = Path(argv[argv.index("--json-output") + 1])
                    path.write_text("not json" if name == "malformed-json" else
                                    json.dumps({"info": {"outcome": outcome}, "result": result}))
                with self.assertRaises((ValueError, RuntimeError)):
                    physical.verify_ios_launch(command, "owned-fixture", Path(temp), {})
                self.assertEqual(sum("launch" in args for args in calls), 1)

    def test_ios_inventory_failure_preserves_evidence_and_finishes_other_cleanup(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run"))
            gate.ios = Mock(); gate.ios.poll.return_value = 0
            gate.iphone = "owned-fixture"
            gate.snapshot = {"wifi_on": "1", "mobile_data": "0", "bluetooth_on": "1"}
            gate.locks = [Path(temp) / "owned-lock"]
            gate.result["ok"] = True
            launch_env = []
            def command(argv, label, timeout=30, *, env=None):
                if "launch" not in argv:
                    raise RuntimeError("Device tunnel unavailable")
                launch_env.append(env)
                Path(argv[argv.index("--json-output") + 1]).write_text(json.dumps({
                    "info": {"outcome": "success"}, "result": {"process": {"processIdentifier": 123,
                    "executable": "file:///private/var/containers/Bundle/Application/fixture/IrisChat.app/IrisChat"}}}))
                return ""
            with patch.object(gate, "command", side_effect=command), \
                    patch.object(gate.saved_account, "restore") as account_restore, \
                    patch.object(gate, "adb", side_effect=lambda *args: gate.snapshot.get(args[-1], "")), \
                    patch.dict(os.environ, {"DEVICECTL_CHILD_IRIS_UI_TEST_RESET": "1", "PATH": "/normal/tools"}), \
                    patch("physical_ble_idle.release") as release:
                gate.restore()
            account_restore.assert_called_once()
            release.assert_called_once_with(gate.locks[0])
            self.assertEqual(len(launch_env), 1)
            self.assertNotIn("DEVICECTL_CHILD_IRIS_UI_TEST_RESET", launch_env[0])
            self.assertEqual(launch_env[0]["PATH"], "/normal/tools")
            self.assertFalse(gate.result["ok"])
            self.assertFalse(gate.restored["ios_normal_launch_survived"])
            for key in ("wifi_on", "mobile_data", "bluetooth_unchanged", "android_cleanup"):
                self.assertTrue(gate.restored[key])
            self.assertTrue((gate.out / "ios-normal-account.json").exists())
            self.assertIn("Device tunnel unavailable", gate.result["iosNormalLaunchSurvival"]["error"])

    def test_saved_legacy_launch_is_not_relabelled_as_a_survival_check(self):
        with TemporaryDirectory() as temp:
            directory = Path(temp)
            save_capture(directory)
            result = evaluate_saved_run(directory, 5, 5)
            self.assertEqual(result["iosLaunchVerification"],
                             {"contract": "legacy-launch-command-only", "survivalChecked": False})
            path = directory / "result.json"
            data = json.loads(path.read_text())
            data["restoration"]["ios_normal_launch_survived"] = True
            data["iosNormalLaunchSurvival"] = {"contract": "same-executable-pid-15s-v1", "passed": True,
                                               "observedSeconds": 0}
            path.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                evaluate_saved_run(directory, 5, 5)

    def test_result_write_failure_releases_owned_reservation_after_cleanup(self):
        with TemporaryDirectory() as temp:
            gate = Gate(SimpleNamespace(artifact_dir=Path(temp) / "run"))
            gate.locks = [Path(temp) / "owned-lock"]
            with patch("physical_ble_idle.write_json", side_effect=OSError("Disk full")), \
                    patch("physical_ble_idle.release") as release:
                with self.assertRaises(OSError):
                    gate.restore()
            release.assert_called_once_with(gate.locks[0])

    def test_saved_run_cli_passes_and_preserves_existing_output(self):
        with TemporaryDirectory() as temp:
            directory = Path(temp)
            save_capture(directory)
            output = directory / "evaluation.json"
            script = Path(__file__).with_name("physical_ble_idle.py")
            command = [sys.executable, str(script), "--evaluate", str(directory), "--output", str(output)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            original = output.read_bytes()
            result = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(output.read_bytes(), original)

    def test_saved_run_cli_reports_missing_evidence_as_failure(self):
        with TemporaryDirectory() as temp:
            directory = Path(temp)
            save_capture(directory)
            (directory / "metrics.log").unlink()
            output = directory / "evaluation.json"
            script = Path(__file__).with_name("physical_ble_idle.py")
            result = subprocess.run([sys.executable, str(script), "--evaluate", str(directory),
                                     "--output", str(output)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertFalse(json.loads(output.read_text())["ok"])


if __name__ == "__main__":
    unittest.main()
