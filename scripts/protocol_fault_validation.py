#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import json
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
]


class ValidationFailure(Exception):
    pass


@dataclass
class CaseResult:
    case: str
    status: str
    fault_injected: bool = False
    repair_observed: bool = False
    visible_result_ok: bool = False
    final_pending_repair_count: int = 0
    artifact_dir: str = ""
    dropped_event_id: str = ""
    details: dict[str, Any] = field(default_factory=dict)
    error: str = ""

    def to_json(self) -> dict[str, Any]:
        result = {
            "case": self.case,
            "status": self.status,
            "fault_injected": self.fault_injected,
            "repair_observed": self.repair_observed,
            "visible_result_ok": self.visible_result_ok,
            "final_pending_repair_count": self.final_pending_repair_count,
            "artifact_dir": self.artifact_dir,
        }
        if self.dropped_event_id:
            result["dropped_event_id"] = self.dropped_event_id
        if self.details:
            result["details"] = self.details
        if self.error:
            result["error"] = self.error
        return result


def run(
    command: list[str],
    *,
    cwd: Path = ROOT,
    log_path: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text("+ " + " ".join(command) + "\n" + completed.stdout, encoding="utf-8")
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


class ProtocolFaultValidation:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.stamp = time.strftime("%Y%m%dT%H%M%S")
        self.artifact_dir = (
            args.artifact_dir.resolve()
            if args.artifact_dir
            else Path(f"/tmp/iris-protocol-fault-validation-{self.stamp}")
        )
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
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
        config["relay"]["drop_file"] = str(Path(config["work_dir"]) / "drop-events.txt")
        config["relay"]["log_file"] = str(Path(config["work_dir"]) / "relay.log")
        config["open_apps"] = True
        config.setdefault("ios", {})
        if self.args.skip_build:
            config["ios"]["build"] = False

        devices = config.setdefault("devices", [])
        if not any(device.get("id") == "carol1" for device in devices):
            devices.append(
                {
                    "id": "carol1",
                    "platform": "ios",
                    "simulator": "Group Hardening Carol",
                    "run_id": "carol",
                    "user": "carol",
                    "display_name": "Carol",
                    "reset": True,
                }
            )
        for device in devices:
            if device.get("platform") == "ios":
                device["reset"] = True

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

    def restart_app(self, case_dir: Path, device_id: str, *, suffix: str = "restart") -> None:
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
        self.activate_connected(case_dir, device_id, drain=False, suffix=f"{suffix}-after-launch")

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
        target_device_id: str | None = None,
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
        if target_device_id:
            target = self.device(target_device_id)
            command.extend(
                [
                    "--target-owner-hex",
                    target["owner_hex"],
                    "--target-device-hex",
                    target["device_hex"],
                ]
            )
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

    def run_revision_repair_flow(
        self,
        case_dir: Path,
        *,
        label: str,
        message_count: int = 1,
        offline_messages: bool = False,
        receiver_restart: bool = False,
        sender_restart: bool = False,
        linked_owner_assertion: bool = False,
    ) -> CaseResult:
        chat_id, group_id = self.base_group()
        stamp = case_stamp()
        baseline = f"{label}-baseline-{stamp}"
        new_name = f"{label} Revision {stamp}"
        messages = [f"{label}-message-{stamp}-{index}" for index in range(1, message_count + 1)]

        self.send_message(case_dir, "alice1", chat_id, baseline, suffix="baseline-send")
        self.wait_message(case_dir, "bob1", chat_id, baseline, suffix="baseline-wait-bob")

        self.begin_fault(case_dir)
        self.update_group_name(
            case_dir,
            group_id,
            new_name,
            wait_for_relay_drain=False,
            suffix="offline-rename",
        )
        if offline_messages:
            for message in messages:
                self.send_message(
                    case_dir,
                    "alice1",
                    chat_id,
                    message,
                    wait_for_delivery=False,
                    wait_for_relay_drain=False,
                    suffix=f"offline-send-{message}",
                )

        rows = self.pending_rows(
            case_dir,
            "alice1",
            target_device_id="bob1",
            pairwise_only=True,
            suffix="bob-pending-before-drop",
        )
        drop_row = self.select_pending_row(rows, selector="newest", purpose="Bob metadata revision")
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-drop")
        self.wait_group_name(case_dir, "alice2", chat_id, new_name, suffix="alice2-renamed")

        if not offline_messages:
            for message in messages:
                self.send_message(case_dir, "alice1", chat_id, message, suffix=f"send-{message}")

        passive_ok = self.wait_message(
            case_dir,
            "bob1",
            chat_id,
            messages[-1],
            check=False,
            suffix="bob-passive-wait-last-message",
        )
        passive_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-passive")
        passive_pending = self.pending_repair_count(passive_debug)
        if not passive_ok and passive_pending == 0:
            raise ValidationFailure("Bob missed the message but did not record pending sender-key repair state")

        if receiver_restart:
            self.restart_app(case_dir, "bob1", suffix="bob-restart-before-repair")
            restarted = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-restart")
            if self.pending_repair_count(restarted) < passive_pending:
                raise ValidationFailure("receiver restart lost pending sender-key repair state")

        if sender_restart:
            self.restart_app(case_dir, "alice1", suffix="alice-restart-before-repair")
            self.report_protocol_debug(case_dir, "alice1", "alice-debug-after-restart")

        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-force")
        self.activate_connected(case_dir, "bob1", drain=False, suffix="bob-force")
        time.sleep(3)
        for message in messages:
            self.wait_message(case_dir, "bob1", chat_id, message, suffix=f"bob-final-wait-{message}")
        self.wait_group_name(case_dir, "bob1", chat_id, new_name, suffix="bob-final-name")
        if linked_owner_assertion:
            self.wait_group_name(case_dir, "alice2", chat_id, new_name, suffix="alice2-final-name")
            for message in messages:
                self.wait_message(
                    case_dir,
                    "alice2",
                    chat_id,
                    message,
                    direction="incoming",
                    suffix=f"alice2-final-wait-{message}",
                )

        final_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-final")
        final_pending = self.pending_repair_count(final_debug)
        for message in messages:
            count = self.visible_message_count("bob1", chat_id, message)
            if count != 1:
                raise ValidationFailure(f"expected Bob to have exactly one `{message}`, found {count}")

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=passive_pending > 0 or passive_ok,
            visible_result_ok=True,
            final_pending_repair_count=final_pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "new_name": new_name,
                "messages": messages,
                "passive_success": passive_ok,
                "passive_pending_repair_count": passive_pending,
            },
        )

    def run_distribution_repair_flow(
        self,
        case_dir: Path,
        *,
        label: str,
        message_count: int = 1,
        receiver_restart: bool = False,
        sender_restart: bool = False,
    ) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"{label} Group {stamp}", ["bob1", "carol1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        baseline = f"{label}-baseline-{stamp}"
        messages = [f"{label}-after-rotation-{stamp}-{index}" for index in range(1, message_count + 1)]

        self.send_message(case_dir, "alice1", chat_id, baseline, suffix="baseline-send")
        self.wait_message(case_dir, "bob1", chat_id, baseline, suffix="baseline-wait-bob")
        self.wait_message(case_dir, "carol1", chat_id, baseline, suffix="baseline-wait-carol")

        self.begin_fault(case_dir)
        self.remove_group_member(
            case_dir,
            group_id,
            chat_id,
            "carol1",
            expected_member_count=2,
            wait_for_relay_drain=False,
        )
        rows = self.pending_rows(
            case_dir,
            "alice1",
            target_device_id="bob1",
            pairwise_only=True,
            suffix="bob-pending-after-remove",
        )
        drop_row = self.select_sender_key_distribution_row(
            rows,
            purpose="Bob rotated sender-key distribution",
        )
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-distribution-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-remove")
        self.wait_member_count(case_dir, "bob1", chat_id, 2, suffix="bob-sees-removal")

        for message in messages:
            self.send_message(case_dir, "alice1", chat_id, message, suffix=f"send-after-rotation-{message}")
        passive_ok = self.wait_message(
            case_dir,
            "bob1",
            chat_id,
            messages[-1],
            check=False,
            suffix="bob-passive-wait-rotated-message",
        )
        passive_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-passive")
        passive_pending = self.pending_repair_count(passive_debug)
        if not passive_ok and passive_pending == 0:
            raise ValidationFailure("Bob missed the rotated-key message but did not record pending sender-key repair state")

        if receiver_restart:
            self.restart_app(case_dir, "bob1", suffix="bob-restart-before-distribution-repair")
            restarted = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-restart")
            if self.pending_repair_count(restarted) < passive_pending:
                raise ValidationFailure("receiver restart lost pending sender-key distribution repair")

        if sender_restart:
            self.restart_app(case_dir, "alice1", suffix="alice-restart-before-distribution-repair")
            self.report_protocol_debug(case_dir, "alice1", "alice-debug-after-restart")

        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-force")
        self.activate_connected(case_dir, "bob1", drain=False, suffix="bob-force")
        for message in messages:
            self.wait_message(case_dir, "bob1", chat_id, message, suffix=f"bob-final-message-{message}")
            self.assert_message_absent(case_dir, "carol1", chat_id, message, suffix=f"carol-removed-absent-{message}")
        final_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-final")
        final_pending = self.pending_repair_count(final_debug)
        for message in messages:
            count = self.visible_message_count("bob1", chat_id, message)
            if count != 1:
                raise ValidationFailure(f"expected Bob to have exactly one `{message}`, found {count}")

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=passive_pending > 0 or passive_ok,
            visible_result_ok=True,
            final_pending_repair_count=final_pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "messages": messages,
                "passive_success": passive_ok,
                "passive_pending_repair_count": passive_pending,
            },
        )

    def case_sender_key_revision_repair(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="revision-repair")

    def case_sender_key_repair_after_receiver_restart(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="receiver-restart", receiver_restart=True)

    def case_sender_key_repair_after_sender_restart(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="sender-restart", sender_restart=True)

    def case_sender_key_duplicate_replay_idempotent(self, case_dir: Path) -> CaseResult:
        result = self.run_revision_repair_flow(case_dir, label="duplicate-replay")
        chat_id = result.details["group_chat_id"]
        message = result.details["messages"][0]
        self.wait_message(case_dir, "bob1", chat_id, message, suffix="bob-rewait-message")
        self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-rewait")
        count = self.visible_message_count("bob1", chat_id, message)
        if count != 1:
            raise ValidationFailure(f"duplicate replay should leave one message, found {count}")
        result.details["bob_message_count_after_rewait"] = count
        return result

    def case_group_metadata_drop_then_multiple_messages(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="multi-message-revision", message_count=3)

    def case_relay_offline_outbox_then_repair(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(
            case_dir,
            label="offline-outbox",
            message_count=2,
            offline_messages=True,
        )

    def case_linked_owner_sender_key_repair(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(
            case_dir,
            label="linked-owner",
            linked_owner_assertion=True,
        )

    def case_sender_key_distribution_repair(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(case_dir, label="distribution-repair")

    def case_sender_key_distribution_repair_after_receiver_restart(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(
            case_dir,
            label="distribution-receiver-restart",
            receiver_restart=True,
        )

    def case_sender_key_distribution_repair_after_sender_restart(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(
            case_dir,
            label="distribution-sender-restart",
            sender_restart=True,
        )

    def case_sender_key_distribution_duplicate_replay_idempotent(self, case_dir: Path) -> CaseResult:
        result = self.run_distribution_repair_flow(case_dir, label="distribution-duplicate")
        chat_id = result.details["group_chat_id"]
        message = result.details["messages"][0]
        self.wait_message(case_dir, "bob1", chat_id, message, suffix="bob-rewait-distribution-message")
        self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-distribution-rewait")
        count = self.visible_message_count("bob1", chat_id, message)
        if count != 1:
            raise ValidationFailure(f"duplicate distribution repair replay should leave one message, found {count}")
        result.details["bob_message_count_after_rewait"] = count
        return result

    def case_sender_key_distribution_multiple_messages(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(
            case_dir,
            label="distribution-multi-message",
            message_count=3,
        )

    def case_sender_key_late_member_post_add_repair(self, case_dir: Path) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"Late Add Repair {stamp}", ["bob1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        message = f"late-member-post-add-rotation-{stamp}"

        self.add_group_member(
            case_dir,
            group_id,
            chat_id,
            "carol1",
            expected_member_count=3,
            wait_for_relay_drain=True,
        )
        self.wait_member_count(case_dir, "carol1", chat_id, 3, suffix="carol-sees-initial-add")

        self.begin_fault(case_dir)
        self.remove_group_member(
            case_dir,
            group_id,
            chat_id,
            "bob1",
            expected_member_count=2,
            wait_for_relay_drain=False,
        )
        rows = self.pending_rows(
            case_dir,
            "alice1",
            target_device_id="carol1",
            pairwise_only=True,
            suffix="carol-pending-after-post-add-rotation",
        )
        drop_row = self.select_sender_key_distribution_row(
            rows,
            purpose="Carol post-add rotated sender-key distribution",
        )
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-post-add-rotation-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-post-add-rotation")
        self.wait_member_count(case_dir, "carol1", chat_id, 2, suffix="carol-sees-post-add-rotation")

        self.send_message(case_dir, "alice1", chat_id, message, suffix="send-post-add-rotation")
        passive_ok = self.wait_message(
            case_dir,
            "carol1",
            chat_id,
            message,
            check=False,
            suffix="carol-passive-wait-post-add",
        )
        passive_debug = self.report_protocol_debug(case_dir, "carol1", "carol-debug-after-passive")
        passive_pending = self.pending_repair_count(passive_debug)
        if not passive_ok and passive_pending == 0:
            raise ValidationFailure("Carol missed the post-add message but did not record pending sender-key repair state")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-force")
        self.activate_connected(case_dir, "carol1", drain=False, suffix="carol-force")
        self.wait_message(case_dir, "carol1", chat_id, message, suffix="carol-final-message")
        final_debug = self.report_protocol_debug(case_dir, "carol1", "carol-debug-final")
        final_pending = self.pending_repair_count(final_debug)

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=passive_pending > 0 or passive_ok,
            visible_result_ok=True,
            final_pending_repair_count=final_pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "message": message,
                "dropped_state": "post_add_rotation_distribution",
                "passive_success": passive_ok,
                "passive_pending_repair_count": passive_pending,
            },
        )

    def case_sender_key_removed_member_repair_denied(self, case_dir: Path) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"Removed Repair Denied {stamp}", ["bob1", "carol1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        message = f"removed-member-future-{stamp}"

        self.begin_fault(case_dir)
        self.remove_group_member(
            case_dir,
            group_id,
            chat_id,
            "bob1",
            expected_member_count=2,
            wait_for_relay_drain=False,
        )
        rows = self.pending_rows(
            case_dir,
            "alice1",
            target_device_id="bob1",
            pairwise_only=True,
            suffix="bob-removal-pending",
        )
        drop_row = self.select_pending_row(rows, selector="newest", purpose="Bob removal metadata")
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-bob-removal-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-remove-bob")

        self.send_message(case_dir, "alice1", chat_id, message, suffix="send-after-bob-remove")
        self.wait_message(case_dir, "carol1", chat_id, message, suffix="carol-receives-after-remove")
        self.assert_message_absent(case_dir, "bob1", chat_id, message, timeout_ms=30000, suffix="bob-removed-absent")
        bob_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-denied")
        pending = self.pending_repair_count(bob_debug)

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=pending > 0,
            visible_result_ok=True,
            final_pending_repair_count=pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "message": message,
                "bob_pending_repair_count_after_denial": pending,
            },
        )

    def case_sender_key_late_member_pre_add_denied(self, case_dir: Path) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"Late Pre Add Denied {stamp}", ["bob1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        pre_add = f"late-member-pre-add-{stamp}"
        post_add = f"late-member-post-add-visible-{stamp}"

        self.send_message(case_dir, "alice1", chat_id, pre_add, suffix="send-pre-add")
        self.wait_message(case_dir, "bob1", chat_id, pre_add, suffix="bob-pre-add")
        self.add_group_member(
            case_dir,
            group_id,
            chat_id,
            "carol1",
            expected_member_count=3,
            wait_for_relay_drain=True,
        )
        self.wait_member_count(case_dir, "carol1", chat_id, 3, suffix="carol-sees-add")
        self.assert_message_absent(case_dir, "carol1", chat_id, pre_add, timeout_ms=30000, suffix="carol-pre-add-absent")
        self.send_message(case_dir, "alice1", chat_id, post_add, suffix="send-post-add-visible")
        self.wait_message(case_dir, "carol1", chat_id, post_add, suffix="carol-post-add-visible")
        carol_debug = self.report_protocol_debug(case_dir, "carol1", "carol-debug-final")
        pending = self.pending_repair_count(carol_debug)

        return CaseResult(
            case="",
            status="passed",
            fault_injected=False,
            repair_observed=pending > 0,
            visible_result_ok=True,
            final_pending_repair_count=pending,
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "pre_add_message": pre_add,
                "post_add_message": post_add,
            },
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
                "passed": all(result.status == "passed" for result in self.results),
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
