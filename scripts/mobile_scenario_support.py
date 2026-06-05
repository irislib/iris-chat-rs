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
PENDING_PUBLISHES = ROOT_DIR / "scripts" / "pending_relay_publishes.py"
ANDROID_RUNNER = "to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CLASS = "to.iris.chat.RealRelayHarnessTest"
ANDROID_APP_PACKAGE = "to.iris.chat.debug"
ANDROID_TEST_PACKAGE = "to.iris.chat.test"
STATUS_RE = re.compile(r"^(?:HARNESS_STATUS|INSTRUMENTATION_STATUS): ([^=]+)=(.*)$")
RAW_STATUS_RE = re.compile(r"^([a-z_][a-z0-9_]*)=(.*)$")
SENSITIVE_VALUE_RE = re.compile(
    r"((?:^|\s|:)(?:secret_key|IRIS_IOS_HARNESS_SECRET_KEY)=)([^ \n\r]+)",
    re.IGNORECASE,
)
SENSITIVE_ARG_RE = re.compile(r"(secret|private|nsec)", re.IGNORECASE)


def redact_sensitive_text(value: str) -> str:
    return SENSITIVE_VALUE_RE.sub(r"\1<redacted>", value)


def redact_status_value(key: str, value: Any) -> str:
    text = str(value)
    if SENSITIVE_ARG_RE.search(key) or text.startswith("nsec"):
        return "<redacted>"
    return redact_sensitive_text(text)


