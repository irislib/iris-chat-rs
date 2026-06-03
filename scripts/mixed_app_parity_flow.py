#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import socket
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from mobile_scenario import (
    ANDROID_APP_PACKAGE,
    ROOT_DIR,
    Scenario,
    discover_android_sdk_dir,
    parse_status,
    redact_sensitive_text,
    run,
)


DEFAULT_SIMULATOR = "Iris Chat iPhone"
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"
REACTION_EMOJI = "\U0001f44d"
CHAT_TTL_SECONDS = 60
DISAPPEARING_TTL_SECONDS = 20


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android app-level parity E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--alice-platform", choices=("ios", "android"), default="ios")
    parser.add_argument("--relay-mode", choices=("local", "public"), default="local")
    parser.add_argument(
        "--public-relays",
        default=os.environ.get("IRIS_E2E_RELAYS", DEFAULT_PUBLIC_RELAYS),
        help=f"Comma-separated public message servers. Default: {DEFAULT_PUBLIC_RELAYS}.",
    )
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching relay URLs.")
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
            f"Need {limit} Android AVD or connected Android device for mixed F12; found {len(avds)} AVDs. "
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
    android_target = select_android_target(args)
    if args.relay_mode == "public":
        port = args.relay_port or 0
        ios_url = args.relay_url or args.public_relays
        android_url = args.android_relay_url or args.relay_url or args.public_relays
        reverse_relay = False
        relay_config = {
            "start": False,
            "port": port,
            "url": ios_url,
            "android_url": android_url,
            "set_id": f"mixed-app-parity-public-{artifact_dir.name}",
        }
    else:
        port = args.relay_port or free_tcp_port()
        ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
        default_android_url = f"ws://127.0.0.1:{port}" if "serial" in android_target else f"ws://10.0.2.2:{port}"
        android_url = args.android_relay_url or args.relay_url or default_android_url
        reverse_relay = relay_uses_localhost(android_url)
        relay_config = {
            "start": True,
            "port": port,
            "label": f"iris.mixed-app-parity.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-app-parity-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        }

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
        "name": f"mixed-app-parity-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": relay_config,
        "ios": {
            "build": not args.skip_build,
        },
        "android": {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": reverse_relay,
        },
        "open_apps": True,
        "devices": [ios_entry, android_entry],
        "groups": [],
    }
    config_path = artifact_dir / "mixed-app-parity-config.json"
    write_json(config_path, config)
    return config_path


def harness(scenario: Scenario, device_id: str, action: str, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, args=args)


@dataclass
class AsyncHarnessProcess:
    device_id: str
    action: str
    process: subprocess.Popen[str]
    reader: threading.Thread
    ready: threading.Event
    lines: list[str]
    log_path: Path


def start_async_harness(
    scenario: Scenario,
    device_id: str,
    action: str,
    *,
    ready_key: str,
    args: dict[str, str],
) -> AsyncHarnessProcess:
    command = scenario.harness_command(device_id, action, args=args)
    env = scenario.scenario_env()
    device = scenario.state["devices"][device_id]
    if device["platform"] == "android":
        env["ANDROID_HOME"] = str(scenario.android_sdk_dir())
    log_path = scenario.work_dir / f"{device_id}-{action}.log"
    lines: list[str] = []
    ready = threading.Event()
    print(redact_sensitive_text("+ " + " ".join(shlex.quote(part) for part in command)), flush=True)
    process = subprocess.Popen(
        command,
        cwd=str(ROOT_DIR),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )

    def read_output() -> None:
        assert process.stdout is not None
        with log_path.open("w", encoding="utf-8") as handle:
            for line in process.stdout:
                lines.append(line)
                redacted = redact_sensitive_text(line)
                print(redacted, end="", flush=True)
                handle.write(redacted)
                handle.flush()
                if f": {ready_key}=true" in line:
                    ready.set()

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    return AsyncHarnessProcess(device_id, action, process, reader, ready, lines, log_path)


def stop_async_harness(handle: AsyncHarnessProcess) -> None:
    if handle.process.poll() is not None:
        handle.reader.join(timeout=5)
        return
    handle.process.terminate()
    try:
        handle.process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        handle.process.kill()
        handle.process.wait(timeout=10)
    handle.reader.join(timeout=5)


def wait_for_async_harness_ready(
    handle: AsyncHarnessProcess,
    *,
    ready_key: str,
    timeout_secs: int,
) -> None:
    if handle.ready.wait(timeout_secs):
        return
    if handle.process.poll() is not None:
        handle.reader.join(timeout=5)
        raise SystemExit(
            f"{handle.device_id} {handle.action} exited before reporting {ready_key}; see {handle.log_path}"
        )
    stop_async_harness(handle)
    raise SystemExit(
        f"Timed out waiting for {ready_key} from {handle.action} on {handle.device_id}; see {handle.log_path}"
    )


