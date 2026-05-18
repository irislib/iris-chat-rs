#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


ROOT_DIR = Path(__file__).resolve().parent.parent
IOS_HARNESS = ROOT_DIR / "scripts" / "run_ios_harness.py"
IOS_SIMULATORS = ROOT_DIR / "scripts" / "run_ios_simulators.sh"
IOS_BUILD = ROOT_DIR / "scripts" / "ios-build"
ANDROID_HARNESS = ROOT_DIR / "scripts" / "run_harness.py"
ANDROID_EMULATORS = ROOT_DIR / "scripts" / "run_android_emulators.sh"
LOCAL_RELAY_BINARY = ROOT_DIR / "core" / "target" / "debug" / "local_nostr_relay"
PENDING_PUBLISHES = ROOT_DIR / "scripts" / "pending_relay_publishes.py"
ANDROID_RUNNER = "to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CLASS = "to.iris.chat.RealRelayHarnessTest"
ANDROID_APP_PACKAGE = "to.iris.chat.debug"
ANDROID_TEST_PACKAGE = "to.iris.chat.test"
STATUS_RE = re.compile(r"^(?:HARNESS_STATUS|INSTRUMENTATION_STATUS): ([^=]+)=(.*)$")
RAW_STATUS_RE = re.compile(r"^([a-z_][a-z0-9_]*)=(.*)$")


