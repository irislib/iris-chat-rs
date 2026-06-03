#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from mobile_scenario import ANDROID_APP_PACKAGE, ROOT_DIR, Scenario


def run(
    command: list[str],
    *,
    cwd: Path = ROOT_DIR,
    capture: bool = True,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    print("+ " + " ".join(command), flush=True)
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


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def stamp() -> str:
    return time.strftime("%Y%m%dT%H%M%S")


def action_stamp() -> str:
    return time.strftime("%H%M%S")


def discover_avds(limit: int) -> list[str]:
    completed = run([str(ROOT_DIR / "scripts" / "run_android_emulators.sh"), "--list"])
    avds = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if len(avds) < limit:
        raise SystemExit(
            f"Need {limit} Android AVDs or serials for F06; found {len(avds)}. "
            "Set IRIS_ANDROID_E2E_AVDS or IRIS_ANDROID_E2E_SERIALS."
        )
    return avds[:limit]


def split_env_list(value: str | None) -> list[str]:
    return [part for part in (value or "").split() if part]


def select_android_targets(serials: list[str], avds: list[str]) -> list[dict[str, str]]:
    selected: list[dict[str, str]] = [{"serial": serial} for serial in serials[:2]]
    needed = 2 - len(selected)
    if needed > 0:
        available_avds = list(avds)
        if len(available_avds) < needed:
            discovered = discover_avds(needed)
            available_avds.extend(avd for avd in discovered if avd not in available_avds)
        selected.extend({"avd": avd} for avd in available_avds[:needed])
    if len(selected) < 2:
        raise SystemExit(
            "Need two Android targets for F06. "
            "Set IRIS_ANDROID_E2E_SERIALS, IRIS_ANDROID_E2E_AVDS, --serials, or --avds."
        )
    return selected[:2]


def relay_uses_localhost(relay_url: str) -> bool:
    return "://127.0.0.1:" in relay_url or "://localhost:" in relay_url


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Android local message-server offline/restart recovery E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed APKs. Requires matching local relay URL.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave emulators running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local relay TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="Relay URL compiled into the Android debug app.")
    parser.add_argument("--serials", help="One or two adb serials, space-separated. Can be combined with --avds.")
    parser.add_argument("--avds", help="One or two AVD names, space-separated. Can be combined with --serials.")
    return parser.parse_args()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    port = args.relay_port or free_tcp_port()
    serials = split_env_list(args.serials) or split_env_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS"))
    avds = split_env_list(args.avds) or split_env_list(os.environ.get("IRIS_ANDROID_E2E_AVDS"))
    targets = select_android_targets(serials, avds)
    has_serial = any("serial" in target for target in targets)
    relay_url = args.relay_url or (f"ws://127.0.0.1:{port}" if has_serial else f"ws://10.0.2.2:{port}")
    devices: list[dict[str, Any]] = []
    for index, (device_id, user, display_name) in enumerate(
        [
            ("alice1", "alice", "Alice"),
            ("bob1", "bob", "Bob"),
        ]
    ):
        device: dict[str, Any] = {
            "id": device_id,
            "platform": "android",
            "run_id": device_id,
            "user": user,
            "display_name": display_name,
            "reset": True,
            "relay_drain_timeout_secs": 240,
        }
        device.update(targets[index])
        devices.append(device)

    config = {
        "name": f"android-offline-restart-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": {
            "port": port,
            "label": f"iris.android-offline-restart.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"android-offline-restart-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": relay_url,
            "url": relay_url,
        },
        "android": {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": relay_uses_localhost(relay_url),
        },
        "open_apps": True,
        "devices": devices,
        "groups": [],
    }
    config_path = artifact_dir / "android-offline-restart-config.json"
    write_json(config_path, config)
    return config_path


def restart_android_app(scenario: Scenario, device_id: str) -> None:
    device = scenario.state["devices"][device_id]
    package_name = device.get("app_package") or ANDROID_APP_PACKAGE
    adb = str(scenario.adb())
    serial = device["serial"]
    run([adb, "-s", serial, "shell", "am", "force-stop", package_name], env=scenario.scenario_env(), check=False)
    time.sleep(1)
    run(
        [
            adb,
            "-s",
            serial,
            "shell",
            "monkey",
            "-p",
            package_name,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ],
        env=scenario.scenario_env(),
        check=False,
    )


def harness(scenario: Scenario, device_id: str, action: str, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, args=args)


def send_peer(
    scenario: Scenario,
    sender: str,
    peer: str,
    message: str,
    *,
    wait_for_delivery: bool = True,
    wait_for_relay_drain: bool = True,
) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        peer_input=scenario.state["devices"][peer]["owner_npub"],
        message=message,
        wait_for_delivery=str(wait_for_delivery).lower(),
        wait_for_relay_drain=str(wait_for_relay_drain).lower(),
        relay_drain_timeout_secs="240",
    )


def wait_peer(
    scenario: Scenario,
    receiver: str,
    peer: str,
    message: str,
    *,
    direction: str = "incoming",
    expected_count: int | None = None,
) -> dict[str, str]:
    args = {
        "peer_input": scenario.state["devices"][peer]["owner_npub"],
        "message": message,
        "direction": direction,
        "timeout_secs": "240",
    }
    if expected_count is not None:
        args["expected_count"] = str(expected_count)
    return harness(scenario, receiver, "wait_for_message_from_args", **args)


