#!/usr/bin/env python3
from __future__ import annotations
import argparse
import base64
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
TRUTHY_VALUES = {"1", "true", "yes", "on"}


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
    parser.add_argument(
        "--pre-body-timeout-secs",
        type=int,
        default=int(os.environ.get("IRIS_IOS_HARNESS_PRE_BODY_TIMEOUT_SECS", "0")),
        help="Optional timeout for test-without-building runs that do not reach the harness test body.",
    )
    parser.add_argument(
        "--pre-test-retries",
        type=int,
        default=int(os.environ.get("IRIS_IOS_HARNESS_PRE_TEST_RETRIES", "1")),
        help="Retry count for simulator install/pre-test launch failures before the harness body starts.",
    )
    parser.add_argument(
        "--build-timeout-secs",
        type=int,
        default=int(os.environ["IRIS_IOS_HARNESS_XCODEBUILD_BUILD_TIMEOUT_SECS"])
        if os.environ.get("IRIS_IOS_HARNESS_XCODEBUILD_BUILD_TIMEOUT_SECS")
        else None,
        help="Hard timeout for xcodebuild build-for-testing. Defaults to --timeout-secs.",
    )
    return parser.parse_args()


def resolve_udid(name: str) -> str:
    command = ["xcrun", "simctl", "list", "devices", "available"]
    completed = subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=20)
    pattern = re.compile(rf"^\s*{re.escape(name)} \(([0-9A-F-]+)\)", re.MULTILINE)
    match = pattern.search(completed.stdout)
    if not match:
        raise SystemExit(f"Simulator `{name}` was not found.")
    return match.group(1)


def is_simulator_udid(udid: str) -> bool:
    command = ["xcrun", "simctl", "list", "devices"]
    try:
        completed = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=20,
        )
    except subprocess.TimeoutExpired:
        return True
    return f"({udid})" in completed.stdout


