#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from mixed_offline_restart_recovery import (
    connected_android_serials,
    discover_avds,
    drain_after_restart,
    free_tcp_port,
    harness,
    relay_uses_localhost,
    restart_app,
    send_chat,
    send_peer,
    short_stamp,
    split_list,
    stamp,
    wait_chat,
    wait_peer,
    write_json,
)
from mobile_scenario import Scenario


DEFAULT_SIMULATOR = "Iris Chat iPhone"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android pre-publish restart recovery E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--case", choices=("direct", "group", "both"), default="both", help="Recovery case to run. Default: both.")
    parser.add_argument("--alice-platform", choices=("ios", "android"), default="android")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching local message server URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local message-server TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
    parser.add_argument("--avd", help="Android AVD name.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First value is used.")
    parser.add_argument("--simulator", default=DEFAULT_SIMULATOR, help=f"iOS simulator name. Default: {DEFAULT_SIMULATOR}.")
    parser.add_argument("--udid", help="iOS simulator/device UDID.")
    return parser.parse_args()


def unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value and value not in seen:
            seen.add(value)
            result.append(value)
    return result


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
        "name": f"mixed-publish-restart-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": {
            "port": port,
            "label": f"iris.mixed-publish-restart.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-publish-restart-{artifact_dir.name}",
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
    config_path = artifact_dir / "mixed-publish-restart-config.json"
    write_json(config_path, config)
    return config_path


def wait_direct_both_sides(
    scenario: Scenario,
    sender: str,
    receiver: str,
    message: str,
) -> dict[str, str]:
    return {
        sender: wait_peer(scenario, sender, receiver, message, direction="outgoing", expected_count=1).get("matching_count", ""),
        receiver: wait_peer(scenario, receiver, sender, message, direction="incoming", expected_count=1).get("matching_count", ""),
    }


def wait_group_both_sides(
    scenario: Scenario,
    sender: str,
    chat_id: str,
    message: str,
) -> dict[str, str]:
    receiver = "bob1" if sender == "alice1" else "alice1"
    return {
        sender: wait_chat(scenario, sender, chat_id, message, direction="outgoing", expected_count=1).get("matching_count", ""),
        receiver: wait_chat(scenario, receiver, chat_id, message, direction="incoming", expected_count=1).get("matching_count", ""),
    }


def wait_member_count(scenario: Scenario, device_id: str, chat_id: str, count: int) -> str:
    return harness(
        scenario,
        device_id,
        "wait_for_group_member_count_from_args",
        chat_id=chat_id,
        member_count=str(count),
        timeout_secs="240",
    ).get("member_count", "")


def warm_direct_chat(scenario: Scenario, flow_stamp: str) -> dict[str, Any]:
    messages = {
        "alice_to_bob": f"publish-restart-warmup-a2b-{flow_stamp}",
        "bob_to_alice": f"publish-restart-warmup-b2a-{flow_stamp}",
    }
    send_peer(scenario, "alice1", "bob1", messages["alice_to_bob"])
    send_peer(scenario, "bob1", "alice1", messages["bob_to_alice"])
    return {
        "messages": messages,
        "counts": {
            "alice_to_bob": wait_direct_both_sides(scenario, "alice1", "bob1", messages["alice_to_bob"]),
            "bob_to_alice": wait_direct_both_sides(scenario, "bob1", "alice1", messages["bob_to_alice"]),
        },
    }


def run_direct_preconfirm_restart(scenario: Scenario, flow_stamp: str) -> dict[str, Any]:
    message = f"publish-restart-direct-a2b-{flow_stamp}"
    reply = f"publish-restart-direct-reply-{flow_stamp}"

    scenario.begin_fault()
    queued_delivery = send_peer(
        scenario,
        "alice1",
        "bob1",
        message,
        wait_for_delivery=False,
        wait_for_relay_drain=False,
    ).get("delivery", "")
    outgoing_before_restart = wait_peer(
        scenario,
        "alice1",
        "bob1",
        message,
        direction="outgoing",
        expected_count=1,
    ).get("matching_count", "")

    restart_app(scenario, "alice1", wait_for_drain=False)
    scenario.start_relay()
    for device_id in ("alice1", "bob1"):
        drain_after_restart(scenario, device_id)

    send_peer(scenario, "bob1", "alice1", reply)
    return {
        "messages": {"sent_before_restart": message, "reply_after_recovery": reply},
        "queued_delivery": queued_delivery,
        "outgoing_before_restart": outgoing_before_restart,
        "counts_after_recovery": wait_direct_both_sides(scenario, "alice1", "bob1", message),
        "reply_counts": wait_direct_both_sides(scenario, "bob1", "alice1", reply),
    }


def run_group_create_preobserve_restart(scenario: Scenario, flow_stamp: str) -> dict[str, Any]:
    group_name = f"Publish Restart Group {flow_stamp}"
    alice_message = f"publish-restart-group-alice-{flow_stamp}"
    bob_message = f"publish-restart-group-bob-{flow_stamp}"

    scenario.begin_fault()
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=group_name,
        member_inputs=scenario.state["devices"]["bob1"]["owner_npub"],
        wait_for_relay_drain="false",
        relay_drain_timeout_secs="240",
    )
    group = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["publish-restart"] = group
    scenario.save_state()

    restart_app(scenario, "alice1", wait_for_drain=False)
    scenario.start_relay()
    for device_id in ("alice1", "bob1"):
        drain_after_restart(scenario, device_id)

    harness(
        scenario,
        "bob1",
        "wait_for_group_chat_from_args",
        chat_id=group["chat_id"],
        timeout_secs="300",
    )
    member_counts = {
        "alice1": wait_member_count(scenario, "alice1", group["chat_id"], 2),
        "bob1": wait_member_count(scenario, "bob1", group["chat_id"], 2),
    }

    send_chat(scenario, "alice1", group["chat_id"], alice_message)
    send_chat(scenario, "bob1", group["chat_id"], bob_message)
    return {
        "group": group,
        "creator_member_count_before_restart": statuses.get("member_count", ""),
        "member_counts_after_recovery": member_counts,
        "messages": {"alice": alice_message, "bob": bob_message},
        "counts_after_recovery": {
            "alice": wait_group_both_sides(scenario, "alice1", group["chat_id"], alice_message),
            "bob": wait_group_both_sides(scenario, "bob1", group["chat_id"], bob_message),
        },
    }


