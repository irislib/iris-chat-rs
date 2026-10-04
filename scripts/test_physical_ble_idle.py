#!/usr/bin/env python3
"""The hardware gate must reject missing evidence as well as expensive idling."""
import copy
import os
import socket
import subprocess
import sys
import unittest
from unittest.mock import patch
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace

from physical_ble_idle import (
    Gate, evaluate_metrics, internet_addresses, require_android_success, require_isolated_instrumentation,
    reserve_phone,
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


class PhysicalIdleGateTests(unittest.TestCase):
    def test_quiet_app_passes_and_reports_each_minute(self):
        result = evaluate_metrics(fixture(), 60, 5, 5)
        self.assertTrue(result["ok"])
        self.assertEqual(result["cpuPercentOneCore"], [3, 3.5])

    def test_previous_cpu_and_write_churn_fails(self):
        result = evaluate_metrics(fixture((7.66, 6.10), (90000, 85000)), 60, 5, 5)
        self.assertFalse(result["ok"])
        self.assertEqual(len(result["failures"]), 4)

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


if __name__ == "__main__":
    unittest.main()
