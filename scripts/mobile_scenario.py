#!/usr/bin/env python3
from __future__ import annotations

import signal

from mobile_scenario_support import *
from mobile_scenario_device_mixin import MobileScenarioDeviceMixin
from mobile_scenario_harness_mixin import MobileScenarioHarnessMixin

TRUTHY_VALUES = {"1", "true", "yes", "on"}


class Scenario(MobileScenarioDeviceMixin, MobileScenarioHarnessMixin):
    def __init__(self, config_path: Path):
        self.config_path = config_path
        self.config = json.loads(config_path.read_text(encoding="utf-8"))
        self.name = self.config["name"]
        self.work_dir = Path(self.config.get("work_dir") or f"/tmp/iris-mobile-scenario-{self.name}")
        self.state_path = self.work_dir / "state.json"
        self.state: dict[str, Any] = self.load_state()
        self._harness_action_seq = 0
        self.action_history: list[dict[str, Any]] = []

    def load_state(self) -> dict[str, Any]:
        if self.state_path.exists():
            return json.loads(self.state_path.read_text(encoding="utf-8"))
        return {"name": self.name, "devices": {}, "users": {}, "groups": {}, "relay": {}}

    def save_state(self) -> None:
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.state_path.write_text(json.dumps(self.state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        env_lines: list[str] = []
        relay = self.state.get("relay", {})
        for key in ("url", "drop_file", "label", "port"):
            if key in relay:
                env_lines.append(f"RELAY_{key.upper()}={relay[key]}")
        for device_id, device in sorted(self.state.get("devices", {}).items()):
            prefix = device_id.upper().replace("-", "_")
            for key in ("udid", "serial", "run_id", "owner_hex", "owner_npub", "device_hex", "data_dir"):
                if key in device:
                    env_lines.append(f"{prefix}_{key.upper()}={device[key]}")
        for group_id, group in sorted(self.state.get("groups", {}).items()):
            prefix = group_id.upper().replace("-", "_")
            for key in ("chat_id", "group_id", "name"):
                if key in group:
                    env_lines.append(f"{prefix}_{key.upper()}={group[key]}")
        (self.work_dir / "state.env").write_text("\n".join(env_lines) + "\n", encoding="utf-8")

    def relay_config(self) -> dict[str, Any]:
        relay = dict(self.config.get("relay") or {})
        relay.setdefault("start", True)
        relay.setdefault("port", 4848)
        relay.setdefault("label", f"iris.scenario.{self.name}.relay")
        relay.setdefault("drop_file", str(self.work_dir / "drop-events.txt"))
        relay.setdefault("log_file", str(self.work_dir / "relay.log"))
        relay.setdefault("set_id", f"local-{self.name}")
        relay.setdefault("bind_host", "0.0.0.0")
        relay.setdefault("host_interface", "en0")
        if shared_set_id := os.environ.get("IRIS_E2E_RELAY_SET_ID"):
            relay["set_id"] = shared_set_id
        return relay

    def uses_local_relay(self) -> bool:
        return bool(self.relay_config().get("start", True))

    def relay_url(self) -> str:
        relay = self.relay_config()
        return relay.get("url") or f"ws://{host_ip(relay.get('host_interface'))}:{int(relay['port'])}"

    def scenario_env(self) -> dict[str, str]:
        env = os.environ.copy()
        relay = self.relay_config()
        env["IRIS_DEFAULT_RELAYS"] = self.relay_url()
        env["IRIS_RELAY_SET_ID"] = str(relay["set_id"])
        env["IRIS_TRUSTED_TEST_BUILD"] = "true"
        if sdk_dir := discover_android_sdk_dir():
            env.setdefault("ANDROID_HOME", str(sdk_dir))
        return env

    def android_sdk_dir(self) -> Path:
        value = discover_android_sdk_dir()
        if value is None:
            raise SystemExit("Android SDK path not found. Set ANDROID_HOME, ANDROID_SDK_ROOT, or android/local.properties.")
        return value

    def adb(self) -> Path:
        adb = self.android_sdk_dir() / "platform-tools" / "adb"
        if not adb.exists():
            raise SystemExit(f"adb not found at {adb}")
        return adb

    def android_relay_url(self) -> str:
        relay = self.relay_config()
        return relay.get("android_url") or f"ws://10.0.2.2:{int(relay['port'])}"

    def relay_urls_for_device(self, device_id: str) -> list[str]:
        device = self.state["devices"][device_id]
        if device.get("platform") == "android":
            return parse_relay_urls(self.android_relay_url())
        return parse_relay_urls(self.relay_url())

    def lazy_ios_boot_enabled(self) -> bool:
        value = os.environ.get("IRIS_E2E_LAZY_IOS_BOOT")
        if value is not None:
            return value.strip().lower() in TRUTHY_VALUES
        return str(self.config.get("ios", {}).get("lazy_boot", "")).strip().lower() in TRUTHY_VALUES

    def configure_android_relay_access(self) -> None:
        android_devices = [
            device for device in self.state.get("devices", {}).values()
            if device.get("platform") == "android"
        ]
        if not android_devices or not self.config.get("android", {}).get("reverse_relay", False):
            return
        port = str(int(self.relay_config()["port"]))
        env = self.scenario_env()
        env["ANDROID_HOME"] = str(self.android_sdk_dir())
        adb = str(self.adb())
        for device in android_devices:
            run([adb, "-s", device["serial"], "reverse", f"tcp:{port}", f"tcp:{port}"], env=env)

    def remove_android_relay_access(self) -> None:
        android_devices = [
            device for device in self.state.get("devices", {}).values()
            if device.get("platform") == "android" and device.get("serial")
        ]
        if not android_devices or not self.config.get("android", {}).get("reverse_relay", False):
            return
        port = str(int(self.relay_config()["port"]))
        env = self.scenario_env()
        env["ANDROID_HOME"] = str(self.android_sdk_dir())
        adb = str(self.adb())
        for device in android_devices:
            run([adb, "-s", device["serial"], "reverse", "--remove", f"tcp:{port}"], env=env, check=False)

    def stop_relay(self) -> None:
        if not self.uses_local_relay():
            return
        pid = self.state.get("relay", {}).get("pid")
        if pid:
            try:
                os.killpg(int(pid), signal.SIGTERM)
                time.sleep(0.2)
            except (ProcessLookupError, ValueError):
                pass
            except PermissionError:
                os.kill(int(pid), signal.SIGTERM)
        label = str(self.relay_config()["label"])
        run(["launchctl", "remove", label], capture=True, check=False)

    def ensure_relay_binary(self) -> None:
        if local_relay_binary().exists():
            return
        run(
            [
                "cargo",
                "build",
                "--manifest-path",
                str(ROOT_DIR / "core" / "Cargo.toml"),
                "--features",
                "local-relay-bin",
                "--bin",
                "local_nostr_relay",
            ]
        )

    def start_relay(self) -> None:
        relay = self.relay_config()
        self.work_dir.mkdir(parents=True, exist_ok=True)
        if not self.uses_local_relay():
            self.state["relay"] = {
                "url": self.relay_url(),
                "set_id": relay["set_id"],
            }
            if relay.get("android_url"):
                self.state["relay"]["android_url"] = relay["android_url"]
            self.save_state()
            return
        drop_file = Path(relay["drop_file"])
        drop_file.parent.mkdir(parents=True, exist_ok=True)
        drop_file.touch()
        log_file = Path(relay["log_file"])
        log_file.parent.mkdir(parents=True, exist_ok=True)
        self.ensure_relay_binary()
        self.stop_relay()
        port = int(relay["port"])
        if tcp_open("127.0.0.1", port):
            raise SystemExit(
                f"TCP port {port} is already in use. Stop the other local relay or change relay.port."
            )
        bind_addr = f"{relay['bind_host']}:{port}"
        relay_binary = local_relay_binary()
        command = (
            f"IRIS_LOCAL_RELAY_DROP_EVENT_IDS_FILE={shlex.quote(str(drop_file))} "
            f"exec {shlex.quote(str(relay_binary))} {shlex.quote(bind_addr)} "
            f">> {shlex.quote(str(log_file))} 2>&1"
        )
        launch = run(["launchctl", "submit", "-l", str(relay["label"]), "--", "/bin/bash", "-lc", command], check=False)
        relay_pid = ""
        if launch.returncode != 0:
            print(f"launchctl submit failed with {launch.returncode}; starting relay directly.", flush=True)
            relay_env = os.environ.copy()
            relay_env["IRIS_LOCAL_RELAY_DROP_EVENT_IDS_FILE"] = str(drop_file)
            with log_file.open("ab") as handle:
                process = subprocess.Popen(
                    [str(relay_binary), bind_addr],
                    env=relay_env,
                    stdout=handle,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
            relay_pid = str(process.pid)
        try:
            wait_for_tcp("127.0.0.1", port, 30)
        except BaseException:
            if relay_pid:
                try:
                    os.killpg(int(relay_pid), signal.SIGTERM)
                except (ProcessLookupError, ValueError):
                    pass
            raise
        self.state["relay"] = {
            "label": relay["label"],
            "port": port,
            "url": self.relay_url(),
            "drop_file": str(drop_file),
            "log_file": str(log_file),
            "set_id": relay["set_id"],
        }
        if relay_pid:
            self.state["relay"]["pid"] = relay_pid
        self.save_state()

    def resolve_member_input(self, value: str) -> str:
        if value in self.state.get("users", {}):
            return self.state["users"][value]["npub"]
        if value in self.state.get("devices", {}):
            return self.state["devices"][value]["owner_npub"]
        return value

    def devices_for_group(self, creator_id: str, member_values: list[str]) -> list[str]:
        users = {self.state["devices"][creator_id]["user"]}
        for member in member_values:
            if member in self.state["users"]:
                users.add(member)
            elif member in self.state["devices"]:
                users.add(self.state["devices"][member]["user"])
        return [
            device_id
            for device_id, device in self.state.get("devices", {}).items()
            if device.get("user") in users
        ]

    def create_groups(self) -> None:
        for group in self.config.get("groups", []):
            group_key = group["id"]
            creator = group["creator"]
            member_inputs = ",".join(self.resolve_member_input(member) for member in group.get("members", []))
            statuses = self.harness(
                creator,
                "create_group_from_args",
                args={
                    "group_name": group["name"],
                    "member_inputs": member_inputs,
                    "wait_for_relay_drain": "true",
                    "relay_drain_timeout_secs": str(group.get("relay_drain_timeout_secs", 60)),
                },
            )
            group_state = {
                "name": group["name"],
                "chat_id": statuses["chat_id"],
                "group_id": statuses["group_id"],
                "creator": creator,
            }
            self.state["groups"][group_key] = group_state
            self.save_state()
            if group.get("wait_for_members", True):
                for device_id in self.devices_for_group(creator, group.get("members", [])):
                    if device_id == creator:
                        continue
                    self.harness(
                        device_id,
                        "wait_for_group_chat_from_args",
                        args={"chat_id": group_state["chat_id"]},
                    )

    def open_apps(self) -> None:
        for device_id, device in self.state.get("devices", {}).items():
            if device.get("platform") == "ios":
                run(["xcrun", "simctl", "launch", device["udid"], "fi.siriusbusiness.irischat"], capture=True, check=False)
            elif device.get("platform") == "android":
                run(
                    [
                        str(self.adb()),
                        "-s",
                        device["serial"],
                        "shell",
                        "monkey",
                        "-p",
                        ANDROID_APP_PACKAGE,
                        "-c",
                        "android.intent.category.LAUNCHER",
                        "1",
                    ],
                    capture=True,
                    check=False,
                )

    def setup(self) -> None:
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.start_relay()
        self.boot_ios()
        self.boot_android()
        self.build_ios()
        self.build_android()
        self.configure_android_relay_access()
        self.setup_accounts()
        self.create_groups()
        if self.config.get("open_apps", True):
            self.open_apps()
        self.save_state()
        print(f"Scenario ready. State: {self.state_path}")

    def begin_fault(self) -> None:
        self.stop_relay()
        drop_file = Path(self.relay_config()["drop_file"])
        drop_file.parent.mkdir(parents=True, exist_ok=True)
        drop_file.write_text("", encoding="utf-8")
        print(f"Relay stopped. Drop file cleared: {drop_file}")

    def inspect_pending(self, device_id: str, extra: list[str]) -> None:
        device = self.state.get("devices", {}).get(device_id)
        if not device:
            raise SystemExit(f"Unknown device `{device_id}` in state. Run `setup` first.")
        data_dir = self.pending_data_source(device_id)
        run([sys.executable, str(PENDING_PUBLISHES), "list", "--data-dir", data_dir, *extra], env=self.scenario_env())

    def drop_and_resume(self, sender_device: str, peer_device: str, *, limit: int, pairwise_only: bool) -> None:
        sender = self.state.get("devices", {}).get(sender_device)
        peer = self.state.get("devices", {}).get(peer_device)
        if not sender:
            raise SystemExit(f"Unknown sender device `{sender_device}` in state. Run `setup` first.")
        if not peer:
            raise SystemExit(f"Unknown peer device `{peer_device}` in state. Run `setup` first.")
        args = [
            sys.executable,
            str(PENDING_PUBLISHES),
            "write-drop-file",
            "--data-dir",
            self.pending_data_source(sender_device),
            "--limit",
            str(limit),
            "--drop-file",
            str(self.relay_config()["drop_file"]),
            "--chat-id",
            peer["owner_hex"],
        ]
        if pairwise_only:
            args.insert(5, "--pairwise-only")
        run(args, env=self.scenario_env())
        self.start_relay()
        print(f"Relay restarted. Drop file: {self.relay_config()['drop_file']}")

    def pending_data_source(self, device_id: str) -> str:
        device = self.state["devices"][device_id]
        if device.get("platform") == "ios":
            data_dir = device.get("data_dir")
            if not data_dir:
                raise SystemExit(f"Device {device_id} has no data_dir in state")
            return data_dir
        if device.get("platform") == "android":
            destination = self.work_dir / f"{device_id}-core.sqlite3"
            with destination.open("wb") as handle:
                completed = subprocess.run(
                    [
                        str(self.adb()),
                        "-s",
                        device["serial"],
                        "exec-out",
                        "run-as",
                        ANDROID_APP_PACKAGE,
                        "cat",
                        "files/core.sqlite3",
                    ],
                    stdout=handle,
                    stderr=subprocess.PIPE,
                )
            if completed.returncode != 0:
                raise SystemExit(completed.stderr.decode("utf-8", errors="replace"))
            return str(destination)
        raise SystemExit(f"Unsupported platform for pending rows: {device.get('platform')}")

    def cleanup(self, *, shutdown_devices: bool) -> None:
        self.remove_android_relay_access()
        self.stop_relay()
        if shutdown_devices:
            for device in self.state.get("devices", {}).values():
                if device.get("platform") == "ios" and device.get("udid"):
                    run(["xcrun", "simctl", "shutdown", device["udid"]], capture=True, check=False)
                elif (
                    device.get("platform") == "android"
                    and device.get("serial")
                    and device.get("avd")
                ):
                    run([str(self.adb()), "-s", device["serial"], "emu", "kill"], capture=True, check=False)
            shutdown_stale_ios_simulators([])


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run deterministic mobile scenarios from JSON config.")
    parser.add_argument("--config", required=True, type=Path, help="Scenario JSON config.")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("setup", help="Start relay, boot devices, build, seed users/devices/groups.")
    sub.add_parser("begin-fault", help="Stop relay and clear the drop file before manual UI action.")
    inspect = sub.add_parser("inspect-pending", help="List pending relay publishes for a device.")
    inspect.add_argument("--device", required=True)
    inspect.add_argument("--pairwise-only", action="store_true")
    inspect.add_argument("--group-sender-outer-only", action="store_true")
    inspect.add_argument("--format", choices=("table", "json", "ids"), default="table")
    drop = sub.add_parser("drop-and-resume", help="Write a pending event to the drop file and restart relay.")
    drop.add_argument("--sender-device", required=True)
    drop.add_argument("--peer-device", required=True)
    drop.add_argument("--limit", type=int, default=1)
    drop.set_defaults(pairwise_only=True)
    drop.add_argument(
        "--include-non-pairwise",
        action="store_false",
        dest="pairwise_only",
        help="Do not add --pairwise-only when selecting a pending event to drop.",
    )
    cleanup = sub.add_parser("cleanup", help="Stop relay and optionally shut down devices.")
    cleanup.add_argument("--shutdown-devices", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    scenario = Scenario(args.config)
    if args.command == "setup":
        scenario.setup()
    elif args.command == "begin-fault":
        scenario.begin_fault()
    elif args.command == "inspect-pending":
        extra = ["--format", args.format]
        if args.pairwise_only:
            extra.append("--pairwise-only")
        if args.group_sender_outer_only:
            extra.append("--group-sender-outer-only")
        scenario.inspect_pending(args.device, extra)
    elif args.command == "drop-and-resume":
        scenario.drop_and_resume(
            args.sender_device,
            args.peer_device,
            limit=args.limit,
            pairwise_only=args.pairwise_only,
        )
    elif args.command == "cleanup":
        scenario.cleanup(shutdown_devices=args.shutdown_devices)
    else:
        raise SystemExit(f"Unknown command: {args.command}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
