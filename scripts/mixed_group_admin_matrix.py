#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import socket
import subprocess
import time
from pathlib import Path
from typing import Any

from mobile_scenario import ROOT_DIR, Scenario, discover_android_sdk_dir, run


DEFAULT_SIMULATOR = "Iris Chat iPhone"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android group admin E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--alice-platform", choices=("ios", "android"), default="ios")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching local relay URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local relay TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="Relay URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="Relay URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
    parser.add_argument("--avd", help="Android AVD name.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First value is used.")
    parser.add_argument("--simulator", default=DEFAULT_SIMULATOR, help=f"iOS simulator name. Default: {DEFAULT_SIMULATOR}.")
    parser.add_argument("--udid", help="iOS simulator/device UDID.")
    return parser.parse_args()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def stamp() -> str:
    return time.strftime("%Y%m%dT%H%M%S")


def short_stamp() -> str:
    return time.strftime("%H%M%S")


def split_list(value: str | None) -> list[str]:
    if not value:
        return []
    return [part for part in re.split(r"[\s,|]+", value.strip()) if part]


def relay_uses_localhost(relay_url: str) -> bool:
    return "://127.0.0.1:" in relay_url or "://localhost:" in relay_url


def discover_avds(limit: int) -> list[str]:
    completed = run([str(ROOT_DIR / "scripts" / "run_android_emulators.sh"), "--list"])
    avds = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if len(avds) < limit:
        raise SystemExit(
            f"Need {limit} Android AVD or connected Android device for mixed F09; found {len(avds)} AVDs. "
            "Set --serial, --avd, IRIS_ANDROID_E2E_SERIALS, or IRIS_ANDROID_E2E_AVDS."
        )
    return avds[:limit]


