"""Exercise real controller cleanup after an interrupted pre-CPU functional gate."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import android_saved_background_diagnostics as driver
from android_saved_history import fingerprint


class SavedFunctionalControllerTest(unittest.TestCase):
    def test_failed_pre_call_check_restores_test_apk_alerts_and_account_without_cpu_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = {}
            for name in ("old.apk", "new.apk", "old-test.apk", "functional-test.apk", "fixture", "relay"):
                path = root / name; path.write_text(name); artifacts[name] = path
            digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
            installed = {driver.PACKAGE: digest(artifacts["old.apk"]), driver.PACKAGE + ".test": digest(artifacts["old-test.apk"])}
            before = dict(owner="a"*64, devices=["b"*64], alerts={key: 0 for key in driver.ALERTS},
                relays=["ws://127.0.0.1:1234"], other_preferences_sha256="settings", permissions={},
                account_store_sha256="encrypted-account", history_counts=dict(threads=1, messages=1, groups=0),
                history=dict(threads={fingerprint("old"): "draft"},
                             messages={"old-id": dict(chat=fingerprint("old"), content="content")}, groups={}))
            after = copy.deepcopy(before)
            after["history"]["threads"][fingerprint("c"*64)] = "empty-draft"
            after["history"]["messages"]["setup-id"] = dict(chat=fingerprint("c"*64), content="setup")
            after["history_counts"].update(threads=2, messages=2)
            after["other_preferences_sha256"] = "settings-with-test-contact"
            after["test_contact_acceptance"] = dict(owner="c"*64, prior_preferences_sha256="settings")
            receipt = dict(package=driver.PACKAGE, owner=before["owner"], device=before["devices"][0],
                preferences={**before["alerts"], "nostr_relay_urls_json": json.dumps(before["relays"])},
                installed_artifacts=dict(installed), history_counts=before["history_counts"],
                account_store_sha256=before["account_store_sha256"])
            receipt_path = root / "account.json"; receipt_path.write_text(json.dumps(receipt))
            output = root / "output"
            argv = ["driver", "--serial", "synthetic-device", "--app-apk", str(artifacts["new.apk"]),
                "--previous-apk", str(artifacts["old.apk"]), "--test-apk", str(artifacts["old-test.apk"]),
                "--aapt2", str(root / "aapt2"), "--relay-bin", str(artifacts["relay"]),
                "--account-receipt", str(receipt_path), "--output", str(output), "--require-fips-services",
                "--fixture-bin", str(artifacts["fixture"]), "--fixture-dir", str(root / "sender"),
                "--functional-test-apk", str(artifacts["functional-test.apk"])]
            manifest = '''E: instrumentation (line=9)
              A: http://schemas.android.com/apk/res/android:name(0x01)="androidx.test.runner.AndroidJUnitRunner"
              A: http://schemas.android.com/apk/res/android:targetPackage(0x02)="to.iris.chat.backgroundtest"
            '''
            def probe(command, **kwargs):
                if "xmltree" in command: return manifest
                if "verify" in command: return "V2 Signer: certificate SHA-256 digest: " + "a"*64
                package = driver.PACKAGE + (".test" if Path(command[-1]).name in ("old-test.apk", "functional-test.apk") else "")
                return "package: name='" + package + "'"
            methods = []; commands = []; visible = False
            def run(command, **kwargs):
                nonlocal visible
                parts = command[3:]; commands.append(parts); value = ""
                if parts[0] == "install":
                    path = Path(parts[-1]); package = driver.PACKAGE + (".test" if "test" in path.name else "")
                    installed[package] = digest(path); value = "Success"
                elif parts[:3] == ["shell", "am", "instrument"]:
                    method = parts[parts.index("class") + 1].split("#")[1]; methods.append(method)
                    value = "OK (1 test)"
                    if method == "connect_saved_normal_fixture":
                        self.assertIn(before["devices"][0], parts)
                        value += "\nINSTRUMENTATION_STATUS: contact_added=true\nINSTRUMENTATION_STATUS: setup_message_sent=true"
                elif parts[:3] == ["shell", "am", "start"]:
                    visible = True; value = "Status: ok"
                elif parts[:3] == ["shell", "input", "keyevent"] and parts[-1] in ("HOME", "SLEEP"):
                    visible = False
                elif parts[:3] == ["shell", "dumpsys", "power"]: value = "mWakefulness=Asleep"
                elif parts[:4] == ["shell", "dumpsys", "activity", "services"]:
                    value = "BackgroundMessageService isForeground=true"
                elif parts[:4] == ["shell", "dumpsys", "activity", "activities"]:
                    value = f"packageName={driver.PACKAGE} " + ("state=RESUMED mVisible=true mVisibleRequested=true" if visible
                        else "state=STOPPED mAppStopped=true mVisible=false mVisibleRequested=false")
                return subprocess.CompletedProcess(command, 0, value, "")
            fixture = MagicMock()
            fixture.ready = dict(owner="c"*64, device="d"*64, device_npub="npub1example", invite="synthetic-invite")
            fixture.before_idle.side_effect = RuntimeError("pre-call failure")
            relay = MagicMock(); relay.poll.return_value = None
            with patch.object(sys, "argv", argv), patch.object(driver, "acquire", return_value=("lock", {})), \
                 patch.object(driver, "release") as release, patch.object(driver.subprocess, "check_output", side_effect=probe), \
                 patch.object(driver.subprocess, "run", side_effect=run), patch.object(driver.subprocess, "Popen", return_value=relay), \
                 patch.object(driver.socket, "create_connection", return_value=MagicMock()), \
                 patch.object(driver, "read_saved_state", side_effect=[copy.deepcopy(before), copy.deepcopy(before), after]), \
                 patch.object(driver, "installed_hash", side_effect=lambda adb, package: installed[package]), \
                 patch("android_saved_fixture.SavedFixture", return_value=fixture):
                with self.assertRaisesRegex(RuntimeError, "pre-call failure"): driver.main()
            self.assertEqual([], json.loads((output / "cleanup.json").read_text())["errors"])
            self.assertFalse((output / "profile-ready.json").exists())
            self.assertEqual(digest(artifacts["old-test.apk"]), installed[driver.PACKAGE + ".test"])
            self.assertEqual(["resume_saved_background_receiver", "connect_saved_normal_fixture", "stop_when_alerts_disabled"], methods)
            self.assertFalse(any("grant" in command for command in commands))
            fixture.stop.assert_called_once(); release.assert_called_once_with("lock")


if __name__ == "__main__": unittest.main()
