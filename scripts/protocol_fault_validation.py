#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import json
import os
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable


ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / "scripts"
DEFAULT_CONFIG = SCRIPTS / "scenarios" / "alice_alice2_bob_group.json"
IOS_HARNESS = SCRIPTS / "run_ios_harness.py"
MOBILE_SCENARIO = SCRIPTS / "mobile_scenario.py"
PENDING_PUBLISHES = SCRIPTS / "pending_relay_publishes.py"
STATUS_PATTERN = re.compile(r"^INSTRUMENTATION_STATUS: ([^=]+)=(.*)$")
DEFAULT_CASES = [
    "sender_key_revision_repair",
    "sender_key_distribution_repair",
    "sender_key_distribution_repair_after_receiver_restart",
    "sender_key_distribution_repair_after_sender_restart",
    "sender_key_distribution_duplicate_replay_idempotent",
    "sender_key_distribution_multiple_messages",
    "sender_key_repair_after_receiver_restart",
    "sender_key_repair_after_sender_restart",
    "sender_key_duplicate_replay_idempotent",
    "sender_key_removed_member_repair_denied",
    "sender_key_late_member_post_add_repair",
    "sender_key_late_member_pre_add_denied",
    "group_metadata_drop_then_multiple_messages",
    "relay_offline_outbox_then_repair",
    "direct_group_offline_restart_recovery",
]

CAROL_PEER_CASES = {
    "sender_key_distribution_repair",
    "sender_key_distribution_repair_after_receiver_restart",
    "sender_key_distribution_repair_after_sender_restart",
    "sender_key_distribution_duplicate_replay_idempotent",
    "sender_key_distribution_multiple_messages",
    "sender_key_removed_member_repair_denied",
    "sender_key_late_member_post_add_repair",
    "sender_key_late_member_pre_add_denied",
}


from protocol_fault_common import CaseResult, ValidationFailure
from protocol_fault_cases import ProtocolFaultCasesMixin


TRUTHY_VALUES = {"1", "true", "yes", "on"}


