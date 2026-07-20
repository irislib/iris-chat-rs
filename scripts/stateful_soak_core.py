#!/usr/bin/env python3
"""Pure model, generation, and journaling primitives for stateful_soak."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from mobile_scenario_support import redact_sensitive_text


SCHEMA_VERSION = 1
SENSITIVE_KEY = re.compile(
    r"secret|private|nsec|invite_input|invite_url|link_url|device_input",
    re.IGNORECASE,
)


class InvariantViolation(AssertionError):
    """The observed app state contradicted the independent model."""


def utc_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def stamp() -> str:
    return time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())


def split_names(value: str) -> list[str]:
    return [part.strip() for part in re.split(r"[,|]+", value) if part.strip()]


def command_output(*args: str) -> str:
    try:
        completed = subprocess.run(
            list(args),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=15,
            check=False,
        )
    except OSError as error:
        return str(error)
    return completed.stdout.strip()


def parse_duration(value: str) -> float:
    match = re.fullmatch(r"\s*(\d+(?:\.\d+)?)\s*([smh]?)\s*", value)
    if not match:
        raise argparse.ArgumentTypeError("duration must look like 30s, 5m, or 2h")
    amount = float(match.group(1))
    factor = {"": 1.0, "s": 1.0, "m": 60.0, "h": 3600.0}[match.group(2)]
    if amount <= 0:
        raise argparse.ArgumentTypeError("duration must be positive")
    return amount * factor


def safe_value(value: Any, key: str = "") -> Any:
    if SENSITIVE_KEY.search(key):
        return "<redacted>"
    if isinstance(value, dict):
        return {str(k): safe_value(v, str(k)) for k, v in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [safe_value(item) for item in value]
    if isinstance(value, str):
        if value.startswith("nsec"):
            return "<redacted>"
        return redact_sensitive_text(value)
    return value


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(safe_value(value), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


class Journal:
    def __init__(self, path: Path):
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self._handle = path.open("a", encoding="utf-8", buffering=1)
        self._event = 0
        self._started = time.monotonic()

    def append(self, event_type: str, **fields: Any) -> dict[str, Any]:
        self._event += 1
        record = {
            "schema_version": SCHEMA_VERSION,
            "event": self._event,
            "type": event_type,
            "at": utc_now(),
            "elapsed_secs": round(time.monotonic() - self._started, 3),
            **safe_value(fields),
        }
        self._handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
        self._handle.flush()
        os.fsync(self._handle.fileno())
        return record

    def close(self) -> None:
        if not self._handle.closed:
            self._handle.flush()
            os.fsync(self._handle.fileno())
            self._handle.close()


class Tee:
    def __init__(self, *streams: Any):
        self.streams = streams

    def write(self, value: str) -> int:
        value = redact_sensitive_text(value)
        for stream in self.streams:
            stream.write(value)
        return len(value)

    def flush(self) -> None:
        for stream in self.streams:
            stream.flush()

    def isatty(self) -> bool:
        return any(getattr(stream, "isatty", lambda: False)() for stream in self.streams)


@dataclass(frozen=True)
class Action:
    kind: str
    args: dict[str, Any]

    def as_dict(self) -> dict[str, Any]:
        return {"kind": self.kind, "args": self.args}

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "Action":
        return cls(kind=str(value["kind"]), args=dict(value.get("args") or {}))


@dataclass
class DeviceModel:
    user: str
    provisioned: bool
    authorized: bool
    running: bool = True

    def as_dict(self) -> dict[str, Any]:
        return {
            "user": self.user,
            "provisioned": self.provisioned,
            "authorized": self.authorized,
            "running": self.running,
        }


@dataclass
class GroupModel:
    symbol: str
    name: str
    creator: str
    members: set[str]
    admins: set[str]
    ever_members: set[str]
    chat_id: str = ""
    group_id: str = ""
    last_message: dict[str, str] | None = None

    def as_dict(self) -> dict[str, Any]:
        return {
            "symbol": self.symbol,
            "name": self.name,
            "creator": self.creator,
            "members": sorted(self.members),
            "admins": sorted(self.admins),
            "ever_members": sorted(self.ever_members),
            "chat_id": self.chat_id,
            "group_id": self.group_id,
            "last_message": self.last_message,
        }


@dataclass
class WorldModel:
    seed: int
    devices: dict[str, DeviceModel] = field(default_factory=dict)
    groups: dict[str, GroupModel] = field(default_factory=dict)
    direct_latest: dict[str, dict[str, str]] = field(default_factory=dict)
    coverage: set[str] = field(default_factory=set)
    successful_actions: int = 0

    @classmethod
    def initial(cls, seed: int) -> "WorldModel":
        return cls(
            seed=seed,
            devices={
                "alice1": DeviceModel("alice", True, True),
                "bob1": DeviceModel("bob", True, True),
                "carol1": DeviceModel("carol", True, True),
                "alice2": DeviceModel("alice", False, False, False),
            },
        )

    @property
    def users(self) -> tuple[str, ...]:
        return ("alice", "bob", "carol")

    def active_devices(self, user: str | None = None) -> list[str]:
        return sorted(
            device_id
            for device_id, device in self.devices.items()
            if device.provisioned
            and device.authorized
            and device.running
            and (user is None or device.user == user)
        )

    def primary(self, user: str) -> str:
        candidates = [device_id for device_id in self.active_devices(user) if device_id.endswith("1")]
        if not candidates:
            raise InvariantViolation(f"no active primary device for {user}")
        return candidates[0]

    def direct_key(self, first: str, second: str) -> str:
        return "|".join(sorted((first, second)))

    def as_dict(self) -> dict[str, Any]:
        return {
            "seed": self.seed,
            "successful_actions": self.successful_actions,
            "coverage": sorted(self.coverage),
            "devices": {key: value.as_dict() for key, value in sorted(self.devices.items())},
            "groups": {key: value.as_dict() for key, value in sorted(self.groups.items())},
            "direct_latest": dict(sorted(self.direct_latest.items())),
        }

    def digest(self) -> str:
        rendered = json.dumps(self.as_dict(), sort_keys=True, separators=(",", ":"))
        return hashlib.sha256(rendered.encode("utf-8")).hexdigest()


class ActionGenerator:
    COVERAGE_ORDER = (
        "link_device",
        "direct_send",
        "create_group",
        "group_send",
        "add_group_member",
        "rename_group",
        "set_group_admin",
        "restart_device",
        "offline_direct_catchup",
        "remove_group_member",
        "revoke_device",
        "audit",
    )
    WEIGHTS = {
        "direct_send": 28,
        "group_send": 25,
        "restart_device": 10,
        "offline_direct_catchup": 8,
        "rename_group": 5,
        "set_group_admin": 4,
        "add_group_member": 4,
        "remove_group_member": 4,
        "create_group": 3,
        "link_device": 2,
        "revoke_device": 1,
        "audit": 6,
    }

    def __init__(self, seed: int, max_groups: int):
        self.rng = random.Random(seed)
        self.max_groups = max_groups

    def eligible(self, model: WorldModel) -> list[str]:
        result = ["direct_send", "restart_device", "offline_direct_catchup", "audit"]
        if not model.devices["alice2"].provisioned:
            result.append("link_device")
        elif model.devices["alice2"].authorized and len(model.coverage) >= 8:
            result.append("revoke_device")
        if len(model.groups) < self.max_groups:
            result.append("create_group")
        if model.groups:
            result.extend(("group_send", "rename_group"))
        if any(set(model.users) - group.members for group in model.groups.values()):
            result.append("add_group_member")
        if any(len(group.members) > 2 for group in model.groups.values()):
            result.append("remove_group_member")
        if any(group.members - {group.creator} for group in model.groups.values()):
            result.append("set_group_admin")
        return sorted(set(result))

    def next(self, model: WorldModel, sequence: int) -> Action:
        eligible = self.eligible(model)
        uncovered = [kind for kind in self.COVERAGE_ORDER if kind in eligible and kind not in model.coverage]
        if uncovered:
            kind = uncovered[0]
        else:
            kind = self.rng.choices(
                eligible,
                weights=[self.WEIGHTS[kind] for kind in eligible],
                k=1,
            )[0]
        return self._make(kind, model, sequence)

    def _message(self, model: WorldModel, sequence: int, kind: str) -> str:
        return f"iris-soak-{model.seed}-{sequence:06d}-{kind}"

    def _admin_device(self, model: WorldModel, group: GroupModel) -> str:
        # The creator is never removed by this model, so using it keeps generated
        # administration actions valid even while other admin roles are toggled.
        return model.primary(group.creator)

    def _make(self, kind: str, model: WorldModel, sequence: int) -> Action:
        if kind == "link_device":
            return Action(kind, {"device": "alice2"})
        if kind in {"direct_send", "offline_direct_catchup"}:
            sender = self.rng.choice(model.active_devices())
            sender_user = model.devices[sender].user
            recipient = self.rng.choice([user for user in model.users if user != sender_user])
            args: dict[str, Any] = {
                "sender": sender,
                "recipient": recipient,
                "message": self._message(model, sequence, kind),
            }
            if kind == "offline_direct_catchup":
                args["offline_device"] = self.rng.choice(model.active_devices(recipient))
            return Action(kind, args)
        if kind == "create_group":
            creator = self.rng.choice(model.users)
            others = [user for user in model.users if user != creator]
            members = [self.rng.choice(others)]
            symbol = f"group-{len(model.groups) + 1}"
            return Action(
                kind,
                {
                    "group": symbol,
                    "actor": model.primary(creator),
                    "members": members,
                    "name": f"Iris Soak {model.seed} {symbol}",
                },
            )
        groups = sorted(model.groups.values(), key=lambda group: group.symbol)
        if kind == "group_send":
            group = self.rng.choice(groups)
            sender_user = self.rng.choice(sorted(group.members))
            return Action(
                kind,
                {
                    "group": group.symbol,
                    "sender": self.rng.choice(model.active_devices(sender_user)),
                    "message": self._message(model, sequence, kind),
                },
            )
        if kind == "add_group_member":
            candidates = [group for group in groups if set(model.users) - group.members]
            group = self.rng.choice(candidates)
            target = self.rng.choice(sorted(set(model.users) - group.members))
            return Action(
                kind,
                {
                    "group": group.symbol,
                    "actor": self._admin_device(model, group),
                    "user": target,
                },
            )
        if kind == "remove_group_member":
            candidates = [group for group in groups if len(group.members) > 2]
            group = self.rng.choice(candidates)
            removable = sorted(group.members - {group.creator})
            return Action(
                kind,
                {
                    "group": group.symbol,
                    "actor": self._admin_device(model, group),
                    "user": self.rng.choice(removable),
                },
            )
        if kind == "rename_group":
            group = self.rng.choice(groups)
            return Action(
                kind,
                {
                    "group": group.symbol,
                    "actor": self._admin_device(model, group),
                    "name": f"Iris Soak {model.seed} renamed {sequence}",
                },
            )
        if kind == "set_group_admin":
            group = self.rng.choice(
                [group for group in groups if group.members - {group.creator}]
            )
            user = self.rng.choice(sorted(group.members - {group.creator}))
            return Action(
                kind,
                {
                    "group": group.symbol,
                    "actor": self._admin_device(model, group),
                    "user": user,
                    "is_admin": user not in group.admins,
                },
            )
        if kind == "restart_device":
            return Action(kind, {"device": self.rng.choice(model.active_devices())})
        if kind == "revoke_device":
            return Action(kind, {"actor": "alice1", "device": "alice2"})
        if kind == "audit":
            return Action(kind, {"reason": "generated"})
        raise ValueError(f"unknown action kind: {kind}")


def read_replay_actions(path: Path) -> tuple[list[Action], dict[str, Any]]:
    actions: list[Action] = []
    metadata: dict[str, Any] = {}
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as error:
                raise SystemExit(f"invalid journal JSON at line {line_number}: {error}") from error
            if record.get("schema_version") != SCHEMA_VERSION:
                raise SystemExit(f"unsupported journal schema at line {line_number}")
            if record.get("type") == "run_started":
                metadata = dict(record.get("run") or {})
            elif record.get("type") == "semantic_action_planned":
                actions.append(Action.from_dict(record["action"]))
    if not actions:
        raise SystemExit(f"journal contains no semantic actions: {path}")
    return actions, metadata