def run_flow(scenario: Scenario, artifact_dir: Path, case: str) -> dict[str, Any]:
    flow_stamp = short_stamp()
    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    cases: dict[str, Any] = {}

    warmup = warm_direct_chat(scenario, flow_stamp)
    if case in ("direct", "both"):
        cases["direct_preconfirm_restart"] = run_direct_preconfirm_restart(scenario, flow_stamp)
    if case in ("group", "both"):
        cases["group_create_preobserve_restart"] = run_group_create_preobserve_restart(scenario, flow_stamp)

    for device_id in ("alice1", "bob1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "case": case,
        "alice_platform": alice["platform"],
        "bob_platform": bob["platform"],
        "relay_url": scenario.relay_url(),
        "android_relay_url": scenario.android_relay_url(),
        "warmup": warmup,
        "cases": cases,
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
    write_json(artifact_dir / "mixed-publish-restart-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    suffix = f"{run_id}-{args.alice_platform}-alice-{args.case}"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-publish-restart-{suffix}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        try:
            scenario.setup()
            result = run_flow(scenario, artifact_dir, args.case)
            print(json.dumps(result, indent=2, sort_keys=True))
        except BaseException as exc:
            failure = {
                "status": "failed",
                "artifact_dir": str(artifact_dir),
                "case": args.case,
                "error": str(exc),
                "error_type": type(exc).__name__,
                "state": str(scenario.state_path),
            }
            write_json(artifact_dir / "mixed-publish-restart-summary.json", failure)
            raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