def finish_async_harness(
    handle: AsyncHarnessProcess,
    *,
    timeout_secs: int,
) -> dict[str, str]:
    try:
        exit_code = handle.process.wait(timeout=timeout_secs)
    except subprocess.TimeoutExpired:
        stop_async_harness(handle)
        raise SystemExit(f"Timed out waiting for {handle.action} on {handle.device_id}; see {handle.log_path}")
    handle.reader.join(timeout=10)
    output = "".join(handle.lines)
    if exit_code != 0 or "INSTRUMENTATION_CODE: -1" not in output:
        raise SystemExit(f"Harness action failed or did not report success: {handle.action} on {handle.device_id}")
    return parse_status(output)


def live_typing(
    scenario: Scenario,
    *,
    sender_id: str,
    receiver_id: str,
    sender_chat_id: str,
    receiver_chat_id: str,
) -> tuple[dict[str, str], dict[str, str]]:
    receiver = start_async_harness(
        scenario,
        receiver_id,
        "wait_for_typing_from_args",
        ready_key="typing_wait_ready",
        args={
            "chat_id": receiver_chat_id,
            "timeout_secs": "120",
        },
    )
    try:
        wait_for_async_harness_ready(
            receiver,
            ready_key="typing_wait_ready",
            timeout_secs=60,
        )
        sent = harness(
            scenario,
            sender_id,
            "send_typing_from_args",
            chat_id=sender_chat_id,
            wait_for_relay_drain="true",
            relay_drain_timeout_secs="240",
        )
        seen = finish_async_harness(receiver, timeout_secs=150)
        return sent, seen
    except BaseException:
        stop_async_harness(receiver)
        raise


def restart_app(scenario: Scenario, device_id: str, *, wait_for_drain: bool = True) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"], check=False)
        time.sleep(1)
        run(["xcrun", "simctl", "launch", device["udid"], "to.iris.chat"], check=False)
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
        drain_after_restart(scenario, device_id)


def stop_app(scenario: Scenario, device_id: str) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"], check=False)
        return

    adb = str(scenario.adb())
    package_name = device.get("app_package") or ANDROID_APP_PACKAGE
    run([adb, "-s", device["serial"], "shell", "am", "force-stop", package_name], env=scenario.scenario_env(), check=False)


def drain_after_restart(scenario: Scenario, device_id: str) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "report_logged_in_identity",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
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
    scenario.state.setdefault("groups", {})["alice-bob-mixed-app-parity"] = group_state
    scenario.save_state()
    harness(
        scenario,
        "bob1",
        "wait_for_group_chat_from_args",
        chat_id=group_state["chat_id"],
        timeout_secs="300",
    )
    return group_state


