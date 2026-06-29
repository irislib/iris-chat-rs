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

from mobile_scenario import ANDROID_APP_PACKAGE, ROOT_DIR, Scenario, discover_android_sdk_dir, run


DEFAULT_SIMULATOR = "Iris Chat iPhone"
USER_VISIBLE_TIMEOUT_SECS = "60"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android offline and restart recovery E2E.")
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
            f"Need {limit} Android AVD or connected Android device for mixed F06; found {len(avds)} AVDs. "
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
        "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
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
        "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
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
        "name": f"mixed-offline-restart-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": {
            "port": port,
            "label": f"iris.mixed-offline-restart.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-offline-restart-{artifact_dir.name}",
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
    config_path = artifact_dir / "mixed-offline-restart-config.json"
    write_json(config_path, config)
    return config_path


def harness(scenario: Scenario, device_id: str, action: str, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, args=args)


def restart_app(
    scenario: Scenario,
    device_id: str,
    *,
    wait_for_drain: bool = True,
    relay_drain_timeout_secs: str = USER_VISIBLE_TIMEOUT_SECS,
) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "fi.siriusbusiness.irischat"], check=False)
        time.sleep(1)
        run(["xcrun", "simctl", "launch", device["udid"], "fi.siriusbusiness.irischat"], check=False)
    else:
        adb = str(scenario.adb())
        package_name = device.get("app_package") or ANDROID_APP_PACKAGE
        run([adb, "-s", device["serial"], "shell", "am", "force-stop", package_name], env=scenario.scenario_env(), check=False)
        time.sleep(1)
        run(
            [
                adb,
                "-s",
                device["serial"],
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

    if wait_for_drain:
        drain_after_restart(scenario, device_id, relay_drain_timeout_secs=relay_drain_timeout_secs)


def stop_app(scenario: Scenario, device_id: str) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "fi.siriusbusiness.irischat"], check=False)
        return

    adb = str(scenario.adb())
    package_name = device.get("app_package") or ANDROID_APP_PACKAGE
    run([adb, "-s", device["serial"], "shell", "am", "force-stop", package_name], env=scenario.scenario_env(), check=False)


def drain_after_restart(
    scenario: Scenario,
    device_id: str,
    *,
    relay_drain_timeout_secs: str = USER_VISIBLE_TIMEOUT_SECS,
) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "report_logged_in_identity",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=relay_drain_timeout_secs,
        relay_drain_runtime_only="true",
    )


def send_peer(
    scenario: Scenario,
    sender: str,
    peer: str,
    message: str,
    *,
    wait_for_delivery: bool = True,
    wait_for_relay_drain: bool = True,
    relay_drain_timeout_secs: str = USER_VISIBLE_TIMEOUT_SECS,
    timeout_secs: str | None = None,
) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        peer_input=scenario.state["devices"][peer]["owner_npub"],
        message=message,
        wait_for_delivery=str(wait_for_delivery).lower(),
        wait_for_relay_drain=str(wait_for_relay_drain).lower(),
        relay_drain_timeout_secs=relay_drain_timeout_secs,
        timeout_secs=timeout_secs or relay_drain_timeout_secs,
    )


def wait_peer(
    scenario: Scenario,
    receiver: str,
    peer: str,
    message: str,
    *,
    direction: str = "incoming",
    expected_count: int | None = None,
    timeout_secs: str = USER_VISIBLE_TIMEOUT_SECS,
) -> dict[str, str]:
    args = {
        "peer_input": scenario.state["devices"][peer]["owner_npub"],
        "message": message,
        "direction": direction,
        "timeout_secs": timeout_secs,
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
    relay_drain_timeout_secs: str = USER_VISIBLE_TIMEOUT_SECS,
    timeout_secs: str | None = None,
) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        chat_id=chat_id,
        message=message,
        wait_for_delivery=str(wait_for_delivery).lower(),
        wait_for_relay_drain=str(wait_for_relay_drain).lower(),
        relay_drain_timeout_secs=relay_drain_timeout_secs,
        timeout_secs=timeout_secs or relay_drain_timeout_secs,
    )


