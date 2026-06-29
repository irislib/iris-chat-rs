#!/usr/bin/env python3
from __future__ import annotations

import signal

from mobile_scenario_support import *


class MobileScenarioHarnessMixin:
    def ios_data_dir(self, udid: str) -> str:
        completed = run(
            [
                "xcrun",
                "simctl",
                "get_app_container",
                udid,
                "fi.siriusbusiness.irischat",
                "group.fi.siriusbusiness.irischat",
            ],
            capture=True,
        )
        return str(Path(completed.stdout.strip()) / "iris-chat")

    def next_harness_log_paths(self, device_id: str, action: str) -> tuple[int, Path, Path]:
        self._harness_action_seq += 1
        safe_device = re.sub(r"[^A-Za-z0-9_.-]+", "-", device_id)
        safe_action = re.sub(r"[^A-Za-z0-9_.-]+", "-", action)
        unique_log_path = (
            self.work_dir
            / "harness-actions"
            / f"{self._harness_action_seq:04d}-{safe_device}-{safe_action}.log"
        )
        latest_log_path = self.work_dir / f"{safe_device}-{safe_action}.log"
        return self._harness_action_seq, unique_log_path, latest_log_path

    def record_harness_action(
        self,
        *,
        sequence: int,
        device_id: str,
        platform: str,
        action: str,
        args: dict[str, str] | None,
        elapsed_secs: float,
        returncode: int,
        success: bool,
        log_path: Path,
        latest_log_path: Path,
        statuses: dict[str, str],
    ) -> None:
        redacted_args = {
            key: redact_status_value(key, value)
            for key, value in sorted((args or {}).items())
        }
        redacted_statuses = {
            key: redact_status_value(key, value)
            for key, value in sorted(statuses.items())
        }
        self.action_history.append(
            {
                "sequence": sequence,
                "device_id": device_id,
                "platform": platform,
                "action": action,
                "elapsed_secs": round(elapsed_secs, 3),
                "returncode": returncode,
                "success": success,
                "timeout_secs": redacted_args.get("timeout_secs", ""),
                "relay_drain_timeout_secs": redacted_args.get("relay_drain_timeout_secs", ""),
                "log": str(log_path),
                "latest_log": str(latest_log_path),
                "args": redacted_args,
                "statuses": redacted_statuses,
            }
        )

    def ios_harness(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
        rebuild: bool = False,
        check_code: bool = True,
    ) -> dict[str, str]:
        device = self.state["devices"][device_id]
        command = [
            sys.executable,
            str(IOS_HARNESS),
            "--udid",
            device["udid"],
            "--use-app-storage",
            "--run-id",
            device["run_id"],
            "--action",
            action,
        ]
        if reset:
            command.append("--reset")
        if rebuild:
            command.append("--rebuild")
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        sequence, log_path, latest_log_path = self.next_harness_log_paths(device_id, action)
        started_at = time.monotonic()
        completed = run(command, env=self.scenario_env(), check=False)
        elapsed_secs = time.monotonic() - started_at
        redacted_output = redact_sensitive_text(completed.stdout)
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text(redacted_output, encoding="utf-8")
        latest_log_path.write_text(redacted_output, encoding="utf-8")
        statuses = parse_status(completed.stdout)
        success = completed.returncode == 0 and (
            not check_code or "INSTRUMENTATION_CODE: -1" in completed.stdout
        )
        strict_failure = strict_wait_failure(action, args, statuses)
        if strict_failure is not None:
            success = False
        self.record_harness_action(
            sequence=sequence,
            device_id=device_id,
            platform="ios",
            action=action,
            args=args,
            elapsed_secs=elapsed_secs,
            returncode=completed.returncode,
            success=success,
            log_path=log_path,
            latest_log_path=latest_log_path,
            statuses=statuses,
        )
        if not success:
            if strict_failure is not None:
                raise SystemExit(f"iOS harness strict wait failed: {action} on {device_id}: {strict_failure}")
            raise SystemExit(f"iOS harness action failed or did not report success: {action} on {device_id}")
        return statuses

    def android_harness(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
        rebuild: bool = False,
        check_code: bool = True,
    ) -> dict[str, str]:
        del rebuild
        device = self.state["devices"][device_id]
        env = self.scenario_env()
        env["ANDROID_HOME"] = str(self.android_sdk_dir())
        adb = str(self.adb())
        if reset:
            run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_APP_PACKAGE], env=env, check=False)
            run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_TEST_PACKAGE], env=env, check=False)
        command = [
            sys.executable,
            str(ANDROID_HARNESS),
            "--adb",
            adb,
            "--serial",
            device["serial"],
            "--runner",
            ANDROID_RUNNER,
            "--class-name",
            ANDROID_CLASS,
            "--test-name",
            action,
        ]
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        sequence, log_path, latest_log_path = self.next_harness_log_paths(device_id, action)
        started_at = time.monotonic()
        completed = run(command, env=env, check=False)
        elapsed_secs = time.monotonic() - started_at
        redacted_output = redact_sensitive_text(completed.stdout)
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text(redacted_output, encoding="utf-8")
        latest_log_path.write_text(redacted_output, encoding="utf-8")
        statuses = parse_status(completed.stdout)
        success = completed.returncode == 0 and (
            not check_code or "INSTRUMENTATION_CODE: -1" in completed.stdout
        )
        strict_failure = strict_wait_failure(action, args, statuses)
        if strict_failure is not None:
            success = False
        self.record_harness_action(
            sequence=sequence,
            device_id=device_id,
            platform="android",
            action=action,
            args=args,
            elapsed_secs=elapsed_secs,
            returncode=completed.returncode,
            success=success,
            log_path=log_path,
            latest_log_path=latest_log_path,
            statuses=statuses,
        )
        if not success:
            if strict_failure is not None:
                raise SystemExit(f"Android harness strict wait failed: {action} on {device_id}: {strict_failure}")
            raise SystemExit(f"Android harness action failed or did not report success: {action} on {device_id}")
        return statuses

    def harness(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
        rebuild: bool = False,
        check_code: bool = True,
    ) -> dict[str, str]:
        platform = self.state["devices"][device_id]["platform"]
        if platform == "ios":
            return self.ios_harness(
                device_id,
                action,
                args=args,
                reset=reset,
                rebuild=rebuild,
                check_code=check_code,
            )
        if platform == "android":
            return self.android_harness(
                device_id,
                action,
                args=args,
                reset=reset,
                rebuild=rebuild,
                check_code=check_code,
            )
        raise SystemExit(f"Unsupported platform for harness: {platform}")

    def create_account(self, device: dict[str, Any], *, rebuild: bool) -> None:
        device_id = device["id"]
        if device.get("platform") == "ios":
            self.shutdown_ios_devices_except({device_id})
        self.harness(
            device_id,
            "create_account_and_report_identity",
            reset=bool(device.get("reset", False)),
            rebuild=rebuild,
            args={
                "display_name": device.get("display_name", device_id),
            },
        )
        self.configure_device_relays(device_id)
        statuses = self.harness(
            device_id,
            "report_logged_in_identity",
            args={
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": str(device.get("relay_drain_timeout_secs", 60)),
            },
        )
        self.record_identity(device_id, statuses)

    def configure_device_relays(self, device_id: str) -> None:
        relay_urls = self.relay_urls_for_device(device_id)
        if not relay_urls:
            return
        self.harness(
            device_id,
            "set_relays_from_args",
            args={"relay_urls": ",".join(relay_urls)},
        )

    def record_identity(self, device_id: str, statuses: dict[str, str]) -> None:
        device = self.state["devices"][device_id]
        user_id = device["user"]
        device["owner_npub"] = statuses.get("npub", device.get("owner_npub", ""))
        device["owner_hex"] = statuses.get("public_key_hex", device.get("owner_hex", ""))
        device["device_npub"] = statuses.get("device_npub", device.get("device_npub", ""))
        device["device_hex"] = statuses.get("device_public_key_hex", device.get("device_hex", ""))
        if device.get("platform") == "ios":
            device["data_dir"] = self.ios_data_dir(device["udid"])
        elif device.get("platform") == "android":
            device["data_dir"] = statuses.get("data_dir", "/data/user/0/to.iris.chat.debug/files")
            device["app_package"] = statuses.get("app_package", ANDROID_APP_PACKAGE)
        self.state["users"][user_id] = {
            "npub": device["owner_npub"],
            "owner_hex": device["owner_hex"],
        }
        self.save_state()

    def primary_device_for_user(self, user_id: str) -> str:
        for device in self.config.get("devices", []):
            if device.get("user", device["id"]) == user_id and not device.get("linked_to"):
                return device["id"]
        raise SystemExit(f"No primary device configured for user {user_id}")

    def link_device(self, device: dict[str, Any]) -> None:
        device_id = device["id"]
        owner_user = device["linked_to"]
        owner_device_id = self.primary_device_for_user(owner_user)
        owner = self.state["users"].get(owner_user)
        if not owner:
            raise SystemExit(f"Cannot link {device_id}; owner user {owner_user} has no identity")
        self.shutdown_ios_devices_except({device_id, owner_device_id})
        status_file = self.work_dir / f"{device_id}-link.status"
        log_file = self.work_dir / f"{device_id}-link.log"
        status_file.unlink(missing_ok=True)
        with log_file.open("w", encoding="utf-8") as handle:
            command = self.harness_command(
                device_id,
                self.link_wait_action(device_id),
                args=self.link_wait_args(device_id, owner["npub"], status_file),
                reset=bool(device.get("reset", False)),
            )
            print("+ " + " ".join(shlex.quote(part) for part in command), flush=True)
            process = subprocess.Popen(
                command,
                cwd=str(ROOT_DIR),
                env=self.scenario_env(),
                stdout=handle,
                stderr=subprocess.STDOUT,
                text=True,
                start_new_session=True,
            )
        try:
            link_url = self.wait_for_background_status(
                process,
                [status_file, log_file],
                self.link_status_key(device_id),
                int(device.get("link_timeout_secs", 180)),
                log_file=log_file,
            )
        except BaseException:
            self.stop_background_harness(process)
            if log_file.exists():
                print(log_file.read_text(encoding="utf-8", errors="replace"))
            raise
        self.harness(
            owner_device_id,
            "add_authorized_device_from_args",
            args={
                "device_input": link_url,
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": str(device.get("relay_drain_timeout_secs", 60)),
            },
        )
        try:
            exit_code = process.wait(timeout=int(device.get("authorization_timeout_secs", 300)))
        except subprocess.TimeoutExpired:
            self.stop_background_harness(process)
            output = log_file.read_text(encoding="utf-8", errors="replace")
            print(output)
            raise SystemExit(f"Linked device authorization timed out for {device_id}")
        output = log_file.read_text(encoding="utf-8", errors="replace")
        if exit_code != 0 or "INSTRUMENTATION_CODE: -1" not in output:
            print(output)
            raise SystemExit(f"Linked device authorization failed for {device_id}")
        status_output = ""
        if status_file.exists():
            status_output = status_file.read_text(encoding="utf-8", errors="replace")
        self.record_identity(device_id, parse_status(output + "\n" + status_output))

    def wait_for_background_status(
        self,
        process: subprocess.Popen[str],
        paths: list[Path],
        key: str,
        timeout_secs: int,
        *,
        log_file: Path,
    ) -> str:
        deadline = time.monotonic() + timeout_secs
        while time.monotonic() < deadline:
            for path in paths:
                if path.exists():
                    value = parse_status(path.read_text(encoding="utf-8", errors="replace")).get(key)
                    if value:
                        return value
            exit_code = process.poll()
            if exit_code is not None:
                raise SystemExit(f"Background harness exited before reporting {key} (code {exit_code})")
            time.sleep(1)
        joined = ", ".join(str(path) for path in paths)
        raise SystemExit(f"Timed out waiting for {key} in {joined}")

    def stop_background_harness(self, process: subprocess.Popen[str]) -> None:
        if process.poll() is not None:
            return
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        except PermissionError:
            process.terminate()
        try:
            process.wait(timeout=5)
            return
        except subprocess.TimeoutExpired:
            pass
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return
        except PermissionError:
            process.kill()
        process.wait(timeout=5)

    def linked_ios_xcodebuild_timeout_secs(self, device: dict[str, Any]) -> int:
        def positive_int(value: Any, default: int) -> int:
            try:
                parsed = int(value)
            except (TypeError, ValueError):
                return default
            return parsed if parsed > 0 else default

        configured = positive_int(os.environ.get("IRIS_IOS_HARNESS_XCODEBUILD_TIMEOUT_SECS"), 420)
        link_timeout = positive_int(device.get("link_timeout_secs"), 180)
        authorization_timeout = positive_int(device.get("authorization_timeout_secs"), 300)
        return max(configured, link_timeout + authorization_timeout + 120)

    def harness_command(
        self,
        device_id: str,
        action: str,
        *,
        args: dict[str, str] | None = None,
        reset: bool = False,
    ) -> list[str]:
        device = self.state["devices"][device_id]
        if device["platform"] == "ios":
            command = [
                sys.executable,
                str(IOS_HARNESS),
                "--udid",
                device["udid"],
                "--use-app-storage",
                "--run-id",
                device["run_id"],
                "--action",
                action,
            ]
            if reset:
                command.append("--reset")
            if action == "start_linked_device_wait_authorized_from_args":
                command.extend(["--timeout-secs", str(self.linked_ios_xcodebuild_timeout_secs(device))])
        elif device["platform"] == "android":
            adb = str(self.adb())
            if reset:
                run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_APP_PACKAGE], env=self.scenario_env(), check=False)
                run([adb, "-s", device["serial"], "shell", "pm", "clear", ANDROID_TEST_PACKAGE], env=self.scenario_env(), check=False)
            command = [
                sys.executable,
                str(ANDROID_HARNESS),
                "--adb",
                adb,
                "--serial",
                device["serial"],
                "--runner",
                ANDROID_RUNNER,
                "--class-name",
                ANDROID_CLASS,
                "--test-name",
                action,
            ]
        else:
            raise SystemExit(f"Unsupported platform for harness command: {device['platform']}")
        for key, value in (args or {}).items():
            command.extend(["--arg", f"{key}={value}"])
        return command

    def link_wait_action(self, device_id: str) -> str:
        platform = self.state["devices"][device_id]["platform"]
        if platform == "ios":
            return "start_linked_device_wait_authorized_from_args"
        if platform == "android":
            return "start_link_invite_and_wait_for_authorization_from_args"
        raise SystemExit(f"Unsupported platform for link wait: {platform}")

    def link_wait_args(self, device_id: str, owner_input: str, status_file: Path) -> dict[str, str]:
        platform = self.state["devices"][device_id]["platform"]
        args = {"owner_input": owner_input, "status_file": str(status_file)}
        relay_urls = self.relay_urls_for_device(device_id)
        if platform == "ios" and relay_urls:
            args["relay_urls"] = ",".join(relay_urls)
        if platform == "ios":
            timeout_secs = self.state["devices"][device_id].get("authorization_timeout_secs")
            if timeout_secs is not None:
                args["authorization_timeout_secs"] = str(timeout_secs)
        if platform == "android":
            args["authorization_state"] = "AUTHORIZED"
        return args

    def link_status_key(self, device_id: str) -> str:
        platform = self.state["devices"][device_id]["platform"]
        if platform == "android":
            return "invite_url"
        return "link_url"

    def setup_accounts(self) -> None:
        rebuild_next_ios = bool(self.config.get("ios", {}).get("build", True))
        for device in self.config.get("devices", []):
            if device.get("linked_to"):
                continue
            rebuild = bool(device.get("platform") == "ios" and rebuild_next_ios)
            self.create_account(device, rebuild=rebuild)
            if device.get("platform") == "ios":
                rebuild_next_ios = False
        for device in self.config.get("devices", []):
            if device.get("linked_to"):
                self.link_device(device)
