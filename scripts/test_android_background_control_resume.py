import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from android_background_control_resume import CONTROL_PACKAGE, installed_artifact_hashes, paired_resume_settings


class ControlResumeTest(unittest.TestCase):
    def test_installed_bytes_must_match_both_frozen_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            app, tests = Path(directory, "app.apk"), Path(directory, "test.apk")
            app.write_bytes(b"app fixture"); tests.write_bytes(b"test fixture")
            expected = {CONTROL_PACKAGE: app, CONTROL_PACKAGE + ".test": tests}
            def adb(*args):
                if "path" in args:
                    return "package:/data/app/~~base=/" + args[-1] + "-fixture=/base.apk"
                package = CONTROL_PACKAGE + ".test" if ".test-" in args[-1] else CONTROL_PACKAGE
                return hashlib.sha256(expected[package].read_bytes()).hexdigest() + "  path"
            self.assertEqual(2, len(installed_artifact_hashes(adb, app, tests)))
            def wrong(*args):
                return "0" * 64 + "  path" if "sha256sum" in args else adb(*args)
            with self.assertRaises(AssertionError): installed_artifact_hashes(wrong, app, tests)

    def test_resume_preserves_exact_pair_and_rejects_changed_or_measured_attempt(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            (source / "host-account").mkdir()
            (source / "host-account/core.sqlite3").touch()
            receipt = source / "host-account/fixture-account-bundle.json"
            receipt.write_text(json.dumps(dict(version=1, owner_nsec="private-owner", device_nsec="private-device",
                                                owner_pubkey_hex="b" * 64)))
            receipt.chmod(0o600)
            for name, count in (("parser-tests.log", 10), ("pairing.log", 1)):
                (source / name).write_text(f"OK ({count} test{'s' if count != 1 else ''})")
            (source / "bootstrap.log").write_text("OK (1 test)\nINSTRUMENTATION_STATUS: owner=" + "a" * 64 +
                "\nINSTRUMENTATION_STATUS: device_npub=npub1" + "q" * 58 + "\nINSTRUMENTATION_STATUS: udp_port=20123\n")
            (source / "normal-launch.log").write_text(CONTROL_PACKAGE + "/to.iris.chat.MainActivity")
            (source / "cleanup.json").write_text('{"errors":[]}')
            config = dict(phase="paired", relay_url="ws://127.0.0.1:20125", peer_npub="npub1" + "p" * 58,
                          peer_udp="192.168.1.2:20124", local_udp_port=20123)
            def adb(*args, **kwargs):
                return "" if "pidof" in args else json.dumps(config)
            result = paired_resume_settings(adb, source, "192.168.1.2")
            self.assertEqual(config, result["config"])
            self.assertEqual("b" * 64, result["host_owner"])
            self.assertNotIn("private-", json.dumps(result, default=str))
            with self.assertRaises(AssertionError): paired_resume_settings(adb, source, "192.168.1.3")
            receipt.chmod(0o644)
            with self.assertRaises(AssertionError): paired_resume_settings(adb, source, "192.168.1.2")
            receipt.chmod(0o600)
            (source / "profile-ready.json").write_text('{}')
            with self.assertRaises(AssertionError): paired_resume_settings(adb, source, "192.168.1.2")


if __name__ == "__main__": unittest.main()