def wait_chat(
    scenario: Scenario,
    receiver: str,
    chat_id: str,
    message: str,
    *,
    direction: str = "incoming",
    expected_count: int | None = None,
    timeout_secs: str = USER_VISIBLE_TIMEOUT_SECS,
) -> dict[str, str]:
    args = {
        "chat_id": chat_id,
        "message": message,
        "direction": direction,
        "timeout_secs": timeout_secs,
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
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    group_state = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-bob-mixed-offline"] = group_state
    scenario.save_state()
    harness(
        scenario,
        "bob1",
        "wait_for_group_chat_from_args",
        chat_id=group_state["chat_id"],
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    return group_state


def run_receiver_closed_phase(
    scenario: Scenario,
    chat_id: str,
    flow_stamp: str,
) -> tuple[dict[str, str], dict[str, str]]:
    messages = {
        "ios_to_android_direct": f"mixed-app-closed-ios-to-android-direct-{flow_stamp}",
        "ios_to_android_group": f"mixed-app-closed-ios-to-android-group-{flow_stamp}",
        "android_to_ios_direct": f"mixed-app-closed-android-to-ios-direct-{flow_stamp}",
        "android_to_ios_group": f"mixed-app-closed-android-to-ios-group-{flow_stamp}",
    }
    ios_id = next(device_id for device_id, device in scenario.state["devices"].items() if device["platform"] == "ios")
    android_id = next(device_id for device_id, device in scenario.state["devices"].items() if device["platform"] == "android")

    stop_app(scenario, android_id)
    send_peer(scenario, ios_id, android_id, messages["ios_to_android_direct"])
    send_chat(scenario, ios_id, chat_id, messages["ios_to_android_group"])
    restart_app(scenario, android_id)
    ios_to_android_direct = wait_peer(
        scenario,
        android_id,
        ios_id,
        messages["ios_to_android_direct"],
        expected_count=1,
    ).get("matching_count", "")
    ios_to_android_group = wait_chat(
        scenario,
        android_id,
        chat_id,
        messages["ios_to_android_group"],
        expected_count=1,
    ).get("matching_count", "")

    stop_app(scenario, ios_id)
    send_peer(scenario, android_id, ios_id, messages["android_to_ios_direct"])
    send_chat(scenario, android_id, chat_id, messages["android_to_ios_group"])
    restart_app(scenario, ios_id)
    android_to_ios_direct = wait_peer(
        scenario,
        ios_id,
        android_id,
        messages["android_to_ios_direct"],
        expected_count=1,
    ).get("matching_count", "")
    android_to_ios_group = wait_chat(
        scenario,
        ios_id,
        chat_id,
        messages["android_to_ios_group"],
        expected_count=1,
    ).get("matching_count", "")
    return messages, {
        "android_direct_from_ios": ios_to_android_direct,
        "android_group_from_ios": ios_to_android_group,
        "ios_direct_from_android": android_to_ios_direct,
        "ios_group_from_android": android_to_ios_group,
    }


def run_message_server_offline_phase(
    scenario: Scenario,
    chat_id: str,
    flow_stamp: str,
) -> tuple[dict[str, str], dict[str, str], dict[str, str]]:
    messages = {
        "alice_to_bob": f"mixed-relay-offline-a2b-{flow_stamp}",
        "bob_to_alice": f"mixed-relay-offline-b2a-{flow_stamp}",
        "alice_group": f"mixed-relay-offline-alice-group-{flow_stamp}",
        "bob_group": f"mixed-relay-offline-bob-group-{flow_stamp}",
    }

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

    restart_app(scenario, "alice1", wait_for_drain=False)
    restart_app(scenario, "bob1", wait_for_drain=False)
    scenario.start_relay()
    for device_id in ("alice1", "bob1", "alice1", "bob1"):
        drain_after_restart(scenario, device_id)

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
    return messages, queued, duplicate_counts


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = short_stamp()
    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    group = create_alice_bob_group(
        scenario,
        f"Mixed Offline Recovery {alice['platform']}-to-{bob['platform']} {flow_stamp}",
    )
    chat_id = group["chat_id"]

    warmups = {
        "alice_to_bob": f"mixed-offline-warmup-a2b-{flow_stamp}",
        "bob_to_alice": f"mixed-offline-warmup-b2a-{flow_stamp}",
        "group": f"mixed-offline-warmup-group-{flow_stamp}",
    }
    send_peer(scenario, "alice1", "bob1", warmups["alice_to_bob"])
    wait_peer(scenario, "bob1", "alice1", warmups["alice_to_bob"], expected_count=1)
    send_peer(scenario, "bob1", "alice1", warmups["bob_to_alice"])
    wait_peer(scenario, "alice1", "bob1", warmups["bob_to_alice"], expected_count=1)
    send_chat(scenario, "alice1", chat_id, warmups["group"])
    wait_chat(scenario, "bob1", chat_id, warmups["group"], expected_count=1)

    app_closed_messages, app_closed_counts = run_receiver_closed_phase(scenario, chat_id, flow_stamp)
    relay_offline_messages, queued_delivery, duplicate_counts = run_message_server_offline_phase(
        scenario,
        chat_id,
        flow_stamp,
    )

    for device_id in ("alice1", "bob1"):
        harness(
            scenario,
            device_id,
            "report_runtime_debug_snapshot",
            wait_for_relay_drain="true",
            wait_for_runtime_idle="true",
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
            runtime_idle_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )
        harness(
            scenario,
            device_id,
            "report_persisted_protocol_snapshot",
            wait_for_relay_drain="true",
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "alice_platform": alice["platform"],
        "bob_platform": bob["platform"],
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "warmup_messages": warmups,
        "receiver_app_closed_messages": app_closed_messages,
        "receiver_app_closed_counts": app_closed_counts,
        "message_server_offline_messages": relay_offline_messages,
        "queued_delivery": queued_delivery,
        "duplicate_counts": duplicate_counts,
        "harness_action_history": scenario.action_history,
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
    write_json(artifact_dir / "mixed-offline-restart-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    suffix = f"{run_id}-{args.alice_platform}-alice"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-offline-restart-{suffix}")).resolve()
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