def run(
    command: list[str],
    *,
    cwd: Path = ROOT,
    log_path: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
    output_parts: list[str] = []
    with subprocess.Popen(
        command,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    ) as process:
        log_handle = log_path.open("w", encoding="utf-8") if log_path is not None else None
        try:
            if log_handle is not None:
                log_handle.write("+ " + " ".join(command) + "\n")
                log_handle.flush()
            assert process.stdout is not None
            for line in process.stdout:
                output_parts.append(line)
                if log_handle is not None:
                    log_handle.write(line)
                    log_handle.flush()
            returncode = process.wait()
        finally:
            if log_handle is not None:
                log_handle.close()
    completed = subprocess.CompletedProcess(command, returncode, "".join(output_parts))
    if check and completed.returncode != 0:
        sys.stdout.write(completed.stdout)
        raise ValidationFailure(f"command failed ({completed.returncode}): {' '.join(command)}")
    return completed


def parse_status(output: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in output.splitlines():
        match = STATUS_PATTERN.match(line.strip())
        if match:
            key, value = match.groups()
            values[key.lower()] = value
    return values


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def scenario_work_dir(config: Path) -> Path:
    return Path(load_json(config)["work_dir"])


def case_stamp() -> str:
    return time.strftime("%H%M%S")


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


class ProtocolFaultValidation(ProtocolFaultCasesMixin):
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.stamp = time.strftime("%Y%m%dT%H%M%S")
        self.artifact_dir = (
            args.artifact_dir.resolve()
            if args.artifact_dir
            else Path(f"/tmp/iris-protocol-fault-validation-{self.stamp}")
        )
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        self.requested_cases = DEFAULT_CASES if args.all else list(args.case)
        self.config = self.prepare_config(args.config.resolve())
        self.work_dir = scenario_work_dir(self.config)
        self.state: dict[str, Any] = {}
        self.results: list[CaseResult] = []
        self.case_methods: dict[str, Callable[[Path], CaseResult]] = {
            "sender_key_revision_repair": self.case_sender_key_revision_repair,
            "sender_key_distribution_repair": self.case_sender_key_distribution_repair,
            "sender_key_repair_after_receiver_restart": self.case_sender_key_repair_after_receiver_restart,
            "sender_key_repair_after_sender_restart": self.case_sender_key_repair_after_sender_restart,
            "sender_key_duplicate_replay_idempotent": self.case_sender_key_duplicate_replay_idempotent,
            "group_metadata_drop_then_multiple_messages": self.case_group_metadata_drop_then_multiple_messages,
            "relay_offline_outbox_then_repair": self.case_relay_offline_outbox_then_repair,
            "direct_group_offline_restart_recovery": self.case_direct_group_offline_restart_recovery,
            "linked_owner_sender_key_repair": self.case_linked_owner_sender_key_repair,
            "sender_key_late_member_post_add_repair": self.case_sender_key_late_member_post_add_repair,
            "sender_key_removed_member_repair_denied": self.case_sender_key_removed_member_repair_denied,
            "sender_key_late_member_pre_add_denied": self.case_sender_key_late_member_pre_add_denied,
            "sender_key_distribution_repair_after_receiver_restart": (
                self.case_sender_key_distribution_repair_after_receiver_restart
            ),
            "sender_key_distribution_repair_after_sender_restart": (
                self.case_sender_key_distribution_repair_after_sender_restart
            ),
            "sender_key_distribution_duplicate_replay_idempotent": (
                self.case_sender_key_distribution_duplicate_replay_idempotent
            ),
            "sender_key_distribution_multiple_messages": self.case_sender_key_distribution_multiple_messages,
        }

    def prepare_config(self, source_config: Path) -> Path:
        if self.args.reuse_state:
            return source_config

        source = load_json(source_config)
        config = copy.deepcopy(source)
        config["name"] = f"{source.get('name', 'scenario')}-protocol-fault-{self.stamp}"
        config["work_dir"] = str(self.artifact_dir / "scenario")
        config.setdefault("relay", {})
        config["relay"]["label"] = f"iris.protocol-fault.{self.stamp}.relay"
        config["relay"]["port"] = free_tcp_port()
        config["relay"]["url"] = f"ws://127.0.0.1:{config['relay']['port']}"
        config["relay"]["drop_file"] = str(Path(config["work_dir"]) / "drop-events.txt")
        config["relay"]["log_file"] = str(Path(config["work_dir"]) / "relay.log")
        config["open_apps"] = True
        config.setdefault("ios", {})
        if self.args.skip_build:
            config["ios"]["build"] = False

        devices = config.setdefault("devices", [])
        needs_carol = any(name in CAROL_PEER_CASES for name in self.requested_cases)
        if needs_carol and not any(device.get("id") == "carol1" for device in devices):
            devices.append(
                {
                    "id": "carol1",
                    "platform": "ios",
                    "simulator": "Sender Key Carol",
                    "run_id": "carol",
                    "user": "carol",
                    "display_name": "Carol",
                    "reset": True,
                }
            )
        for device in devices:
            if device.get("platform") == "ios":
                device["reset"] = True
                if device.get("linked_to"):
                    device.setdefault("link_timeout_secs", 600)
                    device.setdefault("authorization_timeout_secs", 600)

        config_path = self.artifact_dir / "protocol-fault-validation-config.json"
        write_json(config_path, config)
        return config_path

    def setup(self) -> None:
        if not self.args.reuse_state:
            self.scenario_command(
                "setup",
                log_path=self.artifact_dir / "setup.log",
            )
        self.state = self.load_state()
        self.clear_drop_file()
        self.start_relay(self.artifact_dir / "start-relay.log")
        self.ensure_localhost_relays(self.artifact_dir / "ensure-localhost")
        self.prepare_extra_peers(self.artifact_dir / "prepare-peers")

    def load_state(self) -> dict[str, Any]:
        state_path = self.work_dir / "state.json"
        if not state_path.exists():
            raise ValidationFailure(f"scenario state not found at {state_path}")
        return load_json(state_path)

    def scenario_command(self, command: str, *extra: str, log_path: Path | None = None) -> None:
        run(
            [sys.executable, str(MOBILE_SCENARIO), "--config", str(self.config), command, *extra],
            log_path=log_path,
        )
        self.state = self.load_state() if (self.work_dir / "state.json").exists() else self.state

    def start_relay(self, log_path: Path) -> None:
        code = (
            "from pathlib import Path\n"
            "import sys\n"
            f"sys.path.insert(0, {str(SCRIPTS)!r})\n"
            "from mobile_scenario import Scenario\n"
            f"Scenario(Path({str(self.config)!r})).start_relay()\n"
        )
        run([sys.executable, "-c", code], log_path=log_path)

    def begin_fault(self, case_dir: Path) -> None:
        self.scenario_command("begin-fault", log_path=case_dir / "begin-fault.log")

    def clear_drop_file(self) -> None:
        drop_file = Path(self.state["relay"]["drop_file"])
        drop_file.parent.mkdir(parents=True, exist_ok=True)
        drop_file.write_text("", encoding="utf-8")

    def cleanup(self) -> None:
        if self.args.keep_devices_open:
            return
        run(
            [
                sys.executable,
                str(MOBILE_SCENARIO),
                "--config",
                str(self.config),
                "cleanup",
                "--shutdown-devices",
            ],
            log_path=self.artifact_dir / "cleanup.log",
            check=False,
        )

    def device(self, device_id: str) -> dict[str, Any]:
        device = self.state.get("devices", {}).get(device_id)
        if not device:
            raise ValidationFailure(f"scenario state is missing device `{device_id}`")
        return device

    def exclusive_ios_harness_enabled(self) -> bool:
        value = os.environ.get("IRIS_IOS_HARNESS_EXCLUSIVE_SIMULATOR")
        if value is not None:
            return value.strip().lower() in TRUTHY_VALUES
        try:
            ios_config = load_json(self.config).get("ios", {})
        except Exception:
            return False
        return str(ios_config.get("exclusive_harness_simulators", "")).strip().lower() in TRUTHY_VALUES

    def shutdown_other_ios_simulators(self, keep_device_id: str) -> None:
        if not self.exclusive_ios_harness_enabled():
            return
        for device_id, device in sorted(self.state.get("devices", {}).items()):
            if device_id == keep_device_id:
                continue
            if device.get("platform") != "ios" or not device.get("udid"):
                continue
            try:
                subprocess.run(
                    ["xcrun", "simctl", "shutdown", device["udid"]],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=20,
                )
            except subprocess.TimeoutExpired:
                print(
                    f"INSTRUMENTATION_RETRY: simctl shutdown timed out for {device_id}; "
                    "continuing exclusive iOS harness action",
                    flush=True,
                )

    def group(self, group_key: str = "alice-bob") -> dict[str, Any]:
        group = self.state.get("groups", {}).get(group_key)
        if not group:
            raise ValidationFailure(f"scenario state is missing group `{group_key}`")
        return group

    def harness(
        self,
        case_dir: Path,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        check: bool = True,
        suffix: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        device = self.device(device_id)
        if device["platform"] != "ios":
            raise ValidationFailure("protocol fault validation v1 supports iOS harness devices")
        self.shutdown_other_ios_simulators(device_id)
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
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        log_name = suffix or action
        completed = run(
            command,
            log_path=case_dir / f"{device_id}-{log_name}.log",
            check=False,
        )
        ok = completed.returncode == 0 and "INSTRUMENTATION_CODE: -1" in completed.stdout
        if check and not ok:
            sys.stdout.write(completed.stdout)
            raise ValidationFailure(f"harness action failed: {device_id} {action}")
        return completed

    def ensure_localhost_relays(self, log_dir: Path) -> None:
        log_dir.mkdir(parents=True, exist_ok=True)
        relay = self.state["relay"]
        relay_url = f"ws://127.0.0.1:{int(relay['port'])}"
        for device_id, device in sorted(self.state.get("devices", {}).items()):
            if device.get("platform") != "ios":
                continue
            self.harness(
                log_dir,
                device_id,
                "disable_relays_and_report",
                suffix="disable-existing-relays",
            )
            self.harness(
                log_dir,
                device_id,
                "add_relay_from_args",
                args={"relay_url": relay_url},
                suffix="add-localhost-relay",
            )
            self.harness(
                log_dir,
                device_id,
                "wait_for_connected_relay",
                args={"timeout_secs": "45"},
                suffix="wait-connected-localhost",
            )
            self.harness(
                log_dir,
                device_id,
                "report_logged_in_identity",
                args={"wait_for_relay_drain": "true", "relay_drain_timeout_secs": "180"},
                suffix="drain-after-localhost-relay",
            )

    def activate_connected(
        self,
        case_dir: Path,
        device_id: str,
        *,
        drain: bool = False,
        suffix: str = "activate",
    ) -> None:
        if drain:
            self.harness(
                case_dir,
                device_id,
                "report_logged_in_identity",
                args={"wait_for_relay_drain": "true", "relay_drain_timeout_secs": "180"},
                suffix=f"{suffix}-drain",
            )
        self.harness(
            case_dir,
            device_id,
            "wait_for_connected_relay",
            args={"timeout_secs": "45"},
            suffix=f"{suffix}-connected",
        )

    def device_ids_for_user(self, user: str) -> list[str]:
        return [
            device_id
            for device_id, device in sorted(self.state.get("devices", {}).items())
            if device.get("user") == user
        ]

    def activate_user_devices(
        self,
        case_dir: Path,
        user: str,
        *,
        drain: bool,
        suffix: str,
    ) -> None:
        for device_id in self.device_ids_for_user(user):
            device_suffix = suffix if device_id == "alice1" else f"{device_id}-{suffix}"
            self.activate_connected(case_dir, device_id, drain=drain, suffix=device_suffix)

    def prepare_extra_peers(self, log_dir: Path) -> None:
        if "carol1" not in self.state.get("devices", {}):
            return
        log_dir.mkdir(parents=True, exist_ok=True)
        marker = f"protocol-fault-peer-warmup-{self.stamp}"
        reply = f"protocol-fault-peer-warmup-reply-{self.stamp}"
        self.harness(
            log_dir,
            "alice1",
            "send_message_from_args",
            args={
                "peer_input": self.device("carol1")["owner_npub"],
                "message": marker,
                "wait_for_delivery": "true",
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": "180",
            },
            suffix="alice-warmup-carol-send",
        )
        self.harness(
            log_dir,
            "carol1",
            "wait_for_message_from_args",
            args={
                "peer_input": self.device("alice1")["owner_npub"],
                "message": marker,
                "direction": "incoming",
            },
            suffix="carol-warmup-alice-receive",
        )
        self.harness(
            log_dir,
            "carol1",
            "send_message_from_args",
            args={
                "peer_input": self.device("alice1")["owner_npub"],
                "message": reply,
                "wait_for_delivery": "true",
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": "180",
            },
            suffix="carol-warmup-alice-send",
        )
        self.harness(
            log_dir,
            "alice1",
            "wait_for_message_from_args",
            args={
                "peer_input": self.device("carol1")["owner_npub"],
                "message": reply,
                "direction": "incoming",
            },
            suffix="alice-warmup-carol-receive",
        )

    def restart_app(
        self,
        case_dir: Path,
        device_id: str,
        *,
        suffix: str = "restart",
        wait_connected: bool = True,
    ) -> None:
        device = self.device(device_id)
        if device["platform"] != "ios":
            raise ValidationFailure("protocol fault validation v1 supports iOS harness devices")
        run(
            ["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"],
            log_path=case_dir / f"{device_id}-{suffix}-terminate.log",
            check=False,
        )
        time.sleep(1)
        run(
            ["xcrun", "simctl", "launch", device["udid"], "to.iris.chat"],
            log_path=case_dir / f"{device_id}-{suffix}-launch.log",
            check=False,
        )
        if wait_connected:
            self.activate_connected(case_dir, device_id, drain=False, suffix=f"{suffix}-after-launch")

    def send_peer_message(
        self,
        case_dir: Path,
        device_id: str,
        peer_device_id: str,
        message: str,
        *,
        wait_for_delivery: bool = True,
        wait_for_relay_drain: bool = True,
        suffix: str | None = None,
    ) -> dict[str, str]:
        peer_input = self.device(peer_device_id)["owner_npub"]
        completed = self.harness(
            case_dir,
            device_id,
            "send_message_from_args",
            args={
                "peer_input": peer_input,
                "message": message,
                "wait_for_delivery": str(wait_for_delivery).lower(),
                "wait_for_relay_drain": str(wait_for_relay_drain).lower(),
                "relay_drain_timeout_secs": "180",
            },
            suffix=suffix or f"send-peer-{message}",
        )
        return parse_status(completed.stdout)

    def wait_peer_message(
        self,
        case_dir: Path,
        device_id: str,
        peer_device_id: str,
        message: str,
        *,
        direction: str = "incoming",
        suffix: str | None = None,
    ) -> dict[str, str]:
        peer_input = self.device(peer_device_id)["owner_npub"]
        completed = self.harness(
            case_dir,
            device_id,
            "wait_for_message_from_args",
            args={
                "peer_input": peer_input,
                "message": message,
                "direction": direction,
            },
            suffix=suffix or f"wait-peer-{message}",
        )
        return parse_status(completed.stdout)

    def send_message(
        self,
        case_dir: Path,
        device_id: str,
        chat_id: str,
        message: str,
        *,
        wait_for_delivery: bool = True,
        wait_for_relay_drain: bool = True,
        suffix: str | None = None,
    ) -> dict[str, str]:
        completed = self.harness(
            case_dir,
            device_id,
            "send_message_from_args",
            args={
                "chat_id": chat_id,
                "message": message,
                "wait_for_delivery": str(wait_for_delivery).lower(),
                "wait_for_relay_drain": str(wait_for_relay_drain).lower(),
                "relay_drain_timeout_secs": "180",
            },
            suffix=suffix or f"send-{message}",
        )
        return parse_status(completed.stdout)

    def wait_message(
        self,
        case_dir: Path,
        device_id: str,
        chat_id: str,
        message: str,
        *,
        direction: str = "incoming",
        check: bool = True,
        suffix: str | None = None,
    ) -> bool:
        completed = self.harness(
            case_dir,
            device_id,
            "wait_for_message_from_args",
            args={"chat_id": chat_id, "message": message, "direction": direction},
            check=check,
            suffix=suffix or f"wait-{message}",
        )
        return completed.returncode == 0 and "INSTRUMENTATION_CODE: -1" in completed.stdout

    def assert_message_absent(
        self,
        case_dir: Path,
        device_id: str,
        chat_id: str,
        message: str,
        *,
        timeout_ms: int = 20000,
        suffix: str | None = None,
    ) -> None:
        self.harness(
            case_dir,
            device_id,
            "assert_message_absent_from_args",
            args={"chat_id": chat_id, "message": message, "timeout_ms": str(timeout_ms)},
            suffix=suffix or f"absent-{message}",
        )

    def create_group(
        self,
        case_dir: Path,
        name: str,
        member_device_ids: list[str],
    ) -> dict[str, str]:
        member_inputs = [self.device(device_id)["owner_npub"] for device_id in member_device_ids]
        completed = self.harness(
            case_dir,
            "alice1",
            "create_group_from_args",
            args={
                "group_name": name,
                "member_inputs": "|".join(member_inputs),
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": "180",
            },
            suffix=f"create-group-{name}",
        )
        status = parse_status(completed.stdout)
        chat_id = status["chat_id"]
        for device_id in member_device_ids:
            self.harness(
                case_dir,
                device_id,
                "wait_for_group_chat_from_args",
                args={"chat_id": chat_id},
                suffix=f"wait-group-{name}",
            )
        return status

    def update_group_name(
        self,
        case_dir: Path,
        group_id: str,
        name: str,
        *,
        wait_for_relay_drain: bool,
        suffix: str = "update-group-name",
    ) -> None:
        self.harness(
            case_dir,
            "alice1",
            "update_group_name_from_args",
            args={
                "group_id": group_id,
                "group_name": name,
                "wait_for_relay_drain": str(wait_for_relay_drain).lower(),
                "relay_drain_timeout_secs": "180",
            },
            suffix=suffix,
        )

    def add_group_member(
        self,
        case_dir: Path,
        group_id: str,
        chat_id: str,
        member_device_id: str,
        *,
        expected_member_count: int,
        wait_for_relay_drain: bool,
    ) -> None:
        self.harness(
            case_dir,
            "alice1",
            "add_group_members_from_args",
            args={
                "group_id": group_id,
                "chat_id": chat_id,
                "member_inputs": self.device(member_device_id)["owner_npub"],
                "expected_member_count": str(expected_member_count),
                "wait_for_relay_drain": str(wait_for_relay_drain).lower(),
                "relay_drain_timeout_secs": "180",
            },
            suffix=f"add-{member_device_id}",
        )

    def remove_group_member(
        self,
        case_dir: Path,
        group_id: str,
        chat_id: str,
        member_device_id: str,
        *,
        expected_member_count: int,
        wait_for_relay_drain: bool,
    ) -> None:
        self.harness(
            case_dir,
            "alice1",
            "remove_group_member_from_args",
            args={
                "group_id": group_id,
                "chat_id": chat_id,
                "member_input": self.device(member_device_id)["owner_hex"],
                "expected_member_count": str(expected_member_count),
                "wait_for_relay_drain": str(wait_for_relay_drain).lower(),
                "relay_drain_timeout_secs": "180",
            },
            suffix=f"remove-{member_device_id}",
        )

    def wait_group_name(
        self,
        case_dir: Path,
        device_id: str,
        chat_id: str,
        name: str,
        suffix: str | None = None,
    ) -> None:
        self.harness(
            case_dir,
            device_id,
            "wait_for_group_name_from_args",
            args={"chat_id": chat_id, "group_name": name},
            suffix=suffix or f"wait-name-{name}",
        )

    def wait_member_count(
        self,
        case_dir: Path,
        device_id: str,
        chat_id: str,
        member_count: int,
        suffix: str | None = None,
    ) -> None:
        self.harness(
            case_dir,
            device_id,
            "wait_for_group_member_count_from_args",
            args={"chat_id": chat_id, "member_count": str(member_count)},
            suffix=suffix or f"wait-members-{member_count}",
        )

    def report_protocol_debug(self, case_dir: Path, device_id: str, suffix: str) -> dict[str, Any]:
        self.harness(case_dir, device_id, "report_runtime_debug_snapshot", suffix=suffix)
        debug_path = Path(self.device(device_id)["data_dir"]) / "iris_chat_runtime_debug.json"
        if not debug_path.exists():
            return {}
        try:
            debug = load_json(debug_path)
        except json.JSONDecodeError:
            return {}
        protocol = debug.get("protocol_engine")
        return protocol if isinstance(protocol, dict) else {}

    def pending_repair_count(self, debug: dict[str, Any]) -> int:
        return int(debug.get("pending_group_sender_key_repair_count") or 0)

    def pending_rows(
        self,
        case_dir: Path,
        source_device_id: str,
        *,
        peer_device_id: str | None = None,
        pairwise_only: bool = False,
        group_sender_outer_only: bool = False,
        suffix: str = "pending",
    ) -> list[dict[str, Any]]:
        command = [
            sys.executable,
            str(PENDING_PUBLISHES),
            "list",
            "--data-dir",
            self.device(source_device_id)["data_dir"],
            "--format",
            "json",
        ]
        if peer_device_id:
            command.extend(["--chat-id", self.device(peer_device_id)["owner_hex"]])
        if pairwise_only:
            command.append("--pairwise-only")
        if group_sender_outer_only:
            command.append("--group-sender-outer-only")
        completed = run(command, log_path=case_dir / f"{source_device_id}-{suffix}.log")
        path = case_dir / f"{source_device_id}-{suffix}.json"
        path.write_text(completed.stdout, encoding="utf-8")
        return json.loads(completed.stdout)

    def select_pending_row(
        self,
        rows: list[dict[str, Any]],
        *,
        selector: str,
        purpose: str,
    ) -> dict[str, Any]:
        if not rows:
            raise ValidationFailure(f"no pending row available for {purpose}")
        ordered = sorted(rows, key=lambda row: (row.get("created_at_secs") or 0, row["event_id"]))
        if selector == "oldest":
            return ordered[0]
        if selector == "newest":
            return ordered[-1]
        raise ValidationFailure(f"unknown pending row selector `{selector}`")

    def select_sender_key_distribution_row(
        self,
        rows: list[dict[str, Any]],
        *,
        purpose: str,
    ) -> dict[str, Any]:
        if not rows:
            raise ValidationFailure(f"no pending row available for {purpose}")
        return sorted(
            rows,
            key=lambda row: (
                len(str(row.get("event", {}).get("content", ""))),
                row.get("created_at_secs") or 0,
                row["event_id"],
            ),
        )[0]

    def write_drop_file(self, event_id: str) -> None:
        drop_file = Path(self.state["relay"]["drop_file"])
        drop_file.parent.mkdir(parents=True, exist_ok=True)
        drop_file.write_text(event_id + "\n", encoding="utf-8")

    def copy_artifacts(self, case_dir: Path) -> None:
        case_dir.mkdir(parents=True, exist_ok=True)
        for source, name in [
            (self.work_dir / "state.json", "scenario-state.json"),
            (Path(self.state["relay"]["drop_file"]), "drop-events.txt"),
            (Path(self.state["relay"]["log_file"]), "relay.log"),
        ]:
            if source.exists():
                shutil.copyfile(source, case_dir / name)
        for device_id, device in sorted(self.state.get("devices", {}).items()):
            data_dir = device.get("data_dir")
            if not data_dir:
                continue
            debug_path = Path(data_dir) / "iris_chat_runtime_debug.json"
            if debug_path.exists():
                shutil.copyfile(debug_path, case_dir / f"{device_id}-runtime-debug.json")

    def visible_message_count(self, device_id: str, chat_id: str, message: str) -> int:
        data_dir = Path(self.device(device_id)["data_dir"])
        sqlite_path = data_dir / "core.sqlite3"
        if sqlite_path.exists():
            conn = sqlite3.connect(sqlite_path)
            try:
                return int(
                    conn.execute(
                        """
                        SELECT COUNT(*)
                        FROM messages
                        WHERE lower(chat_id) = lower(?) AND body = ?
                        """,
                        (chat_id, message),
                    ).fetchone()[0]
                )
            finally:
                conn.close()

        threads_dir = data_dir / "core" / "threads"
        count = 0
        if not threads_dir.exists():
            return 0
        for path in threads_dir.glob("*.json"):
            try:
                thread = load_json(path)
            except Exception:
                continue
            if str(thread.get("chat_id", "")).lower() != chat_id.lower():
                continue
            for entry in thread.get("messages", []):
                if isinstance(entry, dict) and entry.get("body") == message:
                    count += 1
        return count

    def base_group(self) -> tuple[str, str]:
        group = self.group("alice-bob")
        return group["chat_id"], group["group_id"]

    def case_direct_group_offline_restart_recovery(self, case_dir: Path) -> CaseResult:
        chat_id, group_id = self.base_group()
        stamp = case_stamp()
        alice_to_bob_warmup = f"offline-restart-a2b-warmup-{stamp}"
        bob_to_alice_warmup = f"offline-restart-b2a-warmup-{stamp}"
        group_warmup = f"offline-restart-group-warmup-{stamp}"
        alice_to_bob = f"offline-restart-a2b-{stamp}"
        bob_to_alice = f"offline-restart-b2a-{stamp}"
        alice_group = f"offline-restart-alice-group-{stamp}"
        bob_group = f"offline-restart-bob-group-{stamp}"

        self.send_peer_message(
            case_dir,
            "alice1",
            "bob1",
            alice_to_bob_warmup,
            suffix="warmup-alice-direct-send",
        )
        self.wait_peer_message(
            case_dir,
            "bob1",
            "alice1",
            alice_to_bob_warmup,
            suffix="warmup-bob-direct-wait",
        )
        self.send_peer_message(
            case_dir,
            "bob1",
            "alice1",
            bob_to_alice_warmup,
            suffix="warmup-bob-direct-send",
        )
        self.wait_peer_message(
            case_dir,
            "alice1",
            "bob1",
            bob_to_alice_warmup,
            suffix="warmup-alice-direct-wait",
        )
        self.wait_peer_message(
            case_dir,
            "alice2",
            "bob1",
            bob_to_alice_warmup,
            suffix="warmup-alice2-direct-self-sync",
        )
        self.send_message(case_dir, "alice1", chat_id, group_warmup, suffix="warmup-group-send")
        self.wait_message(case_dir, "bob1", chat_id, group_warmup, suffix="warmup-group-bob-wait")
        self.wait_message(
            case_dir,
            "alice2",
            chat_id,
            group_warmup,
            direction="outgoing",
            suffix="warmup-group-alice2-self-sync",
        )

        self.begin_fault(case_dir)
        alice_to_bob_send = self.send_peer_message(
            case_dir,
            "alice1",
            "bob1",
            alice_to_bob,
            wait_for_delivery=False,
            wait_for_relay_drain=False,
            suffix="offline-alice-direct-send",
        )
        bob_to_alice_send = self.send_peer_message(
            case_dir,
            "bob1",
            "alice1",
            bob_to_alice,
            wait_for_delivery=False,
            wait_for_relay_drain=False,
            suffix="offline-bob-direct-send",
        )
        alice_group_send = self.send_message(
            case_dir,
            "alice1",
            chat_id,
            alice_group,
            wait_for_delivery=False,
            wait_for_relay_drain=False,
            suffix="offline-alice-group-send",
        )
        bob_group_send = self.send_message(
            case_dir,
            "bob1",
            chat_id,
            bob_group,
            wait_for_delivery=False,
            wait_for_relay_drain=False,
            suffix="offline-bob-group-send",
        )

        for device_id in ("alice1", "alice2", "bob1"):
            self.restart_app(
                case_dir,
                device_id,
                suffix=f"{device_id}-offline-restart",
                wait_connected=False,
            )

        self.start_relay(case_dir / "restart-relay-after-offline-sends.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-offline")
        self.activate_connected(case_dir, "bob1", drain=True, suffix="bob-after-offline")
        self.activate_connected(case_dir, "alice2", drain=True, suffix="alice2-after-offline")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-final-drain")
        self.activate_connected(case_dir, "bob1", drain=True, suffix="bob-final-drain")

        bob_direct = self.wait_peer_message(
            case_dir,
            "bob1",
            "alice1",
            alice_to_bob,
            direction="incoming",
            suffix="bob-final-direct-a2b",
        )
        alice_direct = self.wait_peer_message(
            case_dir,
            "alice1",
            "bob1",
            bob_to_alice,
            direction="incoming",
            suffix="alice-final-direct-b2a",
        )
        alice2_direct = self.wait_peer_message(
            case_dir,
            "alice2",
            "bob1",
            bob_to_alice,
            direction="incoming",
            suffix="alice2-final-direct-b2a",
        )
        self.wait_message(
            case_dir,
            "bob1",
            chat_id,
            alice_group,
            direction="incoming",
            suffix="bob-final-group-from-alice",
        )
        self.wait_message(
            case_dir,
            "alice2",
            chat_id,
            alice_group,
            direction="outgoing",
            suffix="alice2-final-group-self-sync",
        )
        self.wait_message(
            case_dir,
            "alice1",
            chat_id,
            bob_group,
            direction="incoming",
            suffix="alice-final-group-from-bob",
        )
        self.wait_message(
            case_dir,
            "alice2",
            chat_id,
            bob_group,
            direction="incoming",
            suffix="alice2-final-group-from-bob",
        )

        duplicate_checks = {
            "bob_direct_a2b": (
                "bob1",
                bob_direct["chat_id"],
                alice_to_bob,
            ),
            "alice_direct_b2a": (
                "alice1",
                alice_direct["chat_id"],
                bob_to_alice,
            ),
            "alice2_direct_b2a": (
                "alice2",
                alice2_direct["chat_id"],
                bob_to_alice,
            ),
            "bob_group_from_alice": ("bob1", chat_id, alice_group),
            "alice2_group_from_alice": ("alice2", chat_id, alice_group),
            "alice_group_from_bob": ("alice1", chat_id, bob_group),
            "alice2_group_from_bob": ("alice2", chat_id, bob_group),
        }
        duplicate_counts: dict[str, int] = {}
        for label, (device_id, target_chat_id, message) in duplicate_checks.items():
            count = self.visible_message_count(device_id, target_chat_id, message)
            duplicate_counts[label] = count
            if count != 1:
                raise ValidationFailure(
                    f"expected {device_id} to have exactly one `{message}` in {target_chat_id}, found {count}"
                )

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=True,
            visible_result_ok=True,
            final_pending_repair_count=0,
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "offline_messages": {
                    "alice_to_bob": alice_to_bob,
                    "bob_to_alice": bob_to_alice,
                    "alice_group": alice_group,
                    "bob_group": bob_group,
                },
                "queued_delivery": {
                    "alice_to_bob": alice_to_bob_send.get("delivery", ""),
                    "bob_to_alice": bob_to_alice_send.get("delivery", ""),
                    "alice_group": alice_group_send.get("delivery", ""),
                    "bob_group": bob_group_send.get("delivery", ""),
                },
                "duplicate_counts": duplicate_counts,
            },
        )

    def case_linked_owner_sender_key_repair(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(
            case_dir,
            label="linked-owner",
            linked_owner_assertion=True,
        )

    def run_case(self, name: str) -> CaseResult:
        if name not in self.case_methods:
            raise ValidationFailure(f"unknown case `{name}`")
        case_dir = self.artifact_dir / "cases" / name
        case_dir.mkdir(parents=True, exist_ok=True)
        print(f"=== protocol fault case: {name} ===", flush=True)
        try:
            self.clear_drop_file()
            result = self.case_methods[name](case_dir)
            result.case = name
            result.artifact_dir = str(case_dir)
            self.copy_artifacts(case_dir)
            write_json(case_dir / "summary.json", result.to_json())
            self.results.append(result)
            return result
        except Exception as error:
            self.copy_artifacts(case_dir)
            result = CaseResult(
                case=name,
                status="failed",
                artifact_dir=str(case_dir),
                error=str(error),
            )
            write_json(case_dir / "summary.json", result.to_json())
            self.results.append(result)
            raise

    def run(self, cases: list[str]) -> int:
        try:
            self.setup()
            for name in cases:
                result = self.run_case(name)
                print(json.dumps(result.to_json(), indent=2, sort_keys=True), flush=True)
        finally:
            aggregate = {
                "artifact_dir": str(self.artifact_dir),
                "config": str(self.config),
                "cases": [result.to_json() for result in self.results],
                "passed": bool(self.results) and all(result.status == "passed" for result in self.results),
            }
            write_json(self.artifact_dir / "protocol-fault-validation-summary.json", aggregate)
            if not self.results or any(result.status == "failed" for result in self.results):
                print(f"protocol_fault_validation_artifacts={self.artifact_dir}", flush=True)
            self.cleanup()
        return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run app-shell protocol fault validation cases.")
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    parser.add_argument("--all", action="store_true", help="Run all default app-shell fault cases.")
    parser.add_argument("--case", action="append", default=[], help="Run one named case. Repeatable.")
    parser.add_argument("--list", action="store_true", help="List available cases.")
    parser.add_argument("--reuse-state", action="store_true", help="Use existing scenario state instead of fresh setup.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse existing app build during fresh setup.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulators/emulators open after the run.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for logs, summaries, and generated scenario config.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    runner = ProtocolFaultValidation(args)
    available = list(runner.case_methods.keys())
    if args.list:
        for name in available:
            print(name)
        return 0
    if args.all:
        cases = DEFAULT_CASES
    else:
        cases = args.case
    if not cases:
        raise SystemExit("select --all, --case NAME, or --list")
    unknown = [name for name in cases if name not in runner.case_methods]
    if unknown:
        raise SystemExit(f"unknown case(s): {', '.join(unknown)}")
    return runner.run(cases)


if __name__ == "__main__":
    raise SystemExit(main())
