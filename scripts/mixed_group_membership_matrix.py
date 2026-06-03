#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import time
from pathlib import Path
from typing import Any

from mixed_offline_restart_recovery import (
    connected_android_serials,
    free_tcp_port,
    harness,
    relay_uses_localhost,
    send_chat,
    short_stamp,
    split_list,
    stamp,
    wait_chat,
    write_json,
)
from mobile_scenario import ROOT_DIR, Scenario, run


DEFAULT_SIMULATORS = ("Iris Chat iPhone", "Iris Chat iPhone 2")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android group membership E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching local relay URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local message-server TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
    parser.add_argument("--avd", help="Android AVD name.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First value is used.")
    parser.add_argument("--simulators", help="Two iOS simulator names, comma separated.")
    parser.add_argument("--simulator-a", default=DEFAULT_SIMULATORS[0], help="Alice iOS simulator.")
    parser.add_argument("--simulator-b", default=DEFAULT_SIMULATORS[1], help="Carol iOS simulator.")
    parser.add_argument("--udids", help="Two iOS UDIDs, comma/space separated. Overrides simulator names.")
    parser.add_argument("--udid-a", help="Alice iOS UDID.")
    parser.add_argument("--udid-b", help="Carol iOS UDID.")
    return parser.parse_args()


def split_name_list(value: str | None) -> list[str]:
    if not value:
        return []
    return [part.strip() for part in re.split(r"[,|]+", value) if part.strip()]


def unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value and value not in seen:
            seen.add(value)
            result.append(value)
    return result


def discover_avds(limit: int) -> list[str]:
    completed = run([str(ROOT_DIR / "scripts" / "run_android_emulators.sh"), "--list"])
    avds = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if len(avds) < limit:
        raise SystemExit(
            f"Need {limit} Android AVD or connected Android device for mixed F08; found {len(avds)} AVDs. "
            "Set --serial, --avd, IRIS_ANDROID_E2E_SERIALS, or IRIS_ANDROID_E2E_AVDS."
        )
    return avds[:limit]


def select_android_target(args: argparse.Namespace) -> dict[str, str]:
    serials = []
    if args.serial:
        serials.append(args.serial)
    serials.extend(split_list(args.serials))
    serials.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS")))
    serials = unique(serials + connected_android_serials())
    if serials:
        return {"serial": serials[0]}

    avds = []
    if args.avd:
        avds.append(args.avd)
    avds.extend(split_list(args.avds))
    avds.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_AVDS")))
    if avds:
        return {"avd": unique(avds)[0]}

    return {"avd": discover_avds(1)[0]}