def connected_android_serials() -> list[str]:
    sdk_dir = discover_android_sdk_dir()
    if sdk_dir is None:
        return []
    adb = sdk_dir / "platform-tools" / "adb"
    if not adb.exists():
        return []
    completed = subprocess.run(
        [str(adb), "devices", "-l"],
        cwd=str(ROOT_DIR),
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        return []
    serials: list[str] = []
    for line in completed.stdout.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 2 and parts[1] == "device" and not parts[0].startswith("emulator-"):
            serials.append(parts[0])
    return serials


def select_android_target(args: argparse.Namespace) -> dict[str, str]:
    serials = []
    if args.serial:
        serials.append(args.serial)
    serials.extend(split_list(args.serials))
    serials.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS")))
    if serials:
        return {"serial": serials[0]}

    avds = []
    if args.avd:
        avds.append(args.avd)
    avds.extend(split_list(args.avds))
    avds.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_AVDS")))
    if avds:
        return {"avd": avds[0]}

    connected = connected_android_serials()
    if connected:
        return {"serial": connected[0]}

    return {"avd": discover_avds(1)[0]}


def ios_device_entry(device_id: str, user: str, display_name: str, args: argparse.Namespace) -> dict[str, Any]:
    device: dict[str, Any] = {
        "id": device_id,
        "platform": "ios",
        "run_id": device_id,
        "user": user,
        "display_name": display_name,
        "reset": True,
        "relay_drain_timeout_secs": 240,
    }
    if args.udid:
        device["udid"] = args.udid
    else:
        device["simulator"] = args.simulator
    return device


def android_device_entry(
    device_id: str,
    user: str,
    display_name: str,
    target: dict[str, str],
) -> dict[str, Any]:
    device: dict[str, Any] = {
        "id": device_id,
        "platform": "android",
        "run_id": device_id,
        "user": user,
        "display_name": display_name,
        "reset": True,
        "relay_drain_timeout_secs": 240,
    }
    device.update(target)
    return device


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    port = args.relay_port or free_tcp_port()
    ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
    android_target = select_android_target(args)
    default_android_url = f"ws://127.0.0.1:{port}" if "serial" in android_target else f"ws://10.0.2.2:{port}"
    android_url = args.android_relay_url or args.relay_url or default_android_url

    ios_entry = ios_device_entry(
        "alice1" if args.alice_platform == "ios" else "bob1",
        "alice" if args.alice_platform == "ios" else "bob",
        "Alice" if args.alice_platform == "ios" else "Bob",
        args,
    )
    android_entry = android_device_entry(
        "alice1" if args.alice_platform == "android" else "bob1",
        "alice" if args.alice_platform == "android" else "bob",
        "Alice" if args.alice_platform == "android" else "Bob",
        android_target,
    )

    config = {
        "name": f"mixed-group-admin-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": {
            "port": port,
            "label": f"iris.mixed-group-admin.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-group-admin-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        },
        "ios": {
            "build": not args.skip_build,
        },
        "android": {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": relay_uses_localhost(android_url),
        },
        "open_apps": True,
        "devices": [ios_entry, android_entry],
        "groups": [],
    }
    config_path = artifact_dir / "mixed-group-admin-config.json"
    write_json(config_path, config)
    return config_path


def harness(scenario: Scenario, device_id: str, action: str, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, args=args)


def group_id_from_chat_id(chat_id: str) -> str:
    return chat_id.removeprefix("group:")


def create_group(scenario: Scenario, group_name: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=group_name,
        member_inputs=scenario.state["devices"]["bob1"]["owner_npub"],
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    group = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-bob-admin"] = group
    scenario.save_state()
    harness(scenario, "bob1", "wait_for_group_chat_from_args", chat_id=group["chat_id"], timeout_secs="300")
    return group


def wait_group_admin(
    scenario: Scenario,
    device_id: str,
    group_id: str,
    member_device_id: str,
    is_admin: bool,
) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "wait_for_group_admin_from_args",
        group_id=group_id,
        member_input=scenario.state["devices"][member_device_id]["owner_hex"],
        is_admin=str(is_admin).lower(),
        timeout_secs="240",
    )


def rename_group(
    scenario: Scenario,
    device_id: str,
    group: dict[str, str],
    group_name: str,
) -> dict[str, str]:
    statuses = harness(
        scenario,
        device_id,
        "update_group_name_from_args",
        group_id=group["group_id"],
        chat_id=group["chat_id"],
        group_name=group_name,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    group["name"] = statuses["group_name"]
    return statuses


def expect_rename_rejected(
    scenario: Scenario,
    device_id: str,
    group: dict[str, str],
    rejected_name: str,
) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "expect_group_name_update_rejected_from_args",
        group_id=group["group_id"],
        chat_id=group["chat_id"],
        group_name=rejected_name,
        expected_group_name=group["name"],
        timeout_secs="20",
    )


def wait_group_name(scenario: Scenario, device_id: str, group: dict[str, str]) -> str:
    return harness(
        scenario,
        device_id,
        "wait_for_group_name_from_args",
        chat_id=group["chat_id"],
        group_name=group["name"],
        timeout_secs="240",
    ).get("group_name", "")


def send_chat(scenario: Scenario, sender: str, chat_id: str, message: str) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        chat_id=chat_id,
        message=message,
        wait_for_delivery="true",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )


def wait_chat(
    scenario: Scenario,
    receiver: str,
    chat_id: str,
    message: str,
    *,
    direction: str = "incoming",
) -> dict[str, str]:
    return harness(
        scenario,
        receiver,
        "wait_for_message_from_args",
        chat_id=chat_id,
        message=message,
        direction=direction,
        expected_count="1",
        timeout_secs="240",
    )


def wait_group_message_on_members(
    scenario: Scenario,
    sender: str,
    chat_id: str,
    message: str,
) -> dict[str, str]:
    counts: dict[str, str] = {}
    for device_id in ("alice1", "bob1"):
        direction = "outgoing" if device_id == sender else "incoming"
        counts[device_id] = wait_chat(scenario, device_id, chat_id, message, direction=direction).get("matching_count", "")
    return counts


def set_bob_admin(scenario: Scenario, group_id: str, is_admin: bool) -> dict[str, str]:
    return harness(
        scenario,
        "alice1",
        "set_group_admin_from_args",
        group_id=group_id,
        member_input=scenario.state["devices"]["bob1"]["owner_hex"],
        is_admin=str(is_admin).lower(),
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = short_stamp()
    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    group = create_group(scenario, f"Mixed Admin {alice['platform']}-{bob['platform']} {flow_stamp}")
    chat_id = group["chat_id"]
    group_id = group["group_id"]

    admin_observations: dict[str, dict[str, str]] = {
        "alice_sees_alice_admin_initial": wait_group_admin(scenario, "alice1", group_id, "alice1", True),
        "bob_sees_bob_non_admin_initial": wait_group_admin(scenario, "bob1", group_id, "bob1", False),
    }

    baseline_messages = {
        "alice": f"mixed-admin-baseline-alice-{flow_stamp}",
        "bob": f"mixed-admin-baseline-bob-{flow_stamp}",
    }
    send_chat(scenario, "alice1", chat_id, baseline_messages["alice"])
    baseline_counts = {
        "alice": wait_group_message_on_members(scenario, "alice1", chat_id, baseline_messages["alice"]),
    }
    send_chat(scenario, "bob1", chat_id, baseline_messages["bob"])
    baseline_counts["bob"] = wait_group_message_on_members(scenario, "bob1", chat_id, baseline_messages["bob"])

    rejected_before = expect_rename_rejected(
        scenario,
        "bob1",
        group,
        f"Rejected Bob Rename Before Admin {flow_stamp}",
    )
    names_after_rejected_before = {
        "alice1": wait_group_name(scenario, "alice1", group),
        "bob1": wait_group_name(scenario, "bob1", group),
    }

    promote = set_bob_admin(scenario, group_id, True)
    admin_observations["alice_sees_bob_admin_after_promote"] = wait_group_admin(scenario, "alice1", group_id, "bob1", True)
    admin_observations["bob_sees_bob_admin_after_promote"] = wait_group_admin(scenario, "bob1", group_id, "bob1", True)

    promoted_name = f"Bob Admin Rename {flow_stamp}"
    rename_promoted = rename_group(scenario, "bob1", group, promoted_name)
    names_after_promoted_rename = {
        "alice1": wait_group_name(scenario, "alice1", group),
        "bob1": wait_group_name(scenario, "bob1", group),
    }

    demote = set_bob_admin(scenario, group_id, False)
    admin_observations["alice_sees_bob_non_admin_after_demote"] = wait_group_admin(scenario, "alice1", group_id, "bob1", False)
    admin_observations["bob_sees_bob_non_admin_after_demote"] = wait_group_admin(scenario, "bob1", group_id, "bob1", False)

    rejected_after = expect_rename_rejected(
        scenario,
        "bob1",
        group,
        f"Rejected Bob Rename After Demote {flow_stamp}",
    )
    names_after_rejected_after = {
        "alice1": wait_group_name(scenario, "alice1", group),
        "bob1": wait_group_name(scenario, "bob1", group),
    }

    final_name = f"Alice Final Admin Rename {flow_stamp}"
    rename_final = rename_group(scenario, "alice1", group, final_name)
    names_after_final_rename = {
        "alice1": wait_group_name(scenario, "alice1", group),
        "bob1": wait_group_name(scenario, "bob1", group),
    }

    final_message = f"mixed-admin-final-bob-{flow_stamp}"
    send_chat(scenario, "bob1", chat_id, final_message)
    final_counts = wait_group_message_on_members(scenario, "bob1", chat_id, final_message)

    for device_id in ("alice1", "bob1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "group_chat_id": chat_id,
        "group_id": group_id,
        "admin_observations": admin_observations,
        "baseline_messages": baseline_messages,
        "baseline_counts": baseline_counts,
        "rejected_before_promote": rejected_before,
        "names_after_rejected_before": names_after_rejected_before,
        "promote": promote,
        "rename_promoted": rename_promoted,
        "names_after_promoted_rename": names_after_promoted_rename,
        "demote": demote,
        "rejected_after_demote": rejected_after,
        "names_after_rejected_after": names_after_rejected_after,
        "rename_final": rename_final,
        "names_after_final_rename": names_after_final_rename,
        "final_message": final_message,
        "final_counts": final_counts,
        "devices": {
            "alice1": {
                "platform": alice["platform"],
                "serial": alice.get("serial", ""),
                "udid": alice.get("udid", ""),
            },
            "bob1": {
                "platform": bob["platform"],
                "serial": bob.get("serial", ""),
                "udid": bob.get("udid", ""),
            },
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "mixed-group-admin-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    suffix = f"{run_id}-{args.alice_platform}-alice"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-group-admin-{suffix}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        scenario.setup()
        result = run_flow(scenario, artifact_dir)
        print(json.dumps(result, indent=2, sort_keys=True))
    except BaseException as error:
        failure = {
            "status": "failed",
            "artifact_dir": str(artifact_dir),
            "error": str(error),
        }
        write_json(artifact_dir / "mixed-group-admin-summary.json", failure)
        raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
