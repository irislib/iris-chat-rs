#!/usr/bin/env python3
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

from android_runtime_smoke import CONTACT_CLASS, CONTACT_TESTS, verify


class AndroidRuntimeSmokeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.suite = ET.Element("testsuite", tests="6", failures="0", errors="0", skipped="0")
        for name in sorted(CONTACT_TESTS):
            ET.SubElement(self.suite, "testcase", classname=CONTACT_CLASS, name=name)

    def write(self, name="TEST-emulator.xml", modified=200):
        path = self.directory / name
        self.suite.set("tests", str(len(self.suite)))
        ET.ElementTree(self.suite).write(path)
        os.utime(path, (modified, modified))
        return path

    def test_all_six_android_tests_must_execute(self):
        self.write()
        result = verify(self.directory, 100)
        self.assertEqual(result["executed"], 6)
        self.assertEqual(set(result["contact_tests"]), CONTACT_TESTS)

    def test_missing_empty_and_stale_results_fail(self):
        for mode in ("missing", "stale", "empty"):
            with self.subTest(mode=mode):
                if mode == "stale":
                    self.write(modified=50)
                elif mode == "empty":
                    self.suite.clear()
                    self.write()
                with self.assertRaises(ValueError):
                    verify(self.directory, 100)

    def test_missing_named_test_and_wrong_class_fail(self):
        self.suite.remove(self.suite[0])
        self.write()
        with self.assertRaisesRegex(ValueError, "missing"):
            verify(self.directory, 100)
        for case in self.suite:
            case.set("classname", "another.Test")
        self.write()
        with self.assertRaisesRegex(ValueError, "missing"):
            verify(self.directory, 100)

    def test_failures_skips_and_crashes_cannot_be_success(self):
        for tag in ("failure", "error", "skipped"):
            with self.subTest(tag=tag):
                element = ET.SubElement(self.suite[0], tag)
                element.text = "Process crashed" if tag == "error" else "did not pass"
                self.write()
                with self.assertRaises(ValueError):
                    verify(self.directory, 100)
                self.suite[0].remove(element)

    def test_summary_failure_is_rejected_even_without_test_failure_node(self):
        for field in ("failures", "errors", "skipped", "disabled"):
            with self.subTest(field=field):
                self.suite.set(field, "1")
                self.write()
                with self.assertRaises(ValueError):
                    verify(self.directory, 100)
                self.suite.set(field, "0")

    def test_notrun_status_and_duplicate_success_are_rejected(self):
        self.suite[0].set("status", "notrun")
        self.write()
        with self.assertRaises(ValueError):
            verify(self.directory, 100)
        self.suite[0].set("status", "run")
        self.suite.append(ET.fromstring(ET.tostring(self.suite[0])))
        self.write()
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            verify(self.directory, 100)

    def test_multiple_devices_cannot_supply_duplicate_passes(self):
        self.write()
        self.write("TEST-other-emulator.xml")
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            verify(self.directory, 100)

    def test_unrelated_runtime_failure_is_not_hidden_by_contact_passes(self):
        other = ET.SubElement(self.suite, "testcase", classname="OtherSmokeTest", name="startup")
        ET.SubElement(other, "error").text = "Process crashed"
        self.write()
        with self.assertRaises(ValueError):
            verify(self.directory, 100)

    def test_old_failed_report_does_not_replace_current_evidence(self):
        self.write("TEST-old.xml", modified=50)
        self.write()
        self.assertEqual(verify(self.directory, 100)["executed"], 6)

    def test_truncated_xml_and_invalid_start_times_fail(self):
        self.write().write_text("<testsuite>")
        with self.assertRaises(ET.ParseError):
            verify(self.directory, 100)
        for since in (0, -1, float("nan"), float("inf")):
            with self.subTest(since=since), self.assertRaises(ValueError):
                verify(self.directory, since)

    def test_incomplete_test_count_is_rejected(self):
        path = self.write()
        self.suite.set("tests", "7")
        ET.ElementTree(self.suite).write(path)
        with self.assertRaisesRegex(ValueError, "Incomplete"):
            verify(self.directory, 100)


class NativeContractWiringTests(unittest.TestCase):
    def run_gate(self, mode):
        # Exercise the actual shell gate with fake tools; no SDK, build, app or
        # device is touched. The fake Gradle report uses the real test names.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            for name in ("test_native_contract.sh", "android_runtime_smoke.py"):
                shutil.copyfile(Path(__file__).with_name(name), scripts / name)
            (scripts / "mobile_relay_common.sh").write_text("""
local_relay_port() { echo 19765; }
local_android_loopback_relay_url() { echo ws://127.0.0.1:19765; }
local_relay_set_id() { echo synthetic; }
start_local_rust_relay() { echo synthetic; }
stop_local_rust_relay() { :; }
""")
            sdk = root / "sdk" / "platform-tools"
            sdk.mkdir(parents=True)
            adb = sdk / "adb"
            adb.write_text("#!/bin/sh\nexit 0\n")
            adb.chmod(0o755)
            android = root / "android"
            android.mkdir()
            gradle = android / "gradlew"
            gradle.write_text("""#!/usr/bin/env python3
import os, sys
from pathlib import Path
import xml.etree.ElementTree as ET
classes = sys.argv[-1].split('=', 1)[1]
with Path('calls').open('a') as log: log.write(classes + '\\n')
mode = os.environ['FIXTURE_MODE']
if mode == 'gradle_failure': sys.exit(1)
if classes != os.environ['CONTACT_CLASS']: sys.exit(0)
if mode == 'no_runtime_results': sys.exit(0)
suite = ET.Element('testsuite', tests='6', errors='0', failures='0', skipped='0')
for name in os.environ['CONTACT_TESTS'].split(','):
    ET.SubElement(suite, 'testcase', classname=classes, name=name)
if mode == 'skipped': ET.SubElement(suite[0], 'skipped')
output = Path('app/build/outputs/androidTest-results/connected/debug')
output.mkdir(parents=True)
ET.ElementTree(suite).write(output / 'TEST-emulator.xml')
""")
            gradle.chmod(0o755)
            environment = dict(os.environ, ANDROID_HOME=str(sdk.parent),
                               IRIS_ANDROID_SERIAL="synthetic", IRIS_SKIP_FAST="1",
                               IRIS_LOCAL_RELAY_PORT="19765", FIXTURE_MODE=mode,
                               CONTACT_CLASS=CONTACT_CLASS, CONTACT_TESTS=",".join(sorted(CONTACT_TESTS)))
            result = subprocess.run(["bash", str(scripts / "test_native_contract.sh")],
                                    env=environment, capture_output=True, text=True, timeout=15)
            return result, (android / "calls").read_text().splitlines()

    def test_release_gate_executes_and_checks_contact_regressions(self):
        result, calls = self.run_gate("passed")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(calls), 3)
        self.assertEqual(calls[-1], CONTACT_CLASS)

    def test_failed_native_run_is_not_retried_or_hidden(self):
        result, calls = self.run_gate("gradle_failure")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)

    def test_successful_gradle_exit_does_not_prove_runtime_execution(self):
        for mode in ("no_runtime_results", "skipped"):
            with self.subTest(mode=mode):
                result, calls = self.run_gate(mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls[-1], CONTACT_CLASS)


if __name__ == "__main__":
    unittest.main()