def select_ios_entries(args: argparse.Namespace) -> tuple[dict[str, str], dict[str, str]]:
    udids = split_list(args.udids)
    if args.udid_a:
        udids.insert(0, args.udid_a)
    if args.udid_b:
        udids.insert(1 if udids else 0, args.udid_b)
    if len(udids) >= 2:
        return {"udid": udids[0]}, {"udid": udids[1]}

    simulators = split_name_list(args.simulators)
    if len(simulators) < 2:
        simulators = [args.simulator_a, args.simulator_b]
    if len(simulators) < 2:
        raise SystemExit("Need two iOS simulator names or UDIDs for mixed F08.")
    return {"simulator": simulators[0]}, {"simulator": simulators[1]}


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    port = args.relay_port or free_tcp_port()
    ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
    android_target = select_android_target(args)
    alice_ios, carol_ios = select_ios_entries(args)
    default_android_url = f"ws://127.0.0.1:{port}" if "serial" in android_target else f"ws://10.0.2.2:{port}"
    android_url = args.android_relay_url or args.relay_url or default_android_url

    devices: list[dict[str, Any]] = [
        {
            "id": "alice1",
            "platform": "ios",
            "run_id": "alice1",
            "user": "alice",
            "display_name": "Alice",
            "reset": True,
            "relay_drain_timeout_secs": 240,
            **alice_ios,
        },
        {
            "id": "bob1",
            "platform": "android",
            "run_id": "bob1",
            "user": "bob",
            "display_name": "Bob",
            "reset": True,
            "relay_drain_timeout_secs": 240,
            **android_target,
        },
        {
            "id": "carol1",
            "platform": "ios",
            "run_id": "carol1",
            "user": "carol",
            "display_name": "Carol",
            "reset": True,
            "relay_drain_timeout_secs": 240,
            **carol_ios,
        },
    ]

    config = {
        "name": f"mixed-group-membership-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": {
            "port": port,
            "label": f"iris.mixed-group-membership.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-group-membership-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        },
        "ios": {"build": not args.skip_build},
        "android": {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": relay_uses_localhost(android_url),
        },
        "open_apps": True,
        "devices": devices,
        "groups": [],
    }
    config_path = artifact_dir / "mixed-group-membership-config.json"
    write_json(config_path, config)
    return config_path


def group_id_from_chat_id(chat_id: str) -> str:
    return chat_id.removeprefix("group:")


def wait_member_count(scenario: Scenario, device_id: str, chat_id: str, count: int) -> str:
    return harness(
        scenario,
        device_id,
        "wait_for_group_member_count_from_args",
        chat_id=chat_id,
        member_count=str(count),
        timeout_secs="240",
    ).get("member_count", "")


def create_initial_group(scenario: Scenario, group_name: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=group_name,
        member_inputs=scenario.state["devices"]["carol1"]["owner_npub"],
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    group = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-carol-membership"] = group
    scenario.save_state()
    harness(scenario, "carol1", "wait_for_group_chat_from_args", chat_id=group["chat_id"], timeout_secs="300")
    return group


def add_bob(scenario: Scenario, group: dict[str, str]) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "add_group_members_from_args",
        group_id=group["group_id"],
        chat_id=group["chat_id"],
        member_inputs=scenario.state["devices"]["bob1"]["owner_npub"],
        expected_member_count="3",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    counts = {"alice1": statuses.get("member_count", "")}
    for device_id in ("bob1", "carol1"):
        counts[device_id] = wait_member_count(scenario, device_id, group["chat_id"], 3)
    return counts


def remove_bob(scenario: Scenario, group: dict[str, str]) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "remove_group_member_from_args",
        group_id=group["group_id"],
        chat_id=group["chat_id"],
        member_input=scenario.state["devices"]["bob1"]["owner_hex"],
        expected_member_count="2",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    counts = {"alice1": statuses.get("member_count", "")}
    counts["carol1"] = wait_member_count(scenario, "carol1", group["chat_id"], 2)
    counts["bob1"] = wait_member_count(scenario, "bob1", group["chat_id"], 2)
    return counts


def wait_group_message_on_devices(
    scenario: Scenario,
    sender: str,
    chat_id: str,
    message: str,
    devices: tuple[str, ...],
) -> dict[str, str]:
    counts: dict[str, str] = {}
    for device_id in devices:
        direction = "outgoing" if device_id == sender else "incoming"
        counts[device_id] = wait_chat(
            scenario,
            device_id,
            chat_id,
            message,
            direction=direction,
            expected_count=1,
        ).get("matching_count", "")
    return counts


def expect_removed_send_rejected(scenario: Scenario, group: dict[str, str], message: str) -> dict[str, str]:
    return harness(
        scenario,
        "bob1",
        "expect_send_rejected_from_args",
        chat_id=group["chat_id"],
        message=message,
    )


def assert_message_absent(
    scenario: Scenario,
    device_id: str,
    group: dict[str, str],
    message: str,
    *,
    timeout_ms: int = 30_000,
) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "assert_message_absent_from_args",
        chat_id=group["chat_id"],
        message=message,
        direction="any",
        timeout_ms=str(timeout_ms),
    )


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = short_stamp()
    group = create_initial_group(scenario, f"Mixed Membership {flow_stamp}")
    chat_id = group["chat_id"]

    initial_member_counts = {
        "alice1": wait_member_count(scenario, "alice1", chat_id, 2),
        "carol1": wait_member_count(scenario, "carol1", chat_id, 2),
    }

    initial_message = f"mixed-membership-initial-carol-{flow_stamp}"
    send_chat(scenario, "carol1", chat_id, initial_message)
    initial_counts = wait_group_message_on_devices(
        scenario,
        "carol1",
        chat_id,
        initial_message,
        ("alice1", "carol1"),
    )

    add_member_counts = add_bob(scenario, group)
    added_message = f"mixed-membership-added-bob-{flow_stamp}"
    send_chat(scenario, "bob1", chat_id, added_message)
    added_counts = wait_group_message_on_devices(
        scenario,
        "bob1",
        chat_id,
        added_message,
        ("alice1", "bob1", "carol1"),
    )

    remove_member_counts = remove_bob(scenario, group)
    rejected_message = f"mixed-membership-rejected-bob-{flow_stamp}"
    rejected_removed_send = expect_removed_send_rejected(scenario, group, rejected_message)
    rejected_absent = {
        "alice1": assert_message_absent(scenario, "alice1", group, rejected_message),
        "carol1": assert_message_absent(scenario, "carol1", group, rejected_message),
    }

    final_message = f"mixed-membership-after-remove-alice-{flow_stamp}"
    send_chat(scenario, "alice1", chat_id, final_message)
    final_counts = wait_group_message_on_devices(
        scenario,
        "alice1",
        chat_id,
        final_message,
        ("alice1", "carol1"),
    )
    removed_bob_absent_final = assert_message_absent(scenario, "bob1", group, final_message)

    for device_id in ("alice1", "bob1", "carol1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    carol = scenario.state["devices"]["carol1"]
    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "initial_member_counts": initial_member_counts,
        "initial_message": initial_message,
        "initial_counts": initial_counts,
        "add_member_counts": add_member_counts,
        "added_message": added_message,
        "added_counts": added_counts,
        "remove_member_counts": remove_member_counts,
        "rejected_message": rejected_message,
        "rejected_removed_send": rejected_removed_send,
        "rejected_absent": rejected_absent,
        "final_message": final_message,
        "final_counts": final_counts,
        "removed_bob_absent_final": removed_bob_absent_final,
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
            "carol1": {
                "platform": carol["platform"],
                "serial": carol.get("serial", ""),
                "udid": carol.get("udid", ""),
            },
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "mixed-group-membership-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-group-membership-{run_id}")).resolve()
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
        write_json(artifact_dir / "mixed-group-membership-summary.json", failure)
        raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