def send_chat(
    scenario: Scenario,
    sender: str,
    chat_id: str,
    message: str,
    *,
    wait_for_delivery: bool = True,
    wait_for_relay_drain: bool = True,
) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        chat_id=chat_id,
        message=message,
        wait_for_delivery=str(wait_for_delivery).lower(),
        wait_for_relay_drain=str(wait_for_relay_drain).lower(),
        relay_drain_timeout_secs="240",
    )


def wait_chat(
    scenario: Scenario,
    receiver: str,
    chat_id: str,
    message: str,
    *,
    direction: str = "incoming",
    expected_count: int | None = None,
) -> dict[str, str]:
    args = {
        "chat_id": chat_id,
        "message": message,
        "direction": direction,
        "timeout_secs": "240",
    }
    if expected_count is not None:
        args["expected_count"] = str(expected_count)
    return harness(scenario, receiver, "wait_for_message_from_args", **args)


def create_alice_bob_group(scenario: Scenario, group_name: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=group_name,
        member_inputs=scenario.state["devices"]["bob1"]["owner_npub"],
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    group_state = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-bob"] = group_state
    scenario.save_state()
    harness(scenario, "bob1", "wait_for_group_chat_from_args", chat_id=group_state["chat_id"])
    return group_state


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = action_stamp()
    messages = {
        "alice_to_bob_warmup": f"android-offline-warmup-a2b-{flow_stamp}",
        "bob_to_alice_warmup": f"android-offline-warmup-b2a-{flow_stamp}",
        "group_warmup": f"android-offline-warmup-group-{flow_stamp}",
        "alice_to_bob": f"android-offline-a2b-{flow_stamp}",
        "bob_to_alice": f"android-offline-b2a-{flow_stamp}",
        "alice_group": f"android-offline-alice-group-{flow_stamp}",
        "bob_group": f"android-offline-bob-group-{flow_stamp}",
    }

    send_peer(scenario, "alice1", "bob1", messages["alice_to_bob_warmup"])
    wait_peer(scenario, "bob1", "alice1", messages["alice_to_bob_warmup"])
    send_peer(scenario, "bob1", "alice1", messages["bob_to_alice_warmup"])
    wait_peer(scenario, "alice1", "bob1", messages["bob_to_alice_warmup"])

    group = create_alice_bob_group(
        scenario,
        f"Android Offline Recovery {artifact_dir.name[-6:]}",
    )
    chat_id = group["chat_id"]
    send_chat(scenario, "alice1", chat_id, messages["group_warmup"])
    wait_chat(scenario, "bob1", chat_id, messages["group_warmup"])

    scenario.begin_fault()
    queued = {
        "alice_to_bob": send_peer(
            scenario,
            "alice1",
            "bob1",
            messages["alice_to_bob"],
            wait_for_delivery=False,
            wait_for_relay_drain=False,
        ).get("delivery", ""),
        "bob_to_alice": send_peer(
            scenario,
            "bob1",
            "alice1",
            messages["bob_to_alice"],
            wait_for_delivery=False,
            wait_for_relay_drain=False,
        ).get("delivery", ""),
        "alice_group": send_chat(
            scenario,
            "alice1",
            chat_id,
            messages["alice_group"],
            wait_for_delivery=False,
            wait_for_relay_drain=False,
        ).get("delivery", ""),
        "bob_group": send_chat(
            scenario,
            "bob1",
            chat_id,
            messages["bob_group"],
            wait_for_delivery=False,
            wait_for_relay_drain=False,
        ).get("delivery", ""),
    }

    for device_id in ("alice1", "bob1"):
        restart_android_app(scenario, device_id)

    scenario.start_relay()
    for device_id in ("alice1", "bob1", "alice1", "bob1"):
        harness(
            scenario,
            device_id,
            "report_logged_in_identity",
            wait_for_relay_drain="true",
            relay_drain_timeout_secs="240",
            relay_drain_runtime_only="true",
        )

    duplicate_counts = {
        "bob_direct_a2b": wait_peer(
            scenario,
            "bob1",
            "alice1",
            messages["alice_to_bob"],
            expected_count=1,
        ).get("matching_count", ""),
        "alice_direct_b2a": wait_peer(
            scenario,
            "alice1",
            "bob1",
            messages["bob_to_alice"],
            expected_count=1,
        ).get("matching_count", ""),
        "bob_group_from_alice": wait_chat(
            scenario,
            "bob1",
            chat_id,
            messages["alice_group"],
            expected_count=1,
        ).get("matching_count", ""),
        "alice_group_from_bob": wait_chat(
            scenario,
            "alice1",
            chat_id,
            messages["bob_group"],
            expected_count=1,
        ).get("matching_count", ""),
    }

    for device_id in ("alice1", "bob1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "offline_messages": {
            "alice_to_bob": messages["alice_to_bob"],
            "bob_to_alice": messages["bob_to_alice"],
            "alice_group": messages["alice_group"],
            "bob_group": messages["bob_group"],
        },
        "queued_delivery": queued,
        "duplicate_counts": duplicate_counts,
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "android-offline-restart-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-android-offline-restart-{run_id}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        scenario.setup()
        result = run_flow(scenario, artifact_dir)
        print(json.dumps(result, indent=2, sort_keys=True))
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
