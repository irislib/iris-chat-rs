#!/usr/bin/env python3

import hashlib
import json
import os
import platform
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import native_lab


ROOT = Path(__file__).resolve().parent
LAB = ROOT / "native_lab.py"


class NativeLabTests(unittest.TestCase):
    def test_physical_ios_selection_uses_structured_device_properties(self) -> None:
        def device(identifier, udid, name, reality, state):
            return {
                "identifier": identifier,
                "properties": {
                    "hardware": {"udid": udid, "reality": reality, "platform": "iOS"},
                    "connection": {"state": state},
                    "state": {"name": name},
                },
            }

        payload = json.dumps({"result": {"devices": [
            device("sim-id", "sim-udid", "Simulator", "simulated", "connected"),
            device("offline-id", "offline-udid", "Offline phone", "physical", "disconnected"),
            device("phone-id", "phone-udid", "Test phone", "physical", "connected"),
        ]}})
        with mock.patch.object(native_lab.platform, "system", return_value="Darwin"), \
             mock.patch.object(native_lab.shutil, "which", return_value="/usr/bin/xcrun"), \
             mock.patch.object(native_lab, "run_probe", return_value=(True, payload)) as probe:
            for selector in ("auto", "phone-udid", "phone-id", "Test phone"):
                result = native_lab.check_health(f"ios-device:{selector}")
                self.assertTrue(result["available"], result)
                self.assertEqual(result["allocation"], "phone-udid")
            self.assertFalse(native_lab.check_health("ios-device:sim-udid")["available"])
            self.assertFalse(native_lab.check_health("ios-device:offline-udid")["available"])
            self.assertIn("--json-output", probe.call_args.args[0])

    def run_lab(self, *args: str) -> subprocess.CompletedProcess:
        environment = os.environ.copy()
        environment["IRIS_NATIVE_LAB_STATE_DIR"] = self.state_dir
        return subprocess.run(
            [sys.executable, str(LAB), *args],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=environment,
        )

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.state_dir = str(Path(self.temp.name) / "state")

    def test_health_distinguishes_available_and_missing_commands(self) -> None:
        available = self.run_lab("health", "--health", f"command:{Path(sys.executable).name}")
        self.assertEqual(available.returncode, 0, available.stderr)
        self.assertEqual(json.loads(available.stdout)["status"], "available")
        missing = self.run_lab("health", "--health", "command:definitely-not-a-real-tool")
        self.assertEqual(missing.returncode, 75)
        self.assertEqual(json.loads(missing.stdout)["status"], "infrastructure_unavailable")

    def test_run_classifies_product_and_infrastructure_failures(self) -> None:
        product = self.run_lab(
            "run", "--resource", "test-device", "--", sys.executable, "-c", "raise SystemExit(7)"
        )
        self.assertEqual(product.returncode, 7)
        self.assertEqual(json.loads(product.stdout)["status"], "product_failure")
        infra = self.run_lab(
            "run", "--resource", "test-device", "--", sys.executable, "-c", "raise SystemExit(75)"
        )
        self.assertEqual(infra.returncode, 75)
        self.assertEqual(json.loads(infra.stdout)["status"], "infrastructure_unavailable")

    def test_run_writes_machine_readable_result(self) -> None:
        result = Path(self.temp.name) / "result.json"
        completed = self.run_lab(
            "run",
            "--resource",
            "test-device",
            "--result",
            str(result),
            "--",
            sys.executable,
            "-c",
            "raise SystemExit(0)",
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(json.loads(result.read_text(encoding="utf-8"))["status"], "passed")

    def test_run_exports_the_selected_health_allocation(self) -> None:
        variable = "IRIS_NATIVE_LAB_TEST_DEVICE"
        previous = os.environ.get(variable)
        os.environ[variable] = "known-device"
        self.addCleanup(
            lambda: os.environ.pop(variable, None)
            if previous is None
            else os.environ.__setitem__(variable, previous)
        )
        completed = self.run_lab(
            "run",
            "--resource",
            "allocation-export",
            "--health",
            f"env:{variable}",
            "--allocation-env",
            "env=ALLOCATED_DEVICE",
            "--",
            sys.executable,
            "-c",
            "import os; raise SystemExit(0 if os.environ.get('ALLOCATED_DEVICE') == 'known-device' else 9)",
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(json.loads(completed.stdout)["status"], "passed")

    def test_run_rejects_a_busy_named_host_allocation(self) -> None:
        system = platform.system().lower()
        label = {"darwin": "macos", "windows": "windows", "linux": "linux"}[system]
        resource = f"host:local:{socket.gethostname()}"
        digest = hashlib.sha256(resource.encode("utf-8")).hexdigest()[:12]
        readable = "".join(character if character.isalnum() else "-" for character in resource)[:40]
        lock = Path(self.state_dir) / "locks" / f"{readable}-{digest}"
        lock.mkdir(parents=True)
        (lock / "owner.json").write_text(
            json.dumps({"resource": resource, "pid": os.getpid(), "host": socket.gethostname()}),
            encoding="utf-8",
        )
        completed = self.run_lab(
            "run",
            "--resource",
            "unrelated-matrix",
            "--health",
            f"local:{label}",
            "--",
            sys.executable,
            "-c",
            "raise SystemExit(0)",
        )
        report = json.loads(completed.stdout)
        self.assertEqual(completed.returncode, 75)
        self.assertEqual(report["category"], "resource_busy")
        self.assertEqual(report["busy_resource"], resource)


if __name__ == "__main__":
    unittest.main()
