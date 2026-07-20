#!/usr/bin/env python3
"""Stateful model-based soak tests over real Iris app instances.

The runner owns an independent expected-state model, chooses only actions whose
preconditions hold, records every intended action before executing it, and
checks externally visible invariants after each state transition. Journals are
semantic: they can be replayed with fresh disposable identities.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import random
import subprocess
import sys
import time
import traceback
from pathlib import Path
from typing import Any

from mobile_scenario import ROOT_DIR, Scenario
from mobile_scenario_support import redact_sensitive_text
from mixed_linked_device_revocation import roster_device_hexes
from stateful_soak_core import (
    Action,
    ActionGenerator,
    DeviceModel,
    GroupModel,
    InvariantViolation,
    Journal,
    Tee,
    WorldModel,
    command_output,
    parse_duration,
    read_replay_actions,
    safe_value,
    split_names,
    stamp,
    utc_now,
    write_json,
)
from mixed_offline_restart_recovery import (
    free_tcp_port,
    restart_app,
    send_chat,
    send_peer,
    stop_app,
    wait_chat,
    wait_peer,
)


DEFAULT_PUBLIC_RELAYS = ",".join(
    (
        "wss://relay.damus.io",
        "wss://nos.lol",
        "wss://relay.primal.net",
        "wss://relay.snort.social",
        "wss://temp.iris.to",
    )
)
DEFAULT_SIMULATORS = (
    "Iris Soak Alice",
    "Iris Soak Bob",
    "Iris Soak Carol",
    "Iris Soak Alice Linked",
)
ZERO_RUNTIME_KEYS = (
    "pending_protocol_outbound_count",
    "pending_group_fanout_count",
    "pending_group_sender_key_message_count",
    "pending_group_sender_key_retry_count",
    "pending_group_sender_key_unmapped_count",
    "pending_group_sender_key_repair_count",
)


def git_output(*args: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=str(ROOT_DIR),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    return completed.stdout.strip()


def public_relay_probe(value: str) -> list[dict[str, str]]:
    return [
        {
            "url": url,
            "result": command_output(
                "curl", "-sS", "-o", "/dev/null", "-w",
                "code=%{http_code} remote=%{remote_ip} tls=%{ssl_verify_result} time=%{time_total}",
                "--max-time", "10", url.replace("wss://", "https://", 1),
            ),
        }
        for url in value.split(",")
    ]


class SoakScenario(Scenario):
    def __init__(self, config_path: Path, journal: Journal):
        self.journal = journal
        super().__init__(config_path)

    def setup_accounts(self) -> None:
        rebuild_next_ios = bool(self.config.get("ios", {}).get("build", True))
        for device in self.config.get("devices", []):
            if device.get("linked_to") or device.get("defer_provisioning"):
                continue
            rebuild = bool(device.get("platform") == "ios" and rebuild_next_ios)
            self.create_account(device, rebuild=rebuild)
            if device.get("platform") == "ios":
                rebuild_next_ios = False
        for device in self.config.get("devices", []):
            if device.get("linked_to") and not device.get("defer_provisioning"):
                self.link_device(device)

    def record_harness_action(self, **kwargs: Any) -> None:
        super().record_harness_action(**kwargs)
        self.journal.append("harness_action_finished", action=self.action_history[-1])

    def link_device(self, device: dict[str, Any]) -> None:
        try:
            super().link_device(device)
        finally:
            device_id = device["id"]
            for suffix in ("link.status", "link.log"):
                path = self.work_dir / f"{device_id}-{suffix}"
                if path.exists():
                    path.write_text(
                        redact_sensitive_text(path.read_text(encoding="utf-8", errors="replace")),
                        encoding="utf-8",
                    )


class NativeExecutor:
    def __init__(
        self,
        scenario: SoakScenario,
        model: WorldModel,
        timeout_secs: int,
        absence_timeout_ms: int,
    ):
        self.scenario = scenario
        self.model = model
        self.timeout = str(timeout_secs)
        self.absence_timeout_ms = str(absence_timeout_ms)

    def execute(self, action: Action) -> dict[str, Any]:
        handler = getattr(self, f"action_{action.kind}", None)
        if handler is None:
            raise ValueError(f"unsupported action: {action.kind}")
        result = handler(**action.args)
        self.model.successful_actions += 1
        self.model.coverage.add(action.kind)
        return result

    def primary_config(self, device_id: str) -> dict[str, Any]:
        for device in self.scenario.config.get("devices", []):
            if device["id"] == device_id:
                return device
        raise KeyError(device_id)

    def assert_count(self, statuses: dict[str, str], label: str) -> str:
        observed = statuses.get("matching_count", "")
        if observed != "1":
            raise InvariantViolation(f"{label}: expected one matching message, observed {observed!r}")
        return observed

    def wait_direct_everywhere(self, sender: str, recipient: str, message: str) -> dict[str, str]:
        sender_user = self.model.devices[sender].user
        observations: dict[str, str] = {}
        for user, peer_user, direction in (
            (sender_user, recipient, "outgoing"),
            (recipient, sender_user, "incoming"),
        ):
            peer_device = self.model.primary(peer_user)
            for device_id in self.model.active_devices(user):
                statuses = wait_peer(
                    self.scenario,
                    device_id,
                    peer_device,
                    message,
                    direction=direction,
                    expected_count=1,
                    timeout_secs=self.timeout,
                )
                observations[device_id] = self.assert_count(statuses, f"direct message on {device_id}")
        return observations

    def wait_group_everywhere(self, group: GroupModel, sender: str, message: str) -> dict[str, str]:
        sender_user = self.model.devices[sender].user
        observations: dict[str, str] = {}
        for user in sorted(group.members):
            direction = "outgoing" if user == sender_user else "incoming"
            for device_id in self.model.active_devices(user):
                statuses = wait_chat(
                    self.scenario,
                    device_id,
                    group.chat_id,
                    message,
                    direction=direction,
                    expected_count=1,
                    timeout_secs=self.timeout,
                )
                observations[device_id] = self.assert_count(statuses, f"group message on {device_id}")
        for former_user in sorted(group.ever_members - group.members):
            for device_id in self.model.active_devices(former_user):
                statuses = self.scenario.harness(
                    device_id,
                    "assert_message_absent_from_args",
                    args={
                        "chat_id": group.chat_id,
                        "message": message,
                        "direction": "any",
                        "timeout_ms": self.absence_timeout_ms,
                    },
                )
                observations[device_id] = statuses.get("matching_count", "0")
        return observations

    def wait_group_shape(self, group: GroupModel, include_former: bool = False) -> dict[str, Any]:
        devices: set[str] = set()
        users = group.members | (group.ever_members - group.members if include_former else set())
        for user in users:
            devices.update(self.model.active_devices(user))
        observations: dict[str, Any] = {}
        for device_id in sorted(devices):
            member_status = self.scenario.harness(
                device_id,
                "wait_for_group_member_count_from_args",
                args={"chat_id": group.chat_id, "member_count": str(len(group.members)), "timeout_secs": self.timeout},
            )
            name_status = self.scenario.harness(
                device_id,
                "wait_for_group_name_from_args",
                args={"chat_id": group.chat_id, "group_name": group.name, "timeout_secs": self.timeout},
            )
            if member_status.get("member_count") != str(len(group.members)):
                raise InvariantViolation(f"{device_id}: group {group.symbol} member count diverged")
            if name_status.get("group_name") != group.name:
                raise InvariantViolation(f"{device_id}: group {group.symbol} name diverged")
            observations[device_id] = {
                "member_count": member_status.get("member_count", ""),
                "group_name": name_status.get("group_name", ""),
            }
        return observations

    def action_link_device(self, device: str) -> dict[str, Any]:
        if self.model.devices[device].provisioned:
            raise InvariantViolation(f"{device} is already provisioned")
        self.scenario.link_device(self.primary_config(device))
        self.model.devices[device].provisioned = True
        self.model.devices[device].authorized = True
        self.model.devices[device].running = True
        roster = self.scenario.harness("alice1", "report_device_roster_snapshot")
        device_hex = self.scenario.state["devices"][device]["device_hex"].lower()
        if device_hex not in roster_device_hexes(roster):
            raise InvariantViolation(f"owner roster did not contain newly linked {device}")
        return {"linked": device, "owner_roster": roster}

    def action_revoke_device(self, actor: str, device: str) -> dict[str, Any]:
        target_hex = self.scenario.state["devices"][device]["device_hex"]
        before = self.scenario.harness(actor, "report_device_roster_snapshot")
        if target_hex.lower() not in roster_device_hexes(before):
            raise InvariantViolation(f"owner roster omitted {device} before revocation")
        revoke = self.scenario.harness(
            actor,
            "remove_authorized_device_from_args",
            args={"device_input": target_hex},
        )
        after = self.scenario.harness(actor, "report_device_roster_snapshot")
        if target_hex.lower() in roster_device_hexes(after):
            raise InvariantViolation(f"owner roster retained {device} after revocation")
        revoked = self.scenario.harness(device, "wait_for_revoked_state")
        self.model.devices[device].authorized = False
        self.model.devices[device].running = False
        return {"before": before, "revoke": revoke, "after": after, "revoked": revoked}

    def action_direct_send(self, sender: str, recipient: str, message: str) -> dict[str, Any]:
        peer = self.model.primary(recipient)
        sent = send_peer(
            self.scenario,
            sender,
            peer,
            message,
            relay_drain_timeout_secs=self.timeout,
            timeout_secs=self.timeout,
        )
        observed = self.wait_direct_everywhere(sender, recipient, message)
        sender_user = self.model.devices[sender].user
        self.model.direct_latest[self.model.direct_key(sender_user, recipient)] = {
            "sender": sender,
            "recipient": recipient,
            "message": message,
        }
        return {"sent": sent, "observed": observed}

    def action_offline_direct_catchup(
        self,
        sender: str,
        recipient: str,
        message: str,
        offline_device: str,
    ) -> dict[str, Any]:
        if self.model.devices[offline_device].user != recipient:
            raise InvariantViolation("offline target does not belong to recipient")
        peer = self.model.primary(recipient)
        stop_app(self.scenario, offline_device)
        self.model.devices[offline_device].running = False
        sent: dict[str, str] = {}
        try:
            sent = send_peer(
                self.scenario,
                sender,
                peer,
                message,
                wait_for_delivery=False,
                wait_for_relay_drain=True,
                relay_drain_timeout_secs=self.timeout,
                timeout_secs=self.timeout,
            )
        finally:
            restart_app(
                self.scenario,
                offline_device,
                wait_for_drain=True,
                relay_drain_timeout_secs=self.timeout,
            )
            self.model.devices[offline_device].running = True
        observed = self.wait_direct_everywhere(sender, recipient, message)
        sender_user = self.model.devices[sender].user
        self.model.direct_latest[self.model.direct_key(sender_user, recipient)] = {
            "sender": sender,
            "recipient": recipient,
            "message": message,
        }
        return {"offline_device": offline_device, "sent": sent, "observed": observed}

    def action_create_group(
        self,
        group: str,
        actor: str,
        members: list[str],
        name: str,
    ) -> dict[str, Any]:
        creator = self.model.devices[actor].user
        member_inputs = ",".join(self.scenario.state["users"][user]["npub"] for user in members)
        statuses = self.scenario.harness(
            actor,
            "create_group_from_args",
            args={
                "group_name": name,
                "member_inputs": member_inputs,
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": self.timeout,
                "timeout_secs": self.timeout,
            },
        )
        expected_members = {creator, *members}
        group_model = GroupModel(
            symbol=group,
            name=name,
            creator=creator,
            members=expected_members,
            admins={creator},
            ever_members=set(expected_members),
            chat_id=statuses["chat_id"],
            group_id=statuses["group_id"],
        )
        self.model.groups[group] = group_model
        self.scenario.state.setdefault("groups", {})[group] = {
            "name": name,
            "creator": actor,
            "chat_id": group_model.chat_id,
            "group_id": group_model.group_id,
        }
        self.scenario.save_state()
        for user in sorted(expected_members):
            for device_id in self.model.active_devices(user):
                self.scenario.harness(
                    device_id,
                    "wait_for_group_chat_from_args",
                    args={"chat_id": group_model.chat_id, "timeout_secs": self.timeout},
                )
        shape = self.wait_group_shape(group_model)
        return {"created": statuses, "shape": shape}

    def action_group_send(self, group: str, sender: str, message: str) -> dict[str, Any]:
        group_model = self.model.groups[group]
        sender_user = self.model.devices[sender].user
        if sender_user not in group_model.members:
            raise InvariantViolation(f"{sender_user} is not a member of {group}")
        sent = send_chat(
            self.scenario,
            sender,
            group_model.chat_id,
            message,
            relay_drain_timeout_secs=self.timeout,
            timeout_secs=self.timeout,
        )
        observed = self.wait_group_everywhere(group_model, sender, message)
        group_model.last_message = {"sender": sender, "message": message}
        return {"sent": sent, "observed": observed}

    def action_add_group_member(self, group: str, actor: str, user: str) -> dict[str, Any]:
        group_model = self.model.groups[group]
        statuses = self.scenario.harness(
            actor,
            "add_group_members_from_args",
            args={
                "group_id": group_model.group_id,
                "chat_id": group_model.chat_id,
                "member_inputs": self.scenario.state["users"][user]["npub"],
                "expected_member_count": str(len(group_model.members) + 1),
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": self.timeout,
                "timeout_secs": self.timeout,
            },
        )
        group_model.members.add(user)
        group_model.ever_members.add(user)
        for device_id in self.model.active_devices(user):
            self.scenario.harness(
                device_id,
                "wait_for_group_chat_from_args",
                args={"chat_id": group_model.chat_id, "timeout_secs": self.timeout},
            )
        return {"updated": statuses, "shape": self.wait_group_shape(group_model)}

    def action_remove_group_member(self, group: str, actor: str, user: str) -> dict[str, Any]:
        group_model = self.model.groups[group]
        statuses = self.scenario.harness(
            actor,
            "remove_group_member_from_args",
            args={
                "group_id": group_model.group_id,
                "chat_id": group_model.chat_id,
                "member_input": self.scenario.state["users"][user]["owner_hex"],
                "expected_member_count": str(len(group_model.members) - 1),
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": self.timeout,
                "timeout_secs": self.timeout,
            },
        )
        group_model.members.remove(user)
        group_model.admins.discard(user)
        return {"updated": statuses, "shape": self.wait_group_shape(group_model, include_former=True)}

    def action_rename_group(self, group: str, actor: str, name: str) -> dict[str, Any]:
        group_model = self.model.groups[group]
        statuses = self.scenario.harness(
            actor,
            "update_group_name_from_args",
            args={
                "group_id": group_model.group_id,
                "chat_id": group_model.chat_id,
                "group_name": name,
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": self.timeout,
                "timeout_secs": self.timeout,
            },
        )
        group_model.name = name
        return {"updated": statuses, "shape": self.wait_group_shape(group_model)}

    def action_set_group_admin(
        self,
        group: str,
        actor: str,
        user: str,
        is_admin: bool,
    ) -> dict[str, Any]:
        group_model = self.model.groups[group]
        statuses = self.scenario.harness(
            actor,
            "set_group_admin_from_args",
            args={
                "group_id": group_model.group_id,
                "member_input": self.scenario.state["users"][user]["owner_hex"],
                "is_admin": str(is_admin).lower(),
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": self.timeout,
            },
        )
        if is_admin:
            group_model.admins.add(user)
        else:
            group_model.admins.discard(user)
        observed: dict[str, str] = {}
        for member in sorted(group_model.members):
            for device_id in self.model.active_devices(member):
                result = self.scenario.harness(
                    device_id,
                    "wait_for_group_admin_from_args",
                    args={
                        "group_id": group_model.group_id,
                        "member_input": self.scenario.state["users"][user]["owner_hex"],
                        "is_admin": str(is_admin).lower(),
                        "timeout_secs": self.timeout,
                    },
                )
                observed[device_id] = result.get("is_admin", str(is_admin).lower())
        return {"updated": statuses, "observed": observed}

    def action_restart_device(self, device: str) -> dict[str, Any]:
        restart_app(
            self.scenario,
            device,
            wait_for_drain=True,
            relay_drain_timeout_secs=self.timeout,
        )
        self.model.devices[device].running = True
        return {"restarted": device}

    def action_audit(self, reason: str) -> dict[str, Any]:
        observations: dict[str, Any] = {"reason": reason, "devices": {}, "groups": {}, "direct": {}}
        for device_id in self.model.active_devices():
            runtime = self.scenario.harness(
                device_id,
                "report_runtime_debug_snapshot",
                args={
                    "wait_for_relay_drain": "true",
                    "wait_for_runtime_idle": "true",
                    "relay_drain_timeout_secs": self.timeout,
                    "runtime_idle_timeout_secs": self.timeout,
                },
            )
            persisted = self.scenario.harness(
                device_id,
                "report_persisted_protocol_snapshot",
                args={"wait_for_relay_drain": "true", "relay_drain_timeout_secs": self.timeout},
            )
            nonzero = {
                key: runtime[key]
                for key in ZERO_RUNTIME_KEYS
                if key in runtime and runtime[key] not in {"", "0"}
            }
            if nonzero:
                raise InvariantViolation(f"{device_id}: runtime did not settle: {nonzero}")
            observations["devices"][device_id] = {"runtime": runtime, "persisted": persisted}
        for symbol, group in sorted(self.model.groups.items()):
            group_result: dict[str, Any] = {"shape": self.wait_group_shape(group)}
            if group.last_message:
                group_result["last_message"] = self.wait_group_everywhere(
                    group,
                    group.last_message["sender"],
                    group.last_message["message"],
                )
            observations["groups"][symbol] = group_result
        for key, expected in sorted(self.model.direct_latest.items()):
            observations["direct"][key] = self.wait_direct_everywhere(
                expected["sender"],
                expected["recipient"],
                expected["message"],
            )
        for user in self.model.users:
            primary = self.model.primary(user)
            roster = self.scenario.harness(primary, "report_device_roster_snapshot")
            actual = roster_device_hexes(roster)
            for device_id, device in self.model.devices.items():
                if device.user != user or not device.provisioned:
                    continue
                device_hex = self.scenario.state["devices"][device_id].get("device_hex", "").lower()
                if device.authorized and device_hex and device_hex not in actual:
                    raise InvariantViolation(f"{primary}: roster omitted authorized {device_id}")
                if not device.authorized and device_hex and device_hex in actual:
                    raise InvariantViolation(f"{primary}: roster retained revoked {device_id}")
            observations["devices"][primary]["roster"] = roster
        return observations

    def capture_failure_evidence(self, failure_dir: Path) -> dict[str, Any]:
        failure_dir.mkdir(parents=True, exist_ok=True)
        captured: dict[str, Any] = {}
        for device_id, device in sorted(self.scenario.state.get("devices", {}).items()):
            device_capture: dict[str, Any] = {}
            if device.get("owner_hex") and self.model.devices.get(device_id, DeviceModel("", False, False)).authorized:
                for action in ("report_runtime_debug_snapshot", "report_persisted_protocol_snapshot"):
                    try:
                        device_capture[action] = self.scenario.harness(device_id, action)
                    except BaseException as error:
                        device_capture[action] = {"capture_error": f"{type(error).__name__}: {error}"}
            if device.get("platform") == "ios" and device.get("udid"):
                log_path = failure_dir / f"{device_id}-system.log"
                try:
                    completed = subprocess.run(
                        [
                            "xcrun",
                            "simctl",
                            "spawn",
                            device["udid"],
                            "log",
                            "show",
                            "--last",
                            "10m",
                            "--style",
                            "compact",
                            "--predicate",
                            'processImagePath CONTAINS[c] "Iris"',
                        ],
                        cwd=str(ROOT_DIR),
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        text=True,
                        encoding="utf-8",
                        errors="replace",
                        timeout=30,
                        check=False,
                    )
                    log_path.write_text(redact_sensitive_text(completed.stdout[-2_000_000:]), encoding="utf-8")
                    device_capture["system_log"] = str(log_path)
                except BaseException as error:
                    device_capture["system_log_error"] = f"{type(error).__name__}: {error}"
            captured[device_id] = device_capture
        write_json(failure_dir / "captured-state.json", captured)
        return captured


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    names = split_names(args.simulators)
    if len(names) != 4:
        raise SystemExit("--simulators must contain exactly four comma- or pipe-separated names")
    relay_mode = "public" if args.profile.endswith("public") else "local"
    if relay_mode == "public":
        relay = {
            "start": False,
            "port": 0,
            "set_id": f"stateful-soak-public-{artifact_dir.name}",
            "url": args.public_relays,
        }
    else:
        port = args.relay_port or free_tcp_port()
        relay = {
            "port": port,
            "label": f"iris.stateful-soak.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"stateful-soak-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "url": args.relay_url or f"ws://127.0.0.1:{port}",
        }
    devices = [
        {
            "id": "alice1",
            "platform": "ios",
            "simulator": names[0],
            "run_id": "alice1",
            "user": "alice",
            "display_name": "Alice",
            "reset": True,
            "relay_drain_timeout_secs": args.timeout_secs,
        },
        {
            "id": "bob1",
            "platform": "ios",
            "simulator": names[1],
            "run_id": "bob1",
            "user": "bob",
            "display_name": "Bob",
            "reset": True,
            "relay_drain_timeout_secs": args.timeout_secs,
        },
        {
            "id": "carol1",
            "platform": "ios",
            "simulator": names[2],
            "run_id": "carol1",
            "user": "carol",
            "display_name": "Carol",
            "reset": True,
            "relay_drain_timeout_secs": args.timeout_secs,
        },
        {
            "id": "alice2",
            "platform": "ios",
            "simulator": names[3],
            "run_id": "alice2",
            "user": "alice",
            "linked_to": "alice",
            "display_name": "Alice Linked",
            "reset": True,
            "defer_provisioning": True,
            "relay_drain_timeout_secs": args.timeout_secs,
            "authorization_timeout_secs": args.timeout_secs * 2,
        },
    ]
    config = {
        "name": f"stateful-soak-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": relay,
        "ios": {"build": not args.skip_build, "lazy_boot": True},
        "android": {"build": False},
        "open_apps": False,
        "devices": devices,
        "groups": [],
    }
    config_path = artifact_dir / "config.json"
    write_json(config_path, config)
    return config_path


def artifact_index(root: Path) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.name == "artifact-index.json":
            continue
        digest = hashlib.sha256()
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        result.append(
            {
                "path": str(path.relative_to(root)),
                "bytes": path.stat().st_size,
                "sha256": digest.hexdigest(),
            }
        )
    return result


def run_metadata(args: argparse.Namespace, artifact_dir: Path, seed: int, replay_source: Path | None) -> dict[str, Any]:
    return {
        "artifact_dir": str(artifact_dir),
        "command": [safe_value(part) for part in sys.argv],
        "created_at": utc_now(),
        "duration_secs": args.duration,
        "git_head": git_output("rev-parse", "HEAD"),
        "git_status": git_output("status", "--short", "--branch"),
        "hostname": platform.node(),
        "max_actions": args.max_actions,
        "profile": args.profile,
        "public_relays": args.public_relays if args.profile.endswith("public") else "",
        "public_relay_probe": public_relay_probe(args.public_relays) if args.profile.endswith("public") else [],
        "python": sys.version,
        "rustc": command_output("rustc", "--version"),
        "cargo": command_output("cargo", "--version"),
        "xcodebuild": command_output("xcodebuild", "-version"),
        "replay_source": str(replay_source) if replay_source else "",
        "seed": seed,
        "simulators": split_names(args.simulators),
        "timeout_secs": args.timeout_secs,
    }


def make_artifact_dir(args: argparse.Namespace, seed: int, suffix: str = "") -> Path:
    if args.artifact_dir:
        return args.artifact_dir.resolve()
    tail = f"-{suffix}" if suffix else ""
    return (ROOT_DIR / "artifacts" / "stateful-soak" / f"{stamp()}-seed-{seed}{tail}").resolve()


def add_common_arguments(parser: argparse.ArgumentParser, profile_default: str | None) -> None:
    parser.add_argument("--profile", choices=("ios-local", "ios-public"), default=profile_default)
    parser.add_argument("--duration", type=parse_duration, default=parse_duration("10m"))
    parser.add_argument("--max-actions", type=int, default=0, help="0 means duration-only")
    parser.add_argument("--max-groups", type=int, default=3)
    parser.add_argument("--seed", type=int)
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--simulators", default="|".join(DEFAULT_SIMULATORS))
    parser.add_argument("--relay-port", type=int)
    parser.add_argument("--relay-url")
    parser.add_argument("--public-relays", default=DEFAULT_PUBLIC_RELAYS)
    parser.add_argument("--timeout-secs", type=int, default=120)
    parser.add_argument("--absence-timeout-ms", type=int, default=5000)
    parser.add_argument("--audit-every", type=int, default=10)
    parser.add_argument("--action-delay-secs", type=float, default=None)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--keep-devices-open", action="store_true")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run replayable stateful model-based Iris soak tests.")
    subparsers = parser.add_subparsers(dest="command", required=True)
    run_parser = subparsers.add_parser("run", help="Generate and execute a stateful action sequence.")
    add_common_arguments(run_parser, "ios-local")
    replay_parser = subparsers.add_parser("replay", help="Replay semantic actions from an earlier journal.")
    replay_parser.add_argument("journal", type=Path)
    add_common_arguments(replay_parser, None)
    args = parser.parse_args()
    if args.max_actions < 0 or args.max_groups < 1 or args.timeout_secs < 1 or args.audit_every < 0:
        parser.error("counts and timeouts must be non-negative, and max-groups/timeout must be positive")
    return args


def execute_journaled_action(
    executor: NativeExecutor,
    journal: Journal,
    model: WorldModel,
    artifact_dir: Path,
    sequence: int,
    action: Action,
) -> None:
    journal.append(
        "semantic_action_planned",
        sequence=sequence,
        action=action.as_dict(),
        model_before_sha256=model.digest(),
    )
    started = time.monotonic()
    try:
        result = executor.execute(action)
    except BaseException as error:
        journal.append(
            "semantic_action_finished",
            sequence=sequence,
            action=action.as_dict(),
            status="failed",
            duration_secs=round(time.monotonic() - started, 3),
            error_type=type(error).__name__,
            error=str(error),
            model_after_sha256=model.digest(),
        )
        raise
    write_json(artifact_dir / "model.json", model.as_dict())
    journal.append(
        "semantic_action_finished",
        sequence=sequence,
        action=action.as_dict(),
        status="passed",
        duration_secs=round(time.monotonic() - started, 3),
        result=result,
        model_after_sha256=model.digest(),
    )


def execute_run(args: argparse.Namespace) -> int:
    replay_actions: list[Action] | None = None
    replay_source: Path | None = None
    replay_metadata: dict[str, Any] = {}
    if args.command == "replay":
        replay_source = args.journal.resolve()
        replay_actions, replay_metadata = read_replay_actions(replay_source)
        if args.seed is None and replay_metadata.get("seed") is not None:
            args.seed = int(replay_metadata["seed"])
        if args.profile is None:
            args.profile = str(replay_metadata.get("profile") or "ios-local")
    seed = args.seed if args.seed is not None else random.SystemRandom().randrange(1, 2**63)
    artifact_dir = make_artifact_dir(args, seed, "replay" if replay_actions else "")
    artifact_dir.mkdir(parents=True, exist_ok=False)
    artifact_dir.chmod(0o700)
    original_stdout = sys.stdout
    original_stderr = sys.stderr
    console_handle = (artifact_dir / "console.log").open("w", encoding="utf-8", buffering=1)
    sys.stdout = Tee(original_stdout, console_handle)
    sys.stderr = Tee(original_stderr, console_handle)
    journal = Journal(artifact_dir / "actions.jsonl")
    metadata = run_metadata(args, artifact_dir, seed, replay_source)
    write_json(artifact_dir / "run.json", metadata)
    journal.append("run_started", run=metadata)
    config_path = build_config(args, artifact_dir)
    scenario = SoakScenario(config_path, journal)
    model = WorldModel.initial(seed)
    write_json(artifact_dir / "model.json", model.as_dict())
    generator = ActionGenerator(seed, args.max_groups)
    executor = NativeExecutor(scenario, model, args.timeout_secs, args.absence_timeout_ms)
    overall_started = time.monotonic()
    action_loop_started = overall_started
    attempted = 0
    succeeded = 0
    failure: dict[str, Any] | None = None
    setup_complete = False
    last_kind = ""
    action_delay = args.action_delay_secs
    if action_delay is None:
        action_delay = 2.0 if args.profile.endswith("public") else 0.0
    try:
        journal.append("phase_started", phase="scenario_setup")
        scenario.setup()
        for device_id in model.active_devices():
            scenario.harness(
                device_id,
                "wait_for_connected_relay",
                args={"timeout_secs": str(args.timeout_secs)},
            )
        setup_complete = True
        action_loop_started = time.monotonic()
        write_json(artifact_dir / "model.json", model.as_dict())
        journal.append("phase_finished", phase="scenario_setup", state=str(scenario.state_path))
        replay_index = 0
        while True:
            elapsed = time.monotonic() - action_loop_started
            if replay_actions is not None:
                if replay_index >= len(replay_actions):
                    break
                action = replay_actions[replay_index]
                replay_index += 1
            else:
                if elapsed >= args.duration:
                    break
                if args.max_actions and attempted >= args.max_actions:
                    break
                sequence = attempted + 1
                if args.audit_every and sequence > 1 and (sequence - 1) % args.audit_every == 0:
                    action = Action("audit", {"reason": f"periodic-{sequence - 1}"})
                else:
                    action = generator.next(model, sequence)
            attempted += 1
            last_kind = action.kind
            execute_journaled_action(executor, journal, model, artifact_dir, attempted, action)
            succeeded += 1
            if action_delay:
                time.sleep(action_delay)
        if setup_complete and last_kind != "audit":
            attempted += 1
            final_audit = Action("audit", {"reason": "final"})
            execute_journaled_action(executor, journal, model, artifact_dir, attempted, final_audit)
            succeeded += 1
    except KeyboardInterrupt as error:
        failure = {"classification": "interrupted", "error_type": type(error).__name__, "error": "user interrupt"}
    except BaseException as error:
        classification = "product_invariant_failure" if isinstance(error, InvariantViolation) else (
            "setup_or_infrastructure_failure" if not setup_complete else "product_or_infrastructure_failure"
        )
        failure = {
            "classification": classification,
            "error_type": type(error).__name__,
            "error": str(error),
            "traceback": traceback.format_exc(),
        }
        if not setup_complete:
            journal.append(
                "phase_finished",
                phase="scenario_setup",
                status="failed",
                error_type=type(error).__name__,
                error=str(error),
            )
        failure_dir = artifact_dir / "failure"
        write_json(failure_dir / "error.json", failure)
        write_json(failure_dir / "model-at-failure.json", model.as_dict())
        if scenario.state.get("devices"):
            failure["capture"] = executor.capture_failure_evidence(failure_dir)
    finally:
        soak_duration_secs = round(time.monotonic() - action_loop_started, 3) if setup_complete else 0.0
        journal.append("phase_started", phase="cleanup")
        try:
            scenario.cleanup(shutdown_devices=not args.keep_devices_open)
            journal.append("phase_finished", phase="cleanup", status="passed")
        except BaseException as cleanup_error:
            journal.append(
                "phase_finished",
                phase="cleanup",
                status="failed",
                error_type=type(cleanup_error).__name__,
                error=str(cleanup_error),
            )
            if failure is None:
                failure = {
                    "classification": "cleanup_failure",
                    "error_type": type(cleanup_error).__name__,
                    "error": str(cleanup_error),
                }
        summary = {
            "status": "failed" if failure else "passed",
            "artifact_dir": str(artifact_dir),
            "attempted_actions": attempted,
            "successful_actions": succeeded,
            "coverage": sorted(model.coverage),
            "duration_secs": round(time.monotonic() - overall_started, 3),
            "soak_duration_secs": soak_duration_secs,
            "failure": failure,
            "journal": str(journal.path),
            "model": str(artifact_dir / "model.json"),
            "scenario_state": str(scenario.state_path),
            "seed": seed,
        }
        write_json(artifact_dir / "summary.json", summary)
        journal.append("run_finished", summary=summary)
        journal.close()
        print(json.dumps(safe_value(summary), indent=2, sort_keys=True))
        sys.stdout.flush()
        sys.stderr.flush()
        sys.stdout = original_stdout
        sys.stderr = original_stderr
        console_handle.close()
        write_json(artifact_dir / "artifact-index.json", artifact_index(artifact_dir))
    return 1 if failure else 0


def main() -> int:
    return execute_run(parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