def simulator_is_booted(udid: str) -> bool:
    try:
        completed = subprocess.run(
            ["xcrun", "simctl", "list", "devices", "booted"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=20,
        )
    except subprocess.TimeoutExpired:
        return False
    return f"({udid}) (Booted)" in completed.stdout


def wait_for_simulator_boot(udid: str) -> None:
    timeout_secs = int(os.environ.get("IRIS_IOS_BOOTSTATUS_TIMEOUT_SECS", "120"))
    fallback_sleep = int(os.environ.get("IRIS_IOS_BOOTSTATUS_FALLBACK_SLEEP_SECS", "20"))
    booted_grace_secs = int(os.environ.get("IRIS_IOS_BOOTSTATUS_BOOTED_GRACE_SECS", "10"))
    process = subprocess.Popen(
        ["xcrun", "simctl", "bootstatus", udid, "-b"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    deadline = time.monotonic() + timeout_secs
    booted_since: float | None = None
    while True:
        if process.poll() is not None:
            if process.returncode == 0:
                return
            break

        now = time.monotonic()
        if simulator_is_booted(udid):
            if booted_since is None:
                booted_since = now
            elif now - booted_since >= booted_grace_secs:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                print(
                    f"INSTRUMENTATION_RETRY: bootstatus still waiting for {udid}; "
                    "continuing because simctl reports Booted",
                    flush=True,
                )
                time.sleep(fallback_sleep)
                return
        else:
            booted_since = None

        if now >= deadline:
            process.kill()
            process.wait()
            if simulator_is_booted(udid):
                print(
                    f"INSTRUMENTATION_RETRY: bootstatus timed out for {udid}; "
                    "continuing because simctl reports Booted",
                    flush=True,
                )
                time.sleep(fallback_sleep)
                return
            raise SystemExit(f"Timed out waiting for simulator {udid} to boot.")

        time.sleep(2)

    if simulator_is_booted(udid):
        print(
            f"INSTRUMENTATION_RETRY: bootstatus failed for {udid}; continuing because simctl reports Booted",
            flush=True,
        )
        time.sleep(fallback_sleep)
        return
    raise SystemExit(f"Simulator {udid} did not finish booting.")


def ensure_simulator_booted(udid: str) -> None:
    if simulator_is_booted(udid):
        return
    boot_simulator(udid)
    wait_for_simulator_boot(udid)


def boot_simulator(udid: str, attempts: int = 3) -> None:
    last_error: subprocess.CalledProcessError | None = None
    for attempt in range(1, attempts + 1):
        try:
            subprocess.run(
                ["xcrun", "simctl", "boot", udid],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=True,
                timeout=20,
            )
            return
        except subprocess.TimeoutExpired:
            print(f"INSTRUMENTATION_RETRY: simctl boot timed out for {udid}; checking bootstatus", flush=True)
            return
        except subprocess.CalledProcessError as error:
            if simulator_is_booted(udid):
                return
            last_error = error
            if attempt < attempts:
                print(
                    f"INSTRUMENTATION_RETRY: simctl boot failed for {udid} "
                    f"with {error.returncode}; retrying",
                    flush=True,
                )
                time.sleep(5)
    if last_error is not None:
        raise last_error


def reboot_simulator(udid: str) -> None:
    try:
        subprocess.run(["xcrun", "simctl", "shutdown", udid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20)
    except subprocess.TimeoutExpired:
        print(f"INSTRUMENTATION_RETRY: simctl shutdown timed out for {udid}; continuing reboot", flush=True)
    boot_simulator(udid)
    wait_for_simulator_boot(udid)


def is_install_service_flake(output: str) -> bool:
    return any(pattern in output for pattern in INSTALL_SERVICE_FLAKE_PATTERNS)


def test_body_started(output: str) -> bool:
    return "HARNESS_STATUS: action=" in output


def is_pre_test_start_failure(completed: subprocess.CompletedProcess[str]) -> bool:
    if completed.returncode == 0:
        return False
    return not test_body_started(completed.stdout)


def env_truthy(name: str) -> bool:
    return os.environ.get(name, "").strip().lower() in TRUTHY_VALUES


def xcodebuild_binary() -> str:
    return shutil.which("xcodebuild") or "/usr/bin/xcodebuild"


def console_user() -> tuple[str, str] | None:
    completed = subprocess.run(
        ["stat", "-f", "%Su", "/dev/console"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    user = completed.stdout.strip()
    if not user or user == "root":
        user = os.environ.get("USER", "").strip()
    if not user:
        return None
    uid_completed = subprocess.run(
        ["id", "-u", user],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    uid = uid_completed.stdout.strip()
    if uid_completed.returncode != 0 or not uid:
        return None
    return user, uid


def xcodebuild_command(args: list[str]) -> list[str]:
    command = [xcodebuild_binary(), *args]
    if not env_truthy("IRIS_IOS_HARNESS_XCODEBUILD_AQUA"):
        return command
    if sys.platform != "darwin":
        raise SystemExit("IRIS_IOS_HARNESS_XCODEBUILD_AQUA is supported only on macOS.")
    user_info = console_user()
    if user_info is None:
        raise SystemExit("Unable to resolve the macOS console user for Aqua xcodebuild.")
    user, uid = user_info
    sudo_ready = subprocess.run(
        ["sudo", "-n", "true"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0
    if not sudo_ready:
        raise SystemExit("Aqua xcodebuild requires passwordless sudo for launchctl asuser.")
    print(f"INSTRUMENTATION_RETRY: running xcodebuild in Aqua session for {user}", flush=True)
    return ["sudo", "-n", "launchctl", "asuser", uid, "sudo", "-n", "-u", user, *command]


def run_xcodebuild(
    args: list[str],
    timeout_secs: int,
    phase: str,
    *,
    pre_body_timeout_secs: int = 0,
) -> subprocess.CompletedProcess[str]:
    command = xcodebuild_command(args)
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

    timed_out_reason: str | None = None
    deadline = time.monotonic() + timeout_secs
    pre_body_deadline = (
        time.monotonic() + pre_body_timeout_secs
        if pre_body_timeout_secs > 0 and phase == "test-without-building"
        else None
    )
    while True:
        if process.poll() is not None:
            break
        now = time.monotonic()
        if now >= deadline:
            timed_out_reason = f"xcodebuild {phase} timed out after {timeout_secs}s"
            break
        if pre_body_deadline is not None:
            with lines_lock:
                stdout_so_far = "".join(lines)
            if test_body_started(stdout_so_far):
                pre_body_deadline = None
            elif now >= pre_body_deadline:
                timed_out_reason = (
                    f"xcodebuild {phase} did not reach harness test body "
                    f"after {pre_body_timeout_secs}s"
                )
                break
        time.sleep(0.5)

    if timed_out_reason is not None:
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
        timeout_line = f"\nINSTRUMENTATION_FAILED: {timed_out_reason}\n"
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


def ensure_build(udid: str, rebuild: bool, timeout_secs: int) -> Path:
    prefer_simulator = is_simulator_udid(udid)
    xctestrun_path = find_xctestrun(prefer_simulator=prefer_simulator)
    if xctestrun_path is not None and not rebuild:
        return xctestrun_path

    command = [
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
    if not prefer_simulator:
        command.insert(-1, "-allowProvisioningUpdates")
    completed = run_xcodebuild(command, timeout_secs=timeout_secs, phase="build-for-testing")
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
    run_id = env_vars.get("IRIS_IOS_HARNESS_RUN_ID", "run")
    action = env_vars.get("IRIS_IOS_HARNESS_ACTION", "action")
    suffix = re.sub(r"[^A-Za-z0-9_.-]+", "-", f"{run_id}-{action}")[:80]
    target = temp_dir / f"{source.stem}.{os.getpid()}.{suffix}.harness.xctestrun"
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


def run_test(
    udid: str,
    xctestrun_path: Path,
    timeout_secs: int,
    *,
    pre_body_timeout_secs: int = 0,
) -> subprocess.CompletedProcess[str]:
    command = [
        "test-without-building",
        "-xctestrun",
        str(xctestrun_path),
        "-destination",
        f"id={udid}",
        "-only-testing:" + ONLY_TEST,
    ]
    return run_xcodebuild(
        command,
        timeout_secs=timeout_secs,
        phase="test-without-building",
        pre_body_timeout_secs=pre_body_timeout_secs,
    )


def run_test_with_retries(
    udid: str,
    xctestrun_path: Path,
    timeout_secs: int,
    *,
    pre_body_timeout_secs: int = 0,
    pre_test_retries: int = 1,
) -> subprocess.CompletedProcess[str]:
    completed = run_test(
        udid,
        xctestrun_path,
        timeout_secs=timeout_secs,
        pre_body_timeout_secs=pre_body_timeout_secs,
    )
    remaining_retries = max(0, pre_test_retries)
    while remaining_retries > 0:
        if completed.returncode == 0:
            return completed
        if is_install_service_flake(completed.stdout):
            print("INSTRUMENTATION_RETRY: iOS simulator install service was not ready; rebooting simulator and retrying")
        elif is_pre_test_start_failure(completed):
            print("INSTRUMENTATION_RETRY: iOS harness did not reach test body; rebooting simulator and retrying")
        else:
            return completed
        reboot_simulator(udid)
        remaining_retries -= 1
        completed = run_test(
            udid,
            xctestrun_path,
            timeout_secs=timeout_secs,
            pre_body_timeout_secs=pre_body_timeout_secs,
        )
    return completed


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
        env_key = "IRIS_IOS_HARNESS_" + key.upper() + "_B64"
        env_vars[env_key] = base64.b64encode(value.encode()).decode()
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
    build_timeout_secs = args.build_timeout_secs or args.timeout_secs
    simulator = is_simulator_udid(udid)
    if simulator:
        ensure_simulator_booted(udid)
    xctestrun_source = ensure_build(udid, rebuild=args.rebuild, timeout_secs=build_timeout_secs)
    env_vars = build_env(args)
    xctestrun_path = prepare_xctestrun(xctestrun_source, env_vars)
    try:
        if simulator:
            completed = run_test_with_retries(
                udid,
                xctestrun_path,
                timeout_secs=args.timeout_secs,
                pre_body_timeout_secs=args.pre_body_timeout_secs,
                pre_test_retries=args.pre_test_retries,
            )
        else:
            completed = run_test(
                udid,
                xctestrun_path,
                timeout_secs=args.timeout_secs,
                pre_body_timeout_secs=args.pre_body_timeout_secs,
            )
    finally:
        if xctestrun_path != xctestrun_source:
            try:
                xctestrun_path.unlink()
            except FileNotFoundError:
                pass
    emit_status_lines(completed.stdout, success=completed.returncode == 0)
    if completed.returncode != 0:
        print("INSTRUMENTATION_FAILED: iOS harness test failed")
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
