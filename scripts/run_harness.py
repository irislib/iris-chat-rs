#!/usr/bin/env python3

import argparse
import base64
import re
import subprocess
import sys


def classify_instrumentation_result(lines: list[str], exit_code: int) -> int:
    if exit_code != 0:
        return exit_code

    saw_failure = False
    saw_runner_success_code = False
    saw_test_success_status = False
    saw_post_success_teardown_crash = False

    for line in lines:
        stripped = line.strip()
        if stripped == "INSTRUMENTATION_CODE: -1":
            saw_runner_success_code = True
        elif stripped == "INSTRUMENTATION_STATUS_CODE: 0":
            saw_test_success_status = True

        if stripped.startswith("INSTRUMENTATION_STATUS_CODE: -") or stripped == "FAILURES!!!":
            saw_failure = True
        elif stripped in {
            "INSTRUMENTATION_RESULT: shortMsg=Process crashed.",
            "INSTRUMENTATION_RESULT: shortMsg=Process crashed",
        }:
            if saw_test_success_status and not saw_failure:
                saw_post_success_teardown_crash = True
            else:
                saw_failure = True
        elif stripped.startswith("INSTRUMENTATION_RESULT: shortMsg="):
            saw_failure = True

    if saw_failure:
        return 1
    if saw_runner_success_code:
        return 0
    if saw_post_success_teardown_crash and saw_test_success_status:
        return 0
    return 1


def verify_isolated_android_user(adb: str, serial: str, user: str) -> None:
    """Only allow destructive push setup in an explicitly selected ephemeral user."""
    if not user.isdecimal() or int(user) == 0:
        raise SystemExit("Select an explicit non-primary ephemeral Android test user.")
    command = [adb] + (["-s", serial] if serial else [])
    users = subprocess.run(command + ["shell", "pm", "list", "users"],
                           check=True, capture_output=True, text=True, timeout=30).stdout
    record = re.search(r"UserInfo\{" + str(int(user)) + r":[^}\n]*:([0-9a-fA-F]+)\}", users)
    # Android UserInfo.FLAG_EPHEMERAL is 0x100; ordinary accounts/profiles must never reset.
    if record is None or not int(record.group(1), 16) & 0x100:
        raise SystemExit("Selected Android test user is not ephemeral; ordinary accounts are protected.")
    current = subprocess.run(command + ["shell", "am", "get-current-user"],
                             check=True, capture_output=True, text=True, timeout=30).stdout.strip()
    if current != str(int(user)):
        raise SystemExit("Switch to the selected ephemeral Android test user before running the push gate.")


def verify_instrumentation_target(adb: str, serial: str, runner: str, package: str) -> None:
    """Reject an installed runner targeting a different application before it runs."""
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+", package):
        raise SystemExit("Invalid Android target package.")
    command = [adb]
    if serial:
        command += ["-s", serial]
    result = subprocess.run(command + ["shell", "pm", "list", "instrumentation"],
                            check=True, capture_output=True, text=True, timeout=30)
    expected = f"instrumentation:{runner} (target={package})"
    if expected not in (line.strip() for line in result.stdout.splitlines()):
        raise SystemExit("Installed instrumentation does not target the selected Android package.")


def main() -> int:
    parser = argparse.ArgumentParser(description="Run an Android instrumentation harness test with quote-safe arguments.")
    parser.add_argument("--adb", required=True, help="Absolute path to adb")
    parser.add_argument("--serial", default="", help="adb device serial. Omit when adb has a single target device.")
    parser.add_argument("--runner", required=True, help="Instrumentation runner package/class")
    parser.add_argument("--package", help="Require the installed runner to target this application ID")
    parser.add_argument("--class-name", required=True, help="Harness test class, without #method")
    parser.add_argument("--test-name", required=True, help="Harness test method")
    parser.add_argument("--user", default="0", help="Android user id")
    parser.add_argument("--require-isolated-user", action="store_true",
                        help="Require the selected user to be current, non-primary, and ephemeral")
    parser.add_argument(
        "--arg",
        action="append",
        default=[],
        help="Instrumentation argument in KEY=VALUE form. Values are base64-encoded before dispatch.",
    )
    args = parser.parse_args()
    if args.require_isolated_user:
        verify_isolated_android_user(args.adb, args.serial, args.user)
    if args.package:
        verify_instrumentation_target(args.adb, args.serial, args.runner, args.package)

    command = [
        args.adb,
        "shell",
        "am",
        "instrument",
        "-w",
        "-r",
        "--user",
        args.user,
    ]
    if args.serial:
        command[1:1] = ["-s", args.serial]
    for item in args.arg:
        if "=" not in item:
            raise SystemExit(f"Invalid --arg `{item}`. Expected KEY=VALUE.")
        key, value = item.split("=", 1)
        encoded = base64.urlsafe_b64encode(value.encode("utf-8")).decode("ascii")
        command.extend(["-e", f"{key}_b64", encoded])

    command.extend(["-e", "class", f"{args.class_name}#{args.test_name}", args.runner])

    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )
    assert process.stdout is not None
    lines: list[str] = []
    for line in process.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        lines.append(line)

    exit_code = process.wait()
    return classify_instrumentation_result(lines, exit_code)


if __name__ == "__main__":
    raise SystemExit(main())
