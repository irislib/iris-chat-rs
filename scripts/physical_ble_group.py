#!/usr/bin/env python3
"""Opt-in real-device gate: group messages over Bluetooth with both phones offline.

The iPhone creates a group with the isolated Android `.blegate` account and a
fresh CLI account on an online Linux host. Both phones then lose IP access
(the Android side is verified to have no address or route throughout), swap
group messages over FIPS Bluetooth, and reconnect to receive the online
member's message. The online member must also receive both offline messages.

Uses development build-for-testing products like `physical_ble_idle.py`,
preserves the Android test account and radios, and always restores the
iPhone's Wi-Fi and mobile data.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import plistlib
import shlex
import signal
import subprocess
import time
import uuid

import android_ble_preservation
from physical_ble_idle import Gate, stop, write_json

TEST = "FipsBlePhysicalUITests/testGroupMessagesOverFipsBleWhileOffline"


def require_group_preserved(before, after):
    """Like the direct-message check, but a test group with new members may add chats.

    The account, its devices, servers, permissions and Bluetooth settings must be
    restored, existing rows must be unchanged, and a run adds at most one group.
    New messages are counted, not restricted: late copies from an earlier run's
    test group may arrive in that group's existing thread.
    """
    for field in ("owner", "devices", "relays", "permissions", "alerts", "permission_flags", "ble_settings"):
        assert after[field] == before[field], "Saved test account changed: " + field
    additions = {}
    for table in ("threads", "messages", "groups"):
        old, new = before["history"][table], after["history"][table]
        assert all(key in new and new[key] == value for key, value in old.items()), "Existing history changed: " + table
        added = new.keys() - old.keys()
        if table == "groups":
            assert len(added) <= 1, "More than one test group was added"
        additions[table] = len(added)
    return additions


# Applies only within this gate's process.
android_ble_preservation.require_ble_preserved = require_group_preserved
RESTORE_TEST = "FipsBlePhysicalUITests/testRestorePhoneNetworkAfterBleGroup"


class GroupGate(Gate):
    def __init__(self, args):
        super().__init__(args)
        tag = uuid.uuid4().hex[:8]
        self.run_id = "ble-group-" + tag
        # adb shell splits extras on spaces, so names and messages have none.
        self.group = "BLE-group-" + tag
        self.msg = {key: f"{key}-{tag}" for key in (
            "android-ready", "linux-ready", "android-offline", "iphone-offline",
            "android-ack", "linux-while-offline")}
        self.linux_dir = f"~/.cache/iris-ble-group/{self.run_id}"
        self.phone_previous = None

    # --- online Linux member (CLI) ---
    def linux(self, *argv, timeout=120):
        # Explicit syncs only: a background sync would lock the folder for the next command.
        remote = " ".join([self.args.linux_iris, "--json", "--no-background-sync", "--data-dir", self.linux_dir,
                           *(shlex.quote(a) for a in argv)])
        out = self.command(["ssh", self.args.linux_host, remote], "linux-" + argv[0], timeout)
        start, end = out.find("{"), out.rfind("}")
        if start < 0:
            return {}
        parsed = json.loads(out[start:end + 1])
        if parsed.get("status") != "ok":
            raise RuntimeError("Linux CLI " + argv[0] + " failed; see private log")
        return parsed.get("data", {})

    def linux_sync(self):
        # The CLI only receives while a command runs, so each poll syncs first.
        self.linux("sync", "--wait-ms", "15000", timeout=90)

    def linux_group_id(self):
        self.linux_sync()
        for group in self.linux("group", "list").get("groups", []):
            if group.get("name") == self.group:
                return group["chat_id"]
        return None

    def linux_has(self, group_id, message):
        self.linux_sync()
        return message in json.dumps(self.linux("group", "read", group_id, "--limit", "200"))

    def wait_until(self, label, check, timeout, interval=5):
        deadline = time.monotonic() + timeout
        while True:
            value = check()
            if value:
                return value
            if time.monotonic() > deadline:
                raise RuntimeError(label + " timed out")
            time.sleep(interval)

    # --- iPhone UI test ---
    def xctestrun(self, test, environment, name):
        source = self.args.xctestrun.resolve()
        def rebase(value):
            if isinstance(value, str):
                return value.replace("__TESTROOT__", str(source.parent))
            if isinstance(value, list):
                return [rebase(v) for v in value]
            if isinstance(value, dict):
                return {k: rebase(v) for k, v in value.items()}
            return value
        data = rebase(plistlib.loads(source.read_bytes()))
        targets = ([t for c in data["TestConfigurations"] for t in c.get("TestTargets", [])]
                   if "TestConfigurations" in data else [v for v in data.values() if isinstance(v, dict)])
        target = next(t for t in targets if "IrisChatUITests" in str(t.get("TestBundlePath", "")))
        # Run only the selected UI test, not the unit-test bundle in the same plan.
        if "TestConfigurations" in data:
            for configuration in data["TestConfigurations"]:
                configuration["TestTargets"] = [t for t in configuration.get("TestTargets", []) if t is target]
        else:
            for key in [k for k, v in data.items()
                        if isinstance(v, dict) and "TestBundlePath" in v and v is not target]:
                del data[key]
        for key in ("EnvironmentVariables", "UITargetAppEnvironmentVariables"):
            target[key] = {k: v for k, v in target.get(key, {}).items()
                           if not k.startswith(("IRIS_UI_TEST_", "IRIS_FIPS_"))}
        target.setdefault("EnvironmentVariables", {}).update(environment)
        target["OnlyTestIdentifiers"] = [test]
        path = self.out / name
        path.write_bytes(plistlib.dumps(data))
        return path

    def ios_test(self, xctestrun, label):
        command = ["xcodebuild", "-xctestrun", str(xctestrun), "-destination", "platform=iOS,id=" + self.iphone,
                   "-resultBundlePath", str(self.out / (label + ".xcresult")),
                   "-parallel-testing-enabled", "NO", "test-without-building"]
        log = (self.out / (label + ".log")).open("w")
        return subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)

    def ios_log(self):
        return (self.out / "ios-group.log").read_text(errors="replace")

    def wait_ios(self, marker, timeout):
        def seen():
            log = self.ios_log()
            if marker in log:
                return True
            if self.ios.poll() is not None:
                raise RuntimeError(f"iPhone test ended before {marker}; see ios-group.log")
            return False
        self.wait_until("iPhone " + marker, seen, timeout, interval=2)
        for line in self.ios_log().splitlines():
            if line.startswith("IRIS_FIPS_PHONE_NETWORK_PREVIOUS="):
                self.phone_previous = line.split("=", 1)[1].strip()

    def require_offline(self, label):
        if not self.offline():
            raise RuntimeError("Android gained an IP path during " + label)

    def execute(self):
        self.preflight()
        self.saved_account.started = True
        identity = self.action("report_logged_in_identity", {"app_package": "to.iris.chat.blegate",
            "public_key_hex": self.saved_account.before["owner"]})
        android_npub = identity.get("npub", "")
        if not android_npub.startswith("npub1"):
            raise ValueError("The isolated Android account did not report its identity")

        account = self.linux("account", "create", "--name", "BLE online member", timeout=180)
        linux_npub = account.get("npub", "")
        if not linux_npub.startswith("npub1"):
            raise ValueError("The Linux CLI account did not report a user ID")
        self.result.update(androidNpub=android_npub, linuxNpub=linux_npub, group=self.group)
        # The isolated Android account normally has no message servers. Give it the
        # defaults a real phone has; preservation restores the saved list afterwards.
        servers = self.linux("relay", "list").get("message_servers", [])
        if not servers:
            raise ValueError("The Linux CLI reported no default message servers")
        self.action("set_relays_from_args", {}, ["-e", "relay_urls", ",".join(servers)])

        environment = {
            "IRIS_FIPS_PHYSICAL_RUN_ID": self.run_id,
            "IRIS_FIPS_GROUP_MEMBERS": android_npub + "," + linux_npub,
            "IRIS_FIPS_GROUP_NAME": self.group,
            "IRIS_FIPS_GROUP_READY_MESSAGES": self.msg["android-ready"] + "," + self.msg["linux-ready"],
            "IRIS_FIPS_GROUP_EXPECT_OFFLINE": self.msg["android-offline"],
            "IRIS_FIPS_GROUP_SEND_OFFLINE": self.msg["iphone-offline"],
            "IRIS_FIPS_GROUP_OFFLINE_ACK": self.msg["android-ack"],
            "IRIS_FIPS_GROUP_EXPECT_ONLINE": self.msg["linux-while-offline"],
        }
        self.ios = self.ios_test(self.xctestrun(TEST, environment, "group.xctestrun"), "ios-group")
        self.wait_ios("IRIS_FIPS_GROUP_CREATED", 420)

        # Online phase: both other members receive the group and post a ready message.
        joined = self.action("wait_for_group_named_from_args", {}, [
            "-e", "group_name", self.group, "-e", "timeout_secs", "240"])
        chat_id = joined["chat_id"]
        self.action("send_message_from_args", {}, ["-e", "chat_id", chat_id,
                    "-e", "message", self.msg["android-ready"]])
        linux_group = self.wait_until("Linux joins the group", self.linux_group_id, 240)
        self.linux("group", "send", linux_group, self.msg["linux-ready"])
        self.result.update(androidChatId=chat_id, linuxGroupId=linux_group)

        # Offline phase: iPhone switches off Wi-Fi/mobile data, Android loses every IP path.
        self.wait_ios("IRIS_FIPS_IPHONE_OFFLINE", 420)
        self.adb("svc", "wifi", "disable")
        self.adb("svc", "data", "disable")
        self.adb("settings", "put", "global", "mobile_data", "0")
        self.wait_until("Android offline", self.offline, 30, interval=1)
        print("Both phones offline. Exchanging group messages over Bluetooth.", flush=True)
        with (self.out / "android-exchange.log").open("w") as log:
            self.android = subprocess.Popen(self.action_command("nearby_group_exchange_from_args", [
                "-e", "chat_id", chat_id, "-e", "send_message", self.msg["android-offline"],
                "-e", "expect_message", self.msg["iphone-offline"], "-e", "ack_message", self.msg["android-ack"],
                "-e", "timeout_secs", "300"]), stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        self.linux("group", "send", linux_group, self.msg["linux-while-offline"])
        deadline = time.monotonic() + 420
        while "IRIS_FIPS_GROUP_OFFLINE_ACKED" not in self.ios_log():
            self.require_offline("the Bluetooth exchange")
            if self.ios.poll() is not None:
                raise RuntimeError("iPhone test ended during the offline exchange; see ios-group.log")
            if self.android.poll() not in (None, 0):
                raise RuntimeError("Android exchange failed; see android-exchange.log")
            if time.monotonic() > deadline:
                raise RuntimeError("Offline Bluetooth group exchange timed out")
            time.sleep(3)
        if self.android.wait(timeout=120) or "OK (1 test)" not in (self.out / "android-exchange.log").read_text():
            raise RuntimeError("Android did not confirm the offline exchange; see android-exchange.log")
        self.require_offline("the end of the Bluetooth exchange")
        self.result["offlineExchange"] = True

        # Reconnect: everyone converges through the online member's relays.
        for key, service in (("wifi_on", "wifi"), ("mobile_data", "data")):
            if self.snapshot[key] == "1":
                self.adb("svc", service, "enable")
        self.adb("settings", "put", "global", "mobile_data", self.snapshot["mobile_data"])
        self.action("wait_for_message_from_args", {}, ["-e", "chat_id", chat_id,
                    "-e", "message", self.msg["linux-while-offline"], "-e", "timeout_secs", "300"])
        self.wait_ios("IRIS_FIPS_GROUP_DONE", 420)
        for key in ("android-offline", "iphone-offline"):
            self.wait_until("Linux receives " + key, lambda key=key: self.linux_has(linux_group, self.msg[key]), 300)
        if self.ios.wait(timeout=120):
            raise RuntimeError("iPhone group test failed; see ios-group.log")
        self.result.update(ok=True, onlineCatchUp=True)

    def restore(self):
        # The iPhone must never stay offline, even after a failure mid-test. Once it
        # went offline, only its running test can turn Wi-Fi back on (a new test
        # runner cannot launch offline), so let that test finish instead of killing it.
        try:
            if self.ios is not None and self.ios.poll() is None and "IRIS_FIPS_IPHONE_OFFLINE" in self.ios_log():
                try:
                    self.ios.wait(timeout=1200)
                except subprocess.TimeoutExpired:
                    self.result["iosStillRunning"] = True
            stop(self.ios)
            spec = self.phone_previous or ("wifi:1" if "IRIS_FIPS_GROUP_MEMBERS_READY" in self.ios_log() else "")
            if spec:
                restore = self.ios_test(self.xctestrun(RESTORE_TEST, {"IRIS_FIPS_RESTORE_NETWORK": spec},
                                                       "restore.xctestrun"), "ios-restore")
                self.restored["ios_network"] = restore.wait(timeout=300) == 0
        except Exception as error:
            self.restored["ios_network"] = False
            self.result["iosRestoreError"] = str(error)
        finally:
            self.ios = None
        try:
            # "[b]le-group-…" keeps pkill from matching this ssh command itself.
            pattern = "[" + self.run_id[0] + "]" + self.run_id[1:]
            # Separate calls: a command line naming the folder would match its own pkill.
            subprocess.run(["ssh", self.args.linux_host, "pkill -f " + shlex.quote(pattern)], timeout=30)
            self.command(["ssh", self.args.linux_host, "rm -rf " + self.linux_dir], "linux-cleanup", 30)
            self.restored["linux_cleanup"] = True
        except Exception:
            self.restored["linux_cleanup"] = False
        ok = self.result.get("ok")
        super().restore()
        if ok and all(v is True for v in self.restored.values()):
            self.result["ok"] = True
            write_json(self.out / "result.json", self.result)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--iphone", required=True)
    p.add_argument("--android-serial", required=True)
    p.add_argument("--xctestrun", type=Path, required=True)
    p.add_argument("--artifact-dir", type=Path, required=True)
    p.add_argument("--linux-host", required=True, help="SSH host of the online member")
    p.add_argument("--linux-iris", required=True, help="Path to the iris CLI on that host")
    args = p.parse_args()
    args.sample_seconds = 60
    os.umask(0o077)
    gate = GroupGate(args)
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt()
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    try:
        gate.execute()
    except (Exception, KeyboardInterrupt) as error:
        gate.result.update(ok=False, error=str(error) or "Interrupted")
    finally:
        gate.restore()
    print(json.dumps(gate.result, indent=2))
    return 0 if gate.result.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