def run(
    command: list[str],
    *,
    env: dict[str, str] | None = None,
    cwd: Path = ROOT_DIR,
    capture: bool = True,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    print(redact_sensitive_text("+ " + " ".join(shlex.quote(part) for part in command)), flush=True)
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
        print(redact_sensitive_text(completed.stdout), end="")
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


def truthy_arg(args: dict[str, str] | None, name: str) -> bool:
    return str((args or {}).get(name, "")).strip().lower() in {"1", "true", "yes", "on"}


def strict_wait_failure(action: str, args: dict[str, str] | None, statuses: dict[str, str]) -> str | None:
    del action
    if truthy_arg(args, "wait_for_runtime_idle") and statuses.get("runtime_settled") != "true":
        return (
            "runtime idle wait was requested but did not settle"
            f" (runtime_settled={statuses.get('runtime_settled', '<missing>')}, "
            f"summary={statuses.get('runtime_pending_summary', '<missing>')})"
        )

    if not truthy_arg(args, "wait_for_relay_drain"):
        return None

    runtime_only = truthy_arg(args, "relay_drain_runtime_only")
    expected_zero = ["pending_runtime_outbound_count", "pending_group_control_count"]
    if not runtime_only:
        expected_zero.insert(0, "pending_outbound_count")
    for key in expected_zero:
        value = statuses.get(key)
        if value not in (None, "", "0"):
            return f"relay drain wait left {key}={value}"

    if statuses.get("network_syncing") == "true":
        return "relay drain wait ended while network_syncing=true"

    for key in ("pending_relay_publishes", "sqlite_pending_relay_publishes"):
        value = statuses.get(key)
        if value not in (None, ""):
            return f"relay drain wait left {key}={value}"

    return None


def parse_relay_urls(value: str) -> list[str]:
    return [
        entry.strip()
        for entry in re.split(r"[,\n|]", value)
        if entry.strip()
    ]


def wait_for_status_file(path: Path, key: str, timeout_secs: int) -> str:
    return wait_for_status_in_files([path], key, timeout_secs)


def wait_for_status_in_files(paths: list[Path], key: str, timeout_secs: int) -> str:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        for path in paths:
            if path.exists():
                value = parse_status(path.read_text(encoding="utf-8", errors="replace")).get(key)
                if value:
                    return value
        time.sleep(1)
    joined = ", ".join(str(path) for path in paths)
    raise SystemExit(f"Timed out waiting for {key} in {joined}")


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


def simulator_has_active_xcodebuild(udid: str) -> bool:
    try:
        completed = subprocess.run(
            ["pgrep", "-fl", "xcodebuild"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=5,
        )
    except subprocess.TimeoutExpired:
        return True
    return f"id={udid}" in completed.stdout


def quit_idle_ios_simulator_app() -> None:
    if os.environ.get("IRIS_E2E_KEEP_IOS_SIMS", "0") == "1":
        return
    try:
        booted = subprocess.run(
            ["xcrun", "simctl", "list", "devices", "booted"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=10,
        )
    except subprocess.TimeoutExpired:
        print("Timed out checking booted iOS simulators; leaving Simulator app running", flush=True)
        return
    if booted.returncode == 0 and "(Booted)" in booted.stdout:
        return
    try:
        xcodebuild = subprocess.run(
            ["pgrep", "-fl", "xcodebuild"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=5,
        )
    except subprocess.TimeoutExpired:
        print("Timed out checking xcodebuild processes; leaving Simulator app running", flush=True)
        return
    if re.search(r"id=[0-9A-F-]{36}|platform=iOS Simulator|iphonesimulator", xcodebuild.stdout):
        return
    try:
        simulator = subprocess.run(
            ["pgrep", "-x", "Simulator"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
    except subprocess.TimeoutExpired:
        return
    if simulator.returncode != 0:
        return
    print("Quitting idle iOS Simulator app", flush=True)
    try:
        quit_result = subprocess.run(
            ["osascript", "-e", 'tell application "Simulator" to quit'],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
    except subprocess.TimeoutExpired:
        quit_result = subprocess.CompletedProcess([], 124)
    if quit_result.returncode != 0:
        subprocess.run(["pkill", "-x", "Simulator"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)


def shutdown_stale_ios_simulators(keep_names: list[str]) -> None:
    if os.environ.get("IRIS_E2E_CLOSE_STALE_IOS_SIMS", "1") == "0":
        return
    try:
        completed = subprocess.run(
            ["xcrun", "simctl", "list", "devices", "booted"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=10,
        )
    except subprocess.TimeoutExpired:
        print("Timed out checking stale iOS simulators; continuing with existing booted devices", flush=True)
        return
    keep = set(keep_names)
    for line in completed.stdout.splitlines():
        match = re.match(r"^\s*(.+?) \(([0-9A-F-]{36})\) \(Booted\)", line)
        if not match:
            continue
        name, udid = match.groups()
        if name in keep:
            continue
        if simulator_has_active_xcodebuild(udid):
            print(f"Keeping active iOS simulator {udid}")
            continue
        print(f"Shutting down stale iOS simulator {udid}")
        try:
            subprocess.run(
                ["xcrun", "simctl", "shutdown", udid],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=10,
            )
        except subprocess.TimeoutExpired:
            print(f"Timed out shutting down stale iOS simulator {udid}; continuing", flush=True)
    quit_idle_ios_simulator_app()


def discover_android_sdk_dir() -> Path | None:
    value = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    local_properties = ROOT_DIR / "android" / "local.properties"
    if not value and local_properties.exists():
        for line in local_properties.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.startswith("sdk.dir="):
                value = line.split("=", 1)[1].strip()
    if not value:
        default = Path.home() / "Library" / "Android" / "sdk"
        if default.exists():
            value = str(default)
    return Path(value) if value else None


def cargo_target_dir() -> Path:
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            str(ROOT_DIR / "core" / "Cargo.toml"),
            "--no-deps",
            "--format-version",
            "1",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode == 0:
        try:
            return Path(json.loads(completed.stdout)["target_directory"])
        except (KeyError, json.JSONDecodeError):
            pass
    return ROOT_DIR / "target"


def local_relay_binary() -> Path:
    candidates = [
        cargo_target_dir() / "debug" / "local_nostr_relay",
        ROOT_DIR / "target" / "debug" / "local_nostr_relay",
        ROOT_DIR / "core" / "target" / "debug" / "local_nostr_relay",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return candidates[0]
