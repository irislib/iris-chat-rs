#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from android_background_control_e2e import (
    CONTROL_PACKAGE, isolated_host_environment, private_ipv4, reject_unexpected_peers,
    require_control_artifacts, stage_control_config, control_foreground_ready,
)
from android_background_health import query_health


class BackgroundControlTest(unittest.TestCase):
    def test_artifacts_cannot_target_saved_or_production_accounts(self):
        for wrong in ("to.iris.chat", "to.iris.chat.backgroundtest", "to.iris.chat.backgroundcontrol", CONTROL_PACKAGE + ".test"):
            with self.subTest(wrong=wrong), patch("subprocess.check_output", return_value=f"package: name='{wrong}'"):
                with self.assertRaises(AssertionError):
                    require_control_artifacts("aapt2", "app.apk", "test.apk")
        with patch("subprocess.check_output", side_effect=[
            f"package: name='{CONTROL_PACKAGE}'", f"package: name='{CONTROL_PACKAGE}.test'",
        ]):
            require_control_artifacts("aapt2", "app.apk", "test.apk")
        with self.assertRaises(AssertionError):
            query_health(lambda *args: self.fail("Must reject before any device operation"),
                         42, "ws://127.0.0.1:1234", True, package="to.iris.chat")

    def test_private_config_staging_preserves_literal_stdin_and_requires_stopped_process(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "cache").mkdir()
            calls = []
            def adb(*args, **kwargs):
                calls.append(args)
                if "pidof" in args:
                    return ""
                self.assertEqual(("shell", "run-as", CONTROL_PACKAGE), args[:3])
                # Model adb's remote-shell joining after run-as; the JSON must stay stdin data.
                subprocess.run(" ".join(args[3:]), shell=True, cwd=directory,
                               input=kwargs["input"], text=True, check=True)
                return ""
            config = {"phase": "bootstrap", "relay_url": "$(touch unexpected)\n'quoted'"}
            stage_control_config(adb, config)
            self.assertEqual(config, json.loads(Path(directory, "cache/background-control.json").read_text()))
            self.assertFalse(Path(directory, "unexpected").exists())
            self.assertEqual(2, len(calls))
        with self.assertRaises(AssertionError):
            stage_control_config(lambda *args, **kwargs: "42", {})

    def test_host_environment_replaces_inherited_discovery_and_uses_device_publisher(self):
        device = "npub1" + "q" * 58
        with patch.dict(os.environ, {"IRIS_CHAT_SAME_HOST_HASHTREE": "1", "IRIS_CHAT_FIPS_ENABLE_WEBRTC": "1",
                                    "IRIS_FIPS_WEBSOCKET_SEED_URLS": "public", "IRIS_CALL_DESKTOP_CODEC": "1"}):
            env = isolated_host_environment("ws://127.0.0.1:1234", "192.168.1.2:1234", device, "192.168.1.3:5678")
        self.assertEqual("0", env["IRIS_CHAT_SAME_HOST_HASHTREE"])
        self.assertEqual("0", env["IRIS_CHAT_FIPS_ENABLE_WEBRTC"])
        self.assertEqual("", env["IRIS_FIPS_WEBSOCKET_SEED_URLS"])
        self.assertEqual("", env["IRIS_CHAT_FIPS_ROUTED_PEERS"])
        self.assertEqual(f"{device}=udp:192.168.1.3:5678", env["IRIS_CHAT_FIPS_STATIC_PEERS"])
        self.assertEqual(f"htree://{device}/diagnostic-control/latest", env["IRIS_UPDATE_HTREE_REF"])
        self.assertNotIn("IRIS_CALL_DESKTOP_CODEC", env)
        self.assertEqual("192.168.1.2", private_ipv4("192.168.1.2"))
        with self.assertRaises(AssertionError):
            private_ipv4("8.8.8.8")

    def test_unexpected_observed_peers_fail_before_waiting_for_readiness(self):
        base = dict(valid=True, configured_direct_peer_count=1, unexpected_connected_peer_count=0,
                    connected_peer_count=1, transports={"udp": {}})
        reject_unexpected_peers({"fips_transport": base})
        for field, value in [("configured_direct_peer_count", 2), ("unexpected_connected_peer_count", 1),
                             ("connected_peer_count", 2), ("transports", {"udp": {}, "websocket": {}})]:
            with self.subTest(field=field), self.assertRaises(AssertionError):
                reject_unexpected_peers({"fips_transport": {**base, field: value}})

    def test_cold_launch_must_be_visible_and_resumed_with_live_service_before_home(self):
        visible = f"packageName={CONTROL_PACKAGE} processName={CONTROL_PACKAGE}\n  state=RESUMED finishing=false\n  mVisible=true mVisibleRequested=true"
        service = "BackgroundMessageService isForeground=true"
        def check(activity, services):
            def adb(*args):
                if args[-1] == "activities": return activity
                if "services" in args: return services
                return ""
            return control_foreground_ready(adb)
        self.assertTrue(check(visible, service))
        for field in ("state=RESUMED", "mVisible=true", "mVisibleRequested=true"):
            self.assertFalse(check(visible.replace(field, "pending"), service))
        self.assertFalse(check(visible, "BackgroundMessageService isForeground=false"))
        self.assertFalse(check(visible.replace(CONTROL_PACKAGE, "to.iris.chat"), service))

    def test_permission_dialog_is_dismissed_without_grant_or_backgrounding(self):
        calls = []
        def adb(*args):
            calls.append(args)
            return f"""  * Hist #2: ActivityRecord{{permission}}
      packageName=com.android.permissioncontroller processName=com.android.permissioncontroller
      launchedFromUid=12345 launchedFromPackage={CONTROL_PACKAGE} launchedFromFeature=null userId=0
      mActivityComponent=com.android.permissioncontroller/.permission.ui.GrantPermissionsActivity
      state=RESUMED finishing=false
      mVisibleRequested=true mVisible=true mClientVisible=true
  * Hist #0: ActivityRecord{{app}}
      packageName={CONTROL_PACKAGE} processName={CONTROL_PACKAGE}
      state=PAUSED finishing=false
      mVisibleRequested=true mVisible=true mClientVisible=true
"""
        self.assertFalse(control_foreground_ready(adb))
        self.assertEqual([("shell", "dumpsys", "activity", "activities"),
                          ("shell", "input", "keyevent", "BACK")], calls)
        calls.clear()
        def foreign_adb(*args):
            return adb(*args).replace(f"launchedFromPackage={CONTROL_PACKAGE}", "launchedFromPackage=another.app")
        self.assertFalse(control_foreground_ready(foreign_adb))
        self.assertFalse(any("keyevent" in call for call in calls))


if __name__ == "__main__":
    unittest.main()
