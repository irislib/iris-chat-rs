#!/usr/bin/env python3
from __future__ import annotations
import argparse
import json
import os
import plistlib
import re
import threading
import signal
import shutil
import subprocess
import sys
import time
from pathlib import Path


ROOT_DIR = Path(__file__).resolve().parent.parent
IOS_DIR = ROOT_DIR / "ios"
PROJECT_PATH = IOS_DIR / "IrisChat.xcodeproj"
SCHEME = "IrisChat"
DERIVED_DATA = IOS_DIR / ".build" / "harness-derived-data"
ONLY_TEST = "IrisChatTests/InteropHarnessTests/testHarnessAction"
STATUS_PATTERN = re.compile(r"^HARNESS_STATUS: ([^=]+)=(.*)$")
INSTALL_SERVICE_FLAKE_PATTERNS = (
    "Simulator device failed to install the application",
    "Failed to create promise",
    "Failed to locate promise",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the iOS interop harness with explicit xctestrun environment injection.")
    parser.add_argument("--udid", help="Device or simulator UDID")
    parser.add_argument("--simulator", default="Iris Chat iPhone", help="Simulator name if --udid is omitted")
    parser.add_argument("--action", required=True, help="Harness action name")
    parser.add_argument("--arg", action="append", default=[], help="Harness argument in KEY=VALUE form")
    parser.add_argument("--run-id", help="Stable logical run id for harness storage")
    parser.add_argument("--service", help="Optional explicit keychain service name")
    parser.add_argument("--data-root", default="/tmp/ndr-ios-harness", help="Stable filesystem root for harness data")
    parser.add_argument("--reset", action="store_true", help="Clear harness state before starting")
    parser.add_argument("--use-app-storage", action="store_true", help="Run against the installed app's normal App Group storage/keychain service")
    parser.add_argument("--rebuild", action="store_true", help="Force build-for-testing before running")
    parser.add_argument(
        "--timeout-secs",
        type=int,
        default=int(os.environ.get("IRIS_IOS_HARNESS_XCODEBUILD_TIMEOUT_SECS", "420")),
        help="Hard timeout for xcodebuild test execution.",
    )
    return parser.parse_args()


def resolve_udid(name: str) -> str:
    command = ["xcrun", "simctl", "list", "devices", "available"]
    completed = subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    pattern = re.compile(rf"^\s*{re.escape(name)} \(([0-9A-F-]+)\)", re.MULTILINE)
    match = pattern.search(completed.stdout)
    if not match:
        raise SystemExit(f"Simulator `{name}` was not found.")
    return match.group(1)


def is_simulator_udid(udid: str) -> bool:
    command = ["xcrun", "simctl", "list", "-j", "devices"]
    completed = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    if completed.returncode != 0:
        return False
    try:
        data = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return False
    for devices in data.get("devices", {}).values():
        for device in devices:
            if device.get("udid") == udid:
                return True
    return False


def simulator_is_booted(udid: str) -> bool:
    completed = subprocess.run(
        ["xcrun", "simctl", "list", "devices", "booted"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return f"({udid}) (Booted)" in completed.stdout


def wait_for_simulator_boot(udid: str) -> None:
    timeout_secs = int(os.environ.get("IRIS_IOS_BOOTSTATUS_TIMEOUT_SECS", "120"))
    fallback_sleep = int(os.environ.get("IRIS_IOS_BOOTSTATUS_FALLBACK_SLEEP_SECS", "20"))
    process = subprocess.Popen(
        ["xcrun", "simctl", "bootstatus", udid, "-b"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    try:
        process.wait(timeout=timeout_secs)
        if process.returncode == 0:
            return
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        if simulator_is_booted(udid):
            print(
                f"INSTRUMENTATION_RETRY: bootstatus timed out for {udid}; continuing because simctl reports Booted"
            )
            time.sleep(fallback_sleep)
            return
        raise SystemExit(f"Timed out waiting for simulator {udid} to boot.")

    if simulator_is_booted(udid):
        print(f"INSTRUMENTATION_RETRY: bootstatus failed for {udid}; continuing because simctl reports Booted")
        time.sleep(fallback_sleep)
        return
    raise SystemExit(f"Simulator {udid} did not finish booting.")


def ensure_simulator_booted(udid: str) -> None:
    if simulator_is_booted(udid):
        return
    subprocess.run(["xcrun", "simctl", "boot", udid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    wait_for_simulator_boot(udid)


def reboot_simulator(udid: str) -> None:
    subprocess.run(["xcrun", "simctl", "shutdown", udid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["xcrun", "simctl", "boot", udid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    wait_for_simulator_boot(udid)


def is_install_service_flake(output: str) -> bool:
    return any(pattern in output for pattern in INSTALL_SERVICE_FLAKE_PATTERNS)


def ensure_build(udid: str, rebuild: bool) -> Path:
    prefer_simulator = is_simulator_udid(udid)
    xctestrun_path = find_xctestrun(prefer_simulator=prefer_simulator)
    if xctestrun_path is not None and not rebuild:
        return xctestrun_path

    command = [
        "xcodebuild",
        "-project",
        str(PROJECT_PATH),
        "-scheme",
        SCHEME,
        "-destination",
        f"id={udid}",
        "-derivedDataPath",
        str(DERIVED_DATA),
        "build-for-testing",
    ]
    completed = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    sys.stdout.write(completed.stdout)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)

    xctestrun_path = find_xctestrun(prefer_simulator=prefer_simulator)
    if xctestrun_path is None:
        raise SystemExit("xctestrun file was not produced by build-for-testing.")
    return xctestrun_path


def find_xctestrun(prefer_simulator: bool | None = None) -> Path | None:
    products_dir = DERIVED_DATA / "Build" / "Products"
    matches = sorted(
        path for path in products_dir.glob("*.xctestrun")
        if ".harness" not in path.name
    )
    if prefer_simulator is not None:
        platform = "iphonesimulator" if prefer_simulator else "iphoneos"
        platform_matches = [path for path in matches if platform in path.name]
        if platform_matches:
            return platform_matches[0]
    simulator_matches = [path for path in matches if "iphonesimulator" in path.name]
    if simulator_matches:
        return simulator_matches[0]
    return matches[0] if matches else None


def prepare_xctestrun(source: Path, env_vars: dict[str, str]) -> Path:
    temp_dir = source.parent
    target = temp_dir / f"{source.stem}.harness.xctestrun"
    if target.exists():
        target.unlink()
    shutil.copy2(source, target)

    with target.open("rb") as handle:
        data = plistlib.load(handle)

    target_config = None
    if "TestConfigurations" in data:
        for test_configuration in data.get("TestConfigurations", []):
            for candidate in test_configuration.get("TestTargets", []):
                if candidate.get("BlueprintName") == "IrisChatTests":
                    target_config = candidate
                    break
            if target_config is not None:
                break
    else:
        for key, value in data.items():
            if key == "__xctestrun_metadata__":
                continue
            if isinstance(value, dict) and value.get("BlueprintName") == "IrisChatTests":
                target_config = value
                break
            if key == "IrisChatTests":
                target_config = value
                break

    if target_config is None:
        raise SystemExit("Unable to find IrisChatTests target in xctestrun file.")

    existing_env = dict(target_config.get("EnvironmentVariables", {}))
    existing_env.update(env_vars)
    target_config["EnvironmentVariables"] = existing_env

    testing_env = dict(target_config.get("TestingEnvironmentVariables", {}))
    testing_env.update(env_vars)
    target_config["TestingEnvironmentVariables"] = testing_env

    with target.open("wb") as handle:
        plistlib.dump(data, handle)

    return target


def run_test(udid: str, xctestrun_path: Path, timeout_secs: int) -> subprocess.CompletedProcess[str]:
    command = [
        "xcodebuild",
        "test-without-building",
        "-xctestrun",
        str(xctestrun_path),
        "-destination",
        f"id={udid}",
        "-only-testing:" + ONLY_TEST,
    ]
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
        start_new_session=True,
    )
    assert process.stdout is not None
    lines: list[str] = []
    lines_lock = threading.Lock()

    def read_output() -> None:
        assert process.stdout is not None
        for line in process.stdout:
            with lines_lock:
                lines.append(line)
            sys.stdout.write(line)
            sys.stdout.flush()

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()

    try:
        process.wait(timeout=timeout_secs)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        reader.join(timeout=5)
        timeout_line = f"\nINSTRUMENTATION_FAILED: xcodebuild timed out after {timeout_secs}s\n"
        with lines_lock:
            lines.append(timeout_line)
            stdout = "".join(lines)
        sys.stdout.write(timeout_line)
        sys.stdout.flush()
        return subprocess.CompletedProcess(command, 124, stdout)

    reader.join(timeout=5)
    with lines_lock:
        stdout = "".join(lines)
    return subprocess.CompletedProcess(command, process.returncode, stdout)


def build_env(args: argparse.Namespace) -> dict[str, str]:
    env_vars = {
        "IRIS_IOS_HARNESS_ACTION": args.action,
        "IRIS_IOS_HARNESS_DATA_ROOT": args.data_root,
    }
    if args.run_id:
        env_vars["IRIS_IOS_HARNESS_RUN_ID"] = args.run_id
    if args.service:
        env_vars["IRIS_IOS_HARNESS_SERVICE"] = args.service
    if args.reset:
        env_vars["IRIS_IOS_HARNESS_RESET"] = "1"
    if args.use_app_storage:
        env_vars["IRIS_IOS_HARNESS_USE_APP_STORAGE"] = "1"

    for item in args.arg:
        if "=" not in item:
            raise SystemExit(f"Invalid --arg `{item}`. Expected KEY=VALUE.")
        key, value = item.split("=", 1)
        env_key = "IRIS_IOS_HARNESS_" + key.upper()
        env_vars[env_key] = value
    return env_vars


def emit_status_lines(output: str, success: bool) -> None:
    for line in output.splitlines():
        match = STATUS_PATTERN.match(line.strip())
        if match:
            key, value = match.groups()
            print(f"INSTRUMENTATION_STATUS: {key.lower()}={value}")
    if success:
        print("INSTRUMENTATION_CODE: -1")


def main() -> int:
    args = parse_args()
    udid = args.udid or resolve_udid(args.simulator)
    simulator = is_simulator_udid(udid)
    if simulator:
        ensure_simulator_booted(udid)
    xctestrun_source = ensure_build(udid, rebuild=args.rebuild)
    env_vars = build_env(args)
    xctestrun_path = prepare_xctestrun(xctestrun_source, env_vars)
    completed = run_test(udid, xctestrun_path, timeout_secs=args.timeout_secs)
    if completed.returncode != 0 and simulator and is_install_service_flake(completed.stdout):
        print("INSTRUMENTATION_RETRY: iOS simulator install service was not ready; rebooting simulator and retrying")
        reboot_simulator(udid)
        completed = run_test(udid, xctestrun_path, timeout_secs=args.timeout_secs)
    emit_status_lines(completed.stdout, success=completed.returncode == 0)
    if completed.returncode != 0:
        print("INSTRUMENTATION_FAILED: iOS harness test failed")
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
