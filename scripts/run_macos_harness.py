#!/usr/bin/env python3
"""Mirror of `run_ios_harness.py` that drives the macOS InteropHarness
target (`IrisChatMacTests`). Mostly identical: same harness env-var
contract, same xctestrun manipulation. The Mac app's GUI scene runs
during the test so the harness needs to avoid blocking dialogs.
"""
from __future__ import annotations
import argparse
import plistlib
import shutil
import subprocess
import sys
from pathlib import Path
import re


ROOT_DIR = Path(__file__).resolve().parent.parent
MACOS_DIR = ROOT_DIR / "macos"
PROJECT_PATH = MACOS_DIR / "IrisChatMac.xcodeproj"
SCHEME = "IrisChatMac"
DERIVED_DATA = MACOS_DIR / ".build" / "harness-derived-data"
ONLY_TEST = "IrisChatMacTests/InteropHarnessTests/testHarnessAction"
STATUS_PATTERN = re.compile(r"^HARNESS_STATUS: ([^=]+)=(.*)$")
TARGET_NAME = "IrisChatMacTests"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the macOS interop harness with explicit xctestrun environment injection."
    )
    parser.add_argument("--action", required=True, help="Harness action name")
    parser.add_argument(
        "--arg", action="append", default=[], help="Harness argument in KEY=VALUE form"
    )
    parser.add_argument("--run-id", help="Stable logical run id for harness storage")
    parser.add_argument("--service", help="Optional explicit keychain service name")
    parser.add_argument(
        "--data-root",
        default="/tmp/iris-macos-harness",
        help="Stable filesystem root for harness data",
    )
    parser.add_argument(
        "--reset", action="store_true", help="Clear harness state before starting"
    )
    parser.add_argument(
        "--rebuild", action="store_true", help="Force build-for-testing before running"
    )
    return parser.parse_args()


def ensure_build(rebuild: bool) -> Path:
    xctestrun_path = find_xctestrun()
    if xctestrun_path is not None and not rebuild:
        return xctestrun_path

    command = [
        "xcodebuild",
        "-project",
        str(PROJECT_PATH),
        "-scheme",
        SCHEME,
        "-destination",
        "platform=macOS",
        "-derivedDataPath",
        str(DERIVED_DATA),
        "-only-testing:" + ONLY_TEST,
        "build-for-testing",
        "CODE_SIGNING_ALLOWED=NO",
    ]
    completed = subprocess.run(
        command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )
    sys.stdout.write(completed.stdout)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)

    xctestrun_path = find_xctestrun()
    if xctestrun_path is None:
        raise SystemExit("xctestrun file was not produced by build-for-testing.")
    return xctestrun_path


def find_xctestrun() -> Path | None:
    products_dir = DERIVED_DATA / "Build" / "Products"
    matches = sorted(
        path for path in products_dir.glob("*.xctestrun") if ".harness" not in path.name
    )
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
                if candidate.get("BlueprintName") == TARGET_NAME:
                    target_config = candidate
                    break
            if target_config is not None:
                break
    else:
        for key, value in data.items():
            if key == "__xctestrun_metadata__":
                continue
            if isinstance(value, dict) and value.get("BlueprintName") == TARGET_NAME:
                target_config = value
                break
            if key == TARGET_NAME:
                target_config = value
                break

    if target_config is None:
        raise SystemExit(f"Unable to find {TARGET_NAME} target in xctestrun file.")

    existing_env = dict(target_config.get("EnvironmentVariables", {}))
    existing_env.update(env_vars)
    target_config["EnvironmentVariables"] = existing_env

    testing_env = dict(target_config.get("TestingEnvironmentVariables", {}))
    testing_env.update(env_vars)
    target_config["TestingEnvironmentVariables"] = testing_env

    with target.open("wb") as handle:
        plistlib.dump(data, handle)

    return target


def run_test(xctestrun_path: Path) -> subprocess.CompletedProcess[str]:
    command = [
        "xcodebuild",
        "test-without-building",
        "-xctestrun",
        str(xctestrun_path),
        "-destination",
        "platform=macOS",
        "-only-testing:" + ONLY_TEST,
    ]
    return subprocess.run(
        command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )


def build_env(args: argparse.Namespace) -> dict[str, str]:
    # Reuse the same IRIS_IOS_HARNESS_* env keys the iOS harness uses
    # so the InteropHarnessTests source compiles into both targets
    # without a platform branch on the env names.
    env_vars = {
        "IRIS_IOS_HARNESS_ACTION": args.action,
        "IRIS_IOS_HARNESS_DATA_ROOT": args.data_root,
        "IRIS_UI_TEST_BYPASS_KEYCHAIN": "1",
    }
    if args.run_id:
        env_vars["IRIS_IOS_HARNESS_RUN_ID"] = args.run_id
        env_vars["IRIS_UI_TEST_RUN_ID"] = args.run_id
    if args.service:
        env_vars["IRIS_IOS_HARNESS_SERVICE"] = args.service
    if args.reset:
        env_vars["IRIS_IOS_HARNESS_RESET"] = "1"
        env_vars["IRIS_UI_TEST_RESET"] = "1"

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
    xctestrun_source = ensure_build(rebuild=args.rebuild)
    env_vars = build_env(args)
    xctestrun_path = prepare_xctestrun(xctestrun_source, env_vars)
    completed = run_test(xctestrun_path)
    sys.stdout.write(completed.stdout)
    emit_status_lines(completed.stdout, success=completed.returncode == 0)
    if completed.returncode != 0:
        print("INSTRUMENTATION_FAILED: macOS harness test failed")
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