def run(
    command: list[str],
    *,
    env: dict[str, str] | None = None,
    cwd: Path = ROOT_DIR,
    capture: bool = True,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    print("+ " + " ".join(shlex.quote(part) for part in command), flush=True)
    completed = subprocess.run(
        command,
        cwd=str(cwd),
        env=env,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.STDOUT if capture else None,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if capture and completed.stdout:
        print(completed.stdout, end="")
    if check and completed.returncode != 0:
        raise SystemExit(completed.returncode)
    return completed


def host_ip(interface: str | None) -> str:
    if interface:
        value = subprocess.run(
            ["ipconfig", "getifaddr", interface],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        ).stdout.strip()
        if value:
            return value
    route = subprocess.run(
        ["route", "get", "default"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    ).stdout
    match = re.search(r"interface:\s+(\S+)", route)
    if match:
        value = subprocess.run(
            ["ipconfig", "getifaddr", match.group(1)],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        ).stdout.strip()
        if value:
            return value
    return "127.0.0.1"


def parse_status(output: str) -> dict[str, str]:
    statuses: dict[str, str] = {}
    for line in output.splitlines():
        stripped = line.strip()
        match = STATUS_RE.match(stripped) or RAW_STATUS_RE.match(stripped)
        if match:
            statuses[match.group(1)] = match.group(2)
    return statuses


def wait_for_status_file(path: Path, key: str, timeout_secs: int) -> str:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        if path.exists():
            value = parse_status(path.read_text(encoding="utf-8", errors="replace")).get(key)
            if value:
                return value
        time.sleep(1)
    raise SystemExit(f"Timed out waiting for {key} in {path}")


def wait_for_tcp(host: str, port: int, timeout_secs: int) -> None:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1):
                return
        except OSError:
            time.sleep(0.5)
    raise SystemExit(f"Timed out waiting for TCP {host}:{port}")


def tcp_open(host: str, port: int) -> bool:
    try:
        with socket.create_connection((host, port), timeout=1):
            return True
    except OSError:
        return False


class Scenario:
    def __init__(self, config_path: Path):
        self.config_path = config_path
        self.config = json.loads(config_path.read_text(encoding="utf-8"))
        self.name = self.config["name"]
        self.work_dir = Path(self.config.get("work_dir") or f"/tmp/iris-mobile-scenario-{self.name}")
        self.state_path = self.work_dir / "state.json"
        self.state: dict[str, Any] = self.load_state()

    def load_state(self) -> dict[str, Any]:
        if self.state_path.exists():
            return json.loads(self.state_path.read_text(encoding="utf-8"))
        return {"name": self.name, "devices": {}, "users": {}, "groups": {}, "relay": {}}

    def save_state(self) -> None:
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.state_path.write_text(json.dumps(self.state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        env_lines: list[str] = []
        relay = self.state.get("relay", {})
        for key in ("url", "drop_file", "label", "port"):
            if key in relay:
                env_lines.append(f"RELAY_{key.upper()}={relay[key]}")
        for device_id, device in sorted(self.state.get("devices", {}).items()):
            prefix = device_id.upper().replace("-", "_")
            for key in ("udid", "serial", "run_id", "owner_hex", "owner_npub", "device_hex", "data_dir"):
                if key in device:
                    env_lines.append(f"{prefix}_{key.upper()}={device[key]}")
        for group_id, group in sorted(self.state.get("groups", {}).items()):
            prefix = group_id.upper().replace("-", "_")
            for key in ("chat_id", "group_id", "name"):
                if key in group:
                    env_lines.append(f"{prefix}_{key.upper()}={group[key]}")
        (self.work_dir / "state.env").write_text("\n".join(env_lines) + "\n", encoding="utf-8")

    def relay_config(self) -> dict[str, Any]:
        relay = dict(self.config.get("relay") or {})
        relay.setdefault("port", 4848)
        relay.setdefault("label", f"iris.scenario.{self.name}.relay")
        relay.setdefault("drop_file", str(self.work_dir / "drop-events.txt"))
        relay.setdefault("log_file", str(self.work_dir / "relay.log"))
        relay.setdefault("set_id", f"local-{self.name}")
        relay.setdefault("bind_host", "0.0.0.0")
        relay.setdefault("host_interface", "en0")
        return relay

    def relay_url(self) -> str:
        relay = self.relay_config()
        return relay.get("url") or f"ws://{host_ip(relay.get('host_interface'))}:{int(relay['port'])}"

    def scenario_env(self) -> dict[str, str]:
        env = os.environ.copy()
        relay = self.relay_config()
        env["IRIS_DEFAULT_RELAYS"] = self.relay_url()
        env["IRIS_RELAY_SET_ID"] = str(relay["set_id"])
        env["IRIS_TRUSTED_TEST_BUILD"] = "true"
        return env

    def android_sdk_dir(self) -> Path:
        value = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
        local_properties = ROOT_DIR / "android" / "local.properties"
        if not value and local_properties.exists():
            for line in local_properties.read_text(encoding="utf-8", errors="replace").splitlines():
                if line.startswith("sdk.dir="):
                    value = line.split("=", 1)[1].strip()
        if not value:
            raise SystemExit("Android SDK path not found. Set ANDROID_HOME, ANDROID_SDK_ROOT, or android/local.properties.")
        return Path(value)

    def adb(self) -> Path:
        adb = self.android_sdk_dir() / "platform-tools" / "adb"
        if not adb.exists():
            raise SystemExit(f"adb not found at {adb}")
        return adb

    def android_relay_url(self) -> str:
        relay = self.relay_config()
        return relay.get("android_url") or f"ws://10.0.2.2:{int(relay['port'])}"

    def stop_relay(self) -> None:
        label = str(self.relay_config()["label"])
        run(["launchctl", "remove", label], capture=True, check=False)

    def ensure_relay_binary(self) -> None:
        if LOCAL_RELAY_BINARY.exists():
            return
        run(
            [
                "cargo",
                "build",
                "--manifest-path",
                str(ROOT_DIR / "core" / "Cargo.toml"),
                "--features",
                "local-relay-bin",
                "--bin",
                "local_nostr_relay",
            ]
        )

    def start_relay(self) -> None:
        relay = self.relay_config()
        self.work_dir.mkdir(parents=True, exist_ok=True)
        drop_file = Path(relay["drop_file"])
        drop_file.parent.mkdir(parents=True, exist_ok=True)
        drop_file.touch()
        log_file = Path(relay["log_file"])
        log_file.parent.mkdir(parents=True, exist_ok=True)
        self.ensure_relay_binary()
        self.stop_relay()
        port = int(relay["port"])
        if tcp_open("127.0.0.1", port):
            raise SystemExit(
                f"TCP port {port} is already in use. Stop the other local relay or change relay.port."
            )
        bind_addr = f"{relay['bind_host']}:{port}"
        command = (
            f"IRIS_LOCAL_RELAY_DROP_EVENT_IDS_FILE={shlex.quote(str(drop_file))} "
            f"exec {shlex.quote(str(LOCAL_RELAY_BINARY))} {shlex.quote(bind_addr)} "
            f">> {shlex.quote(str(log_file))} 2>&1"
        )
        run(["launchctl", "submit", "-l", str(relay["label"]), "--", "/bin/bash", "-lc", command])
        wait_for_tcp("127.0.0.1", port, 30)
        self.state["relay"] = {
            "label": relay["label"],
            "port": port,
            "url": self.relay_url(),
            "drop_file": str(drop_file),
            "log_file": str(log_file),
            "set_id": relay["set_id"],
        }
        self.save_state()

    def boot_ios(self) -> None:
        names = [
            device["simulator"]
            for device in self.config.get("devices", [])
            if device.get("platform") == "ios" and device.get("simulator")
        ]
        if not names:
            return
        completed = run([str(IOS_SIMULATORS), "--no-open", *names])
        udids: dict[str, str] = {}
        for line in completed.stdout.splitlines():
            match = re.match(r"^(.+) ([0-9A-F-]{36}) ", line)
            if match:
                udids[match.group(1)] = match.group(2)
        for device in self.config.get("devices", []):
            if device.get("platform") != "ios":
                continue
            device_id = device["id"]
            entry = self.state["devices"].setdefault(device_id, {})
            entry["platform"] = "ios"
            entry["run_id"] = device.get("run_id", device_id)
            entry["user"] = device.get("user", device_id)
            entry["simulator"] = device.get("simulator", "")
            if device.get("udid"):
                entry["udid"] = device["udid"]
            elif device.get("simulator") in udids:
                entry["udid"] = udids[device["simulator"]]
            else:
                raise SystemExit(f"Unable to resolve UDID for iOS device {device_id}")
        self.save_state()

    def boot_android(self) -> None:
        avds = [
            device["avd"]
            for device in self.config.get("devices", [])
            if device.get("platform") == "android" and device.get("avd")
        ]
        if not avds:
            return
        command = [str(ANDROID_EMULATORS)]
        if self.config.get("android", {}).get("headless", True):
            command.append("--headless")
        if self.config.get("android", {}).get("wipe_data", False):
            command.append("--wipe-data")
        command.extend(avds)
        completed = run(command, env=self.scenario_env())
        serials: dict[str, str] = {}
        for line in completed.stdout.splitlines():
            match = re.match(r"^(.+) (\S+)$", line.strip())
            if match:
                serials[match.group(1)] = match.group(2)
        for device in self.config.get("devices", []):
            if device.get("platform") != "android":
                continue
            device_id = device["id"]
            entry = self.state["devices"].setdefault(device_id, {})
            entry["platform"] = "android"
            entry["run_id"] = device.get("run_id", device_id)
            entry["user"] = device.get("user", device_id)
            entry["avd"] = device.get("avd", "")
            if device.get("serial"):
                entry["serial"] = device["serial"]
            elif device.get("avd") in serials:
                entry["serial"] = serials[device["avd"]]
            else:
                raise SystemExit(f"Unable to resolve serial for Android device {device_id}")
        self.save_state()

    def build_ios(self) -> None:
        if not self.config.get("ios", {}).get("build", True):
            return
        run([str(IOS_BUILD), "ios-xcframework"], env=self.scenario_env())

    def build_android(self) -> None:
        android_devices = [
            device for device in self.state.get("devices", {}).values()
            if device.get("platform") == "android"
        ]
        if not android_devices or not self.config.get("android", {}).get("build", True):
            return
        env = self.scenario_env()
        env["ANDROID_HOME"] = str(self.android_sdk_dir())
        env["IRIS_DEBUG_RELAYS"] = self.android_relay_url()
        env["IRIS_DEBUG_RELAY_SET_ID"] = str(self.relay_config()["set_id"])
        run(["./gradlew", ":app:assembleDebug", ":app:assembleDebugAndroidTest"], cwd=ROOT_DIR / "android", env=env)
        apk = ROOT_DIR / "android" / "app" / "build" / "outputs" / "apk" / "debug" / "app-debug.apk"
        test_apk = ROOT_DIR / "android" / "app" / "build" / "outputs" / "apk" / "androidTest" / "debug" / "app-debug-androidTest.apk"
        for device in android_devices:
            serial = device["serial"]
            run([str(self.adb()), "-s", serial, "install", "-r", "-d", str(apk)], env=env)
            run([str(self.adb()), "-s", serial, "install", "-r", "-d", "-t", str(test_apk)], env=env)

    def ios_data_dir(self, udid: str) -> str:
        completed = run(
            ["xcrun", "simctl", "get_app_container", udid, "to.iris.chat", "group.to.iris.chat"],
            capture=True,
        )
        return str(Path(completed.stdout.strip()) / "iris-chat")

    def ios_harness(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
        rebuild: bool = False,
        check_code: bool = True,
    ) -> dict[str, str]:
        device = self.state["devices"][device_id]
        command = [
            sys.executable,
            str(IOS_HARNESS),
            "--udid",
            device["udid"],
            "--use-app-storage",
            "--run-id",
            device["run_id"],
            "--action",
            action,
        ]
        if reset:
            command.append("--reset")
        if rebuild:
            command.append("--rebuild")
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        completed = run(command, env=self.scenario_env())
        if check_code and "INSTRUMENTATION_CODE: -1" not in completed.stdout:
            raise SystemExit(f"iOS harness action failed or did not report success: {action} on {device_id}")
        statuses = parse_status(completed.stdout)
        log_path = self.work_dir / f"{device_id}-{action}.log"
        log_path.write_text(completed.stdout, encoding="utf-8")
        return statuses

    def android_harness(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
        rebuild: bool = False,
        check_code: bool = True,
    ) -> dict[str, str]:
        del rebuild
        device = self.state["devices"][device_id]
        env = self.scenario_env()
        env["ANDROID_HOME"] = str(self.android_sdk_dir())
        adb = str(self.adb())
        if reset:
            run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_APP_PACKAGE], env=env, check=False)
            run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_TEST_PACKAGE], env=env, check=False)
        command = [
            sys.executable,
            str(ANDROID_HARNESS),
            "--adb",
            adb,
            "--serial",
            device["serial"],
            "--runner",
            ANDROID_RUNNER,
            "--class-name",
            ANDROID_CLASS,
            "--test-name",
            action,
        ]
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        completed = run(command, env=env)
        if check_code and "INSTRUMENTATION_CODE: -1" not in completed.stdout:
            raise SystemExit(f"Android harness action failed or did not report success: {action} on {device_id}")
        statuses = parse_status(completed.stdout)
        log_path = self.work_dir / f"{device_id}-{action}.log"
        log_path.write_text(completed.stdout, encoding="utf-8")
        return statuses

    def harness(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
        rebuild: bool = False,
        check_code: bool = True,
    ) -> dict[str, str]:
        platform = self.state["devices"][device_id]["platform"]
        if platform == "ios":
            return self.ios_harness(
                device_id,
                action,
                args=args,
                reset=reset,
                rebuild=rebuild,
                check_code=check_code,
            )
        if platform == "android":
            return self.android_harness(
                device_id,
                action,
                args=args,
                reset=reset,
                rebuild=rebuild,
                check_code=check_code,
            )
        raise SystemExit(f"Unsupported platform for harness: {platform}")

    def create_account(self, device: dict[str, Any], *, rebuild: bool) -> None:
        device_id = device["id"]
        statuses = self.harness(
            device_id,
            "create_account_and_report_identity",
            reset=bool(device.get("reset", False)),
            rebuild=rebuild,
            args={
                "display_name": device.get("display_name", device_id),
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": str(device.get("relay_drain_timeout_secs", 180)),
            },
        )
        self.record_identity(device_id, statuses)

    def record_identity(self, device_id: str, statuses: dict[str, str]) -> None:
        device = self.state["devices"][device_id]
        user_id = device["user"]
        device["owner_npub"] = statuses.get("npub", device.get("owner_npub", ""))
        device["owner_hex"] = statuses.get("public_key_hex", device.get("owner_hex", ""))
        device["device_npub"] = statuses.get("device_npub", device.get("device_npub", ""))
        device["device_hex"] = statuses.get("device_public_key_hex", device.get("device_hex", ""))
        if device.get("platform") == "ios":
            device["data_dir"] = self.ios_data_dir(device["udid"])
        elif device.get("platform") == "android":
            device["data_dir"] = statuses.get("data_dir", "/data/user/0/to.iris.chat.debug/files")
            device["app_package"] = statuses.get("app_package", ANDROID_APP_PACKAGE)
        self.state["users"][user_id] = {
            "npub": device["owner_npub"],
            "owner_hex": device["owner_hex"],
        }
        self.save_state()

    def primary_device_for_user(self, user_id: str) -> str:
        for device in self.config.get("devices", []):
            if device.get("user", device["id"]) == user_id and not device.get("linked_to"):
                return device["id"]
        raise SystemExit(f"No primary device configured for user {user_id}")

    def link_device(self, device: dict[str, Any]) -> None:
        device_id = device["id"]
        owner_user = device["linked_to"]
        owner_device_id = self.primary_device_for_user(owner_user)
        owner = self.state["users"].get(owner_user)
        if not owner:
            raise SystemExit(f"Cannot link {device_id}; owner user {owner_user} has no identity")
        status_file = self.work_dir / f"{device_id}-link.status"
        log_file = self.work_dir / f"{device_id}-link.log"
        status_file.unlink(missing_ok=True)
        with log_file.open("w", encoding="utf-8") as handle:
            command = self.harness_command(
                device_id,
                self.link_wait_action(device_id),
                args=self.link_wait_args(device_id, owner["npub"], status_file),
                reset=bool(device.get("reset", False)),
            )
            print("+ " + " ".join(shlex.quote(part) for part in command), flush=True)
            process = subprocess.Popen(
                command,
                cwd=str(ROOT_DIR),
                env=self.scenario_env(),
                stdout=handle,
                stderr=subprocess.STDOUT,
                text=True,
            )
        link_url = wait_for_status_file(status_file, self.link_status_key(device_id), int(device.get("link_timeout_secs", 180)))
        self.harness(
            owner_device_id,
            "add_authorized_device_from_args",
            args={
                "device_input": link_url,
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": str(device.get("relay_drain_timeout_secs", 240)),
            },
        )
        exit_code = process.wait(timeout=int(device.get("authorization_timeout_secs", 300)))
        output = log_file.read_text(encoding="utf-8", errors="replace")
        if exit_code != 0 or "INSTRUMENTATION_CODE: -1" not in output:
            print(output)
            raise SystemExit(f"Linked device authorization failed for {device_id}")
        self.record_identity(device_id, parse_status(output + "\n" + status_file.read_text(encoding="utf-8", errors="replace")))

    def harness_command(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
    ) -> list[str]:
        device = self.state["devices"][device_id]
        if device["platform"] == "ios":
            command = [
                sys.executable,
                str(IOS_HARNESS),
                "--udid",
                device["udid"],
                "--use-app-storage",
                "--run-id",
                device["run_id"],
                "--action",
                action,
            ]
            if reset:
                command.append("--reset")
        elif device["platform"] == "android":
            adb = str(self.adb())
            if reset:
                run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_APP_PACKAGE], env=self.scenario_env(), check=False)
                run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_TEST_PACKAGE], env=self.scenario_env(), check=False)
            command = [
                sys.executable,
                str(ANDROID_HARNESS),
                "--adb",
                adb,
                "--serial",
                device["serial"],
                "--runner",
                ANDROID_RUNNER,
                "--class-name",
                ANDROID_CLASS,
                "--test-name",
                action,
            ]
        else:
            raise SystemExit(f"Unsupported platform for harness command: {device['platform']}")
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        return command

    def link_wait_action(self, device_id: str) -> str:
        platform = self.state["devices"][device_id]["platform"]
        if platform == "ios":
            return "start_linked_device_wait_authorized_from_args"
        if platform == "android":
            return "start_link_invite_and_wait_for_authorization_from_args"
        raise SystemExit(f"Unsupported platform for link wait: {platform}")

    def link_wait_args(self, device_id: str, owner_input: str, status_file: Path) -> dict[str, str]:
        platform = self.state["devices"][device_id]["platform"]
        args = {"owner_input": owner_input, "status_file": str(status_file)}
        if platform == "android":
            args["authorization_state"] = "AUTHORIZED"
        return args

    def link_status_key(self, device_id: str) -> str:
        platform = self.state["devices"][device_id]["platform"]
        if platform == "android":
            return "invite_url"
        return "link_url"

    def setup_accounts(self) -> None:
        rebuild_next = True
        for device in self.config.get("devices", []):
            if device.get("linked_to"):
                continue
            self.create_account(device, rebuild=rebuild_next)
            rebuild_next = False
        for device in self.config.get("devices", []):
            if device.get("linked_to"):
                self.link_device(device)

    def resolve_member_input(self, value: str) -> str:
        if value in self.state.get("users", {}):
            return self.state["users"][value]["npub"]
        if value in self.state.get("devices", {}):
            return self.state["devices"][value]["owner_npub"]
        return value

    def devices_for_group(self, creator_id: str, member_values: list[str]) -> list[str]:
        users = {self.state["devices"][creator_id]["user"]}
        for member in member_values:
            if member in self.state["users"]:
                users.add(member)
            elif member in self.state["devices"]:
                users.add(self.state["devices"][member]["user"])
        return [
            device_id
            for device_id, device in self.state.get("devices", {}).items()
            if device.get("user") in users
        ]

    def create_groups(self) -> None:
        for group in self.config.get("groups", []):
            group_key = group["id"]
            creator = group["creator"]
            member_inputs = ",".join(self.resolve_member_input(member) for member in group.get("members", []))
            statuses = self.harness(
                creator,
                "create_group_from_args",
                args={
                    "group_name": group["name"],
                    "member_inputs": member_inputs,
                    "wait_for_relay_drain": "true",
                    "relay_drain_timeout_secs": str(group.get("relay_drain_timeout_secs", 240)),
                },
            )
            group_state = {
                "name": group["name"],
                "chat_id": statuses["chat_id"],
                "group_id": statuses["group_id"],
                "creator": creator,
            }
            self.state["groups"][group_key] = group_state
            self.save_state()
            if group.get("wait_for_members", True):
                for device_id in self.devices_for_group(creator, group.get("members", [])):
                    if device_id == creator:
                        continue
                    self.harness(
                        device_id,
                        "wait_for_group_chat_from_args",
                        args={"chat_id": group_state["chat_id"]},
                    )

    def open_apps(self) -> None:
        for device_id, device in self.state.get("devices", {}).items():
            if device.get("platform") == "ios":
                run(["xcrun", "simctl", "launch", device["udid"], "to.iris.chat"], capture=True, check=False)
            elif device.get("platform") == "android":
                run(
                    [
                        str(self.adb()),
                        "-s",
                        device["serial"],
                        "shell",
                        "monkey",
                        "-p",
                        ANDROID_APP_PACKAGE,
                        "-c",
                        "android.intent.category.LAUNCHER",
                        "1",
                    ],
                    capture=True,
                    check=False,
                )

    def setup(self) -> None:
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.start_relay()
        self.boot_ios()
        self.boot_android()
        self.build_ios()
        self.build_android()
        self.setup_accounts()
        self.create_groups()
        if self.config.get("open_apps", True):
            self.open_apps()
        self.save_state()
        print(f"Scenario ready. State: {self.state_path}")

    def begin_fault(self) -> None:
        self.stop_relay()
        drop_file = Path(self.relay_config()["drop_file"])
        drop_file.parent.mkdir(parents=True, exist_ok=True)
        drop_file.write_text("", encoding="utf-8")
        print(f"Relay stopped. Drop file cleared: {drop_file}")

    def inspect_pending(self, device_id: str, extra: list[str]) -> None:
        device = self.state.get("devices", {}).get(device_id)
        if not device:
            raise SystemExit(f"Unknown device `{device_id}` in state. Run `setup` first.")
        data_dir = self.pending_data_source(device_id)
        run([sys.executable, str(PENDING_PUBLISHES), "list", "--data-dir", data_dir, *extra], env=self.scenario_env())

    def drop_and_resume(self, sender_device: str, target_device: str, *, limit: int, pairwise_only: bool) -> None:
        sender = self.state.get("devices", {}).get(sender_device)
        target = self.state.get("devices", {}).get(target_device)
        if not sender:
            raise SystemExit(f"Unknown sender device `{sender_device}` in state. Run `setup` first.")
        if not target:
            raise SystemExit(f"Unknown target device `{target_device}` in state. Run `setup` first.")
        args = [
            sys.executable,
            str(PENDING_PUBLISHES),
            "write-drop-file",
            "--data-dir",
            self.pending_data_source(sender_device),
            "--target-owner-hex",
            target["owner_hex"],
            "--target-device-hex",
            target["device_hex"],
            "--limit",
            str(limit),
            "--drop-file",
            str(self.relay_config()["drop_file"]),
        ]
        if pairwise_only:
            args.insert(5, "--pairwise-only")
        run(args, env=self.scenario_env())
        self.start_relay()
        print(f"Relay restarted. Drop file: {self.relay_config()['drop_file']}")

    def pending_data_source(self, device_id: str) -> str:
        device = self.state["devices"][device_id]
        if device.get("platform") == "ios":
            data_dir = device.get("data_dir")
            if not data_dir:
                raise SystemExit(f"Device {device_id} has no data_dir in state")
            return data_dir
        if device.get("platform") == "android":
            destination = self.work_dir / f"{device_id}-core.sqlite3"
            with destination.open("wb") as handle:
                completed = subprocess.run(
                    [
                        str(self.adb()),
                        "-s",
                        device["serial"],
                        "exec-out",
                        "run-as",
                        ANDROID_APP_PACKAGE,
                        "cat",
                        "files/core.sqlite3",
                    ],
                    stdout=handle,
                    stderr=subprocess.PIPE,
                )
            if completed.returncode != 0:
                raise SystemExit(completed.stderr.decode("utf-8", errors="replace"))
            return str(destination)
        raise SystemExit(f"Unsupported platform for pending rows: {device.get('platform')}")

    def cleanup(self, *, shutdown_devices: bool) -> None:
        self.stop_relay()
        if shutdown_devices:
            for device in self.state.get("devices", {}).values():
                if device.get("platform") == "ios" and device.get("udid"):
                    run(["xcrun", "simctl", "shutdown", device["udid"]], capture=True, check=False)
                elif device.get("platform") == "android" and device.get("serial"):
                    run([str(self.adb()), "-s", device["serial"], "emu", "kill"], capture=True, check=False)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run deterministic mobile scenarios from JSON config.")
    parser.add_argument("--config", required=True, type=Path, help="Scenario JSON config.")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("setup", help="Start relay, boot devices, build, seed users/devices/groups.")
    sub.add_parser("begin-fault", help="Stop relay and clear the drop file before manual UI action.")
    inspect = sub.add_parser("inspect-pending", help="List pending relay publishes for a device.")
    inspect.add_argument("--device", required=True)
    inspect.add_argument("--pairwise-only", action="store_true")
    inspect.add_argument("--group-sender-outer-only", action="store_true")
    inspect.add_argument("--format", choices=("table", "json", "ids"), default="table")
    drop = sub.add_parser("drop-and-resume", help="Write a pending event to the drop file and restart relay.")
    drop.add_argument("--sender-device", required=True)
    drop.add_argument("--target-device", required=True)
    drop.add_argument("--limit", type=int, default=1)
    drop.set_defaults(pairwise_only=True)
    drop.add_argument(
        "--include-non-pairwise",
        action="store_false",
        dest="pairwise_only",
        help="Do not add --pairwise-only when selecting a pending event to drop.",
    )
    cleanup = sub.add_parser("cleanup", help="Stop relay and optionally shut down devices.")
    cleanup.add_argument("--shutdown-devices", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    scenario = Scenario(args.config)
    if args.command == "setup":
        scenario.setup()
    elif args.command == "begin-fault":
        scenario.begin_fault()
    elif args.command == "inspect-pending":
        extra = ["--format", args.format]
        if args.pairwise_only:
            extra.append("--pairwise-only")
        if args.group_sender_outer_only:
            extra.append("--group-sender-outer-only")
        scenario.inspect_pending(args.device, extra)
    elif args.command == "drop-and-resume":
        scenario.drop_and_resume(
            args.sender_device,
            args.target_device,
            limit=args.limit,
            pairwise_only=args.pairwise_only,
        )
    elif args.command == "cleanup":
        scenario.cleanup(shutdown_devices=args.shutdown_devices)
    else:
        raise SystemExit(f"Unknown command: {args.command}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
