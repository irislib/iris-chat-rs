#!/usr/bin/env python3
"""The hardware gate must reject missing evidence as well as expensive idling."""
import copy
import json
import os
import socket
import subprocess
import plistlib
import sys
import unittest
from unittest.mock import patch
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace

from physical_ble_idle import (
    Gate, evaluate_metrics, internet_addresses, require_android_success, require_isolated_instrumentation,
    reserve_phone, evaluate_saved_run,
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