def sleep_until_expired(expires_at_raw: str, ttl_seconds: int) -> int:
    try:
        expires_at = int(expires_at_raw)
    except ValueError:
        expires_at = int(time.time()) + ttl_seconds
    delay = max(ttl_seconds + 3, expires_at - int(time.time()) + 3)
    delay = min(delay, 90)
    print(f"Waiting {delay}s for disappearing message expiry", flush=True)
    time.sleep(delay)
    return delay


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = short_stamp()
    direct_message = f"f12-direct-{flow_stamp}"
    direct_reply = f"f12-direct-bob-{flow_stamp}"
    group_alice_message = f"f12-group-alice-{flow_stamp}"
    group_bob_message = f"f12-group-bob-{flow_stamp}"
    disappearing_message = f"f12-disappearing-{flow_stamp}"

    direct_send = send_peer(scenario, "alice1", "bob1", direct_message)
    alice_chat_id = direct_send["chat_id"]
    direct_receive = wait_peer(scenario, "bob1", "alice1", direct_message, expected_count=1)
    bob_chat_id = direct_receive["chat_id"]
    request_accepted = harness(
        scenario,
        "bob1",
        "accept_message_request_from_args",
        chat_id=bob_chat_id,
    )
    direct_reply_send = send_peer(scenario, "bob1", "alice1", direct_reply)
    direct_reply_receive = wait_peer(scenario, "alice1", "bob1", direct_reply, expected_count=1)

    group = create_alice_bob_group(scenario, f"Mixed App Parity {flow_stamp}")
    send_chat(scenario, "alice1", group["chat_id"], group_alice_message)
    group_alice_receive = wait_chat(scenario, "bob1", group["chat_id"], group_alice_message, expected_count=1)
    send_chat(scenario, "bob1", group["chat_id"], group_bob_message)
    group_bob_receive = wait_chat(scenario, "alice1", group["chat_id"], group_bob_message, expected_count=1)

    typing, typing_seen = live_typing(
        scenario,
        sender_id="bob1",
        receiver_id="alice1",
        sender_chat_id=bob_chat_id,
        receiver_chat_id=alice_chat_id,
    )

    seen_marked = harness(
        scenario,
        "bob1",
        "mark_message_seen_from_args",
        chat_id=bob_chat_id,
        message=direct_message,
        direction="incoming",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    seen_observed = harness(
        scenario,
        "alice1",
        "wait_for_message_delivery_from_args",
        chat_id=alice_chat_id,
        message=direct_message,
        direction="outgoing",
        delivery="seen",
    )

    reaction_sent = harness(
        scenario,
        "bob1",
        "react_to_message_from_args",
        chat_id=bob_chat_id,
        message=direct_message,
        direction="incoming",
        emoji=REACTION_EMOJI,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    reaction_observed = harness(
        scenario,
        "alice1",
        "wait_for_message_reaction_from_args",
        chat_id=alice_chat_id,
        message=direct_message,
        direction="outgoing",
        emoji=REACTION_EMOJI,
    )

    settings_set = harness(
        scenario,
        "alice1",
        "set_chat_settings_from_args",
        chat_id=alice_chat_id,
        muted="true",
        pinned="true",
        ttl_seconds=str(CHAT_TTL_SECONDS),
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    settings_before_restart = harness(
        scenario,
        "alice1",
        "wait_for_chat_settings_from_args",
        chat_id=alice_chat_id,
        muted="true",
        pinned="true",
        ttl_seconds=str(CHAT_TTL_SECONDS),
    )
    restart_app(scenario, "alice1")
    settings_after_restart = harness(
        scenario,
        "alice1",
        "wait_for_chat_settings_from_args",
        chat_id=alice_chat_id,
        muted="true",
        pinned="true",
        ttl_seconds=str(CHAT_TTL_SECONDS),
    )

    disappearing_sent = harness(
        scenario,
        "bob1",
        "send_disappearing_message_from_args",
        chat_id=bob_chat_id,
        message=disappearing_message,
        ttl_seconds=str(DISAPPEARING_TTL_SECONDS),
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    disappearing_received = harness(
        scenario,
        "alice1",
        "wait_for_message_from_args",
        chat_id=alice_chat_id,
        message=disappearing_message,
        direction="incoming",
        expected_count="1",
        timeout_secs="240",
    )
    stop_app(scenario, "alice1")
    expiry_wait_seconds = sleep_until_expired(
        disappearing_sent.get("expires_at_secs", ""),
        DISAPPEARING_TTL_SECONDS,
    )
    restart_app(scenario, "alice1")
    disappearing_absent = harness(
        scenario,
        "alice1",
        "wait_for_message_absent_from_args",
        chat_id=alice_chat_id,
        message=disappearing_message,
        direction="incoming",
        timeout_secs="90",
    )

    for device_id in ("alice1", "bob1"):
        drain_after_restart(scenario, device_id)
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "alice_platform": alice["platform"],
        "bob_platform": bob["platform"],
        "relay_mode": scenario.relay_config().get("start", True) and "local" or "public",
        "relay_urls": scenario.relay_url(),
        "android_relay_urls": scenario.android_relay_url(),
        "direct_chat_ids": {
            "alice": alice_chat_id,
            "bob": bob_chat_id,
        },
        "group_chat_id": group["chat_id"],
        "group_id": group["group_id"],
        "messages": {
            "direct": direct_message,
            "direct_reply": direct_reply,
            "group_alice": group_alice_message,
            "group_bob": group_bob_message,
            "disappearing": disappearing_message,
        },
        "proofs": {
            "direct_send": direct_send,
            "direct_receive": direct_receive,
            "request_accepted": request_accepted,
            "direct_reply_send": direct_reply_send,
            "direct_reply_receive": direct_reply_receive,
            "group_alice_receive": group_alice_receive,
            "group_bob_receive": group_bob_receive,
            "typing_sent": typing,
            "typing_seen": typing_seen,
            "seen_marked": seen_marked,
            "seen_observed": seen_observed,
            "reaction_sent": reaction_sent,
            "reaction_observed": reaction_observed,
            "settings_set": settings_set,
            "settings_before_restart": settings_before_restart,
            "settings_after_restart": settings_after_restart,
            "disappearing_sent": disappearing_sent,
            "disappearing_received": disappearing_received,
            "disappearing_absent": disappearing_absent,
        },
        "expiry_wait_seconds": expiry_wait_seconds,
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
    write_json(artifact_dir / "mixed-app-parity-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    suffix = f"{run_id}-{args.relay_mode}-{args.alice_platform}-alice"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-app-parity-{suffix}")).resolve()
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
