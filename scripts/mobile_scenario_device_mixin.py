#!/usr/bin/env python3
from __future__ import annotations

from mobile_scenario_support import *


class MobileScenarioDeviceMixin:
    def boot_ios(self) -> None:
        ios_devices = [
            device for device in self.config.get("devices", [])
            if device.get("platform") == "ios"
        ]
        if not ios_devices:
            return
        names = [
            device["simulator"]
            for device in ios_devices
            if device.get("simulator")
        ]
        udids: dict[str, str] = {}
        if names:
            if self.lazy_ios_boot_enabled():
                print("Lazy iOS boot enabled; resolving simulator UDIDs without batch boot", flush=True)
                for name in names:
                    udid = self.resolve_or_create_ios_simulator(name)
                    udids[name] = udid
                    self.shutdown_ios_simulator(udid)
                quit_idle_ios_simulator_app()
            else:
                shutdown_stale_ios_simulators(names)
                completed = run([str(IOS_SIMULATORS), "--no-open", *names])
                for line in completed.stdout.splitlines():
                    match = re.match(r"^(.+) ([0-9A-F-]{36}) ", line)
                    if match:
                        udids[match.group(1)] = match.group(2)
        for device in ios_devices:
            device_id = device["id"]
            entry = self.state["devices"].setdefault(device_id, {})
            entry["platform"] = "ios"
            entry["run_id"] = device.get("run_id", device_id)
            entry["user"] = device.get("user", device_id)
            entry["simulator"] = device.get("simulator", "")
            for key in ("linked_to", "link_timeout_secs", "authorization_timeout_secs", "relay_drain_timeout_secs"):
                if key in device:
                    entry[key] = device[key]
            if device.get("udid"):
                entry["udid"] = device["udid"]
            elif device.get("simulator") in udids:
                entry["udid"] = udids[device["simulator"]]
            else:
                raise SystemExit(f"Unable to resolve UDID for iOS device {device_id}")
        self.save_state()

    def simctl_output(self, args: list[str], *, timeout: int = 30) -> str:
        command = ["xcrun", "simctl", *args]
        print("+ " + " ".join(shlex.quote(part) for part in command), flush=True)
        completed = subprocess.run(
            command,
            cwd=str(ROOT_DIR),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
        )
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.returncode != 0:
            raise SystemExit(completed.returncode)
        return completed.stdout

    def preferred_ios_runtime_and_device_type(self) -> tuple[str, str]:
        runtimes = self.simctl_output(["list", "runtimes", "available"])
        runtime_candidates: list[tuple[tuple[int, ...], str]] = []
        for line in runtimes.splitlines():
            if "com.apple.CoreSimulator.SimRuntime.iOS" not in line:
                continue
            match = re.search(r"(com\.apple\.CoreSimulator\.SimRuntime\.iOS[-A-Za-z0-9_.]+)", line)
            if not match:
                continue
            version_parts = tuple(int(part) for part in re.findall(r"\d+", match.group(1)))
            runtime_candidates.append((version_parts, match.group(1)))
        if not runtime_candidates:
            raise SystemExit("No available iOS simulator runtime found.")
        runtime_id = max(runtime_candidates, key=lambda item: item[0])[1]

        devicetypes = self.simctl_output(["list", "devicetypes"])
        preferred_names = ["iPhone 16", "iPhone 16 Pro", "iPhone 15", "iPhone 14"]
        for name in preferred_names:
            pattern = rf"^\s*{re.escape(name)} \((com\.apple\.CoreSimulator\.SimDeviceType\.[^)]+)\)"
            match = re.search(pattern, devicetypes, flags=re.MULTILINE)
            if match:
                return runtime_id, match.group(1)
        match = re.search(r"^\s*iPhone [^(]+ \((com\.apple\.CoreSimulator\.SimDeviceType\.[^)]+)\)", devicetypes, flags=re.MULTILINE)
        if not match:
            raise SystemExit("No iPhone simulator device type found.")
        return runtime_id, match.group(1)

    def resolve_or_create_ios_simulator(self, simulator_name: str) -> str:
        devices = self.simctl_output(["list", "devices", "available"])
        pattern = rf"^\s*{re.escape(simulator_name)} \(([0-9A-F-]{{36}})\)"
        match = re.search(pattern, devices, flags=re.MULTILINE)
        if match:
            return match.group(1)
        runtime_id, device_type_id = self.preferred_ios_runtime_and_device_type()
        command = ["xcrun", "simctl", "create", simulator_name, device_type_id, runtime_id]
        print("+ " + " ".join(shlex.quote(part) for part in command), flush=True)
        completed = subprocess.run(
            command,
            cwd=str(ROOT_DIR),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
        )
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.returncode != 0:
            raise SystemExit(completed.returncode)
        udid = completed.stdout.strip()
        if not re.fullmatch(r"[0-9A-F-]{36}", udid):
            raise SystemExit(f"Unexpected simctl create output for {simulator_name}: {udid}")
        return udid

    def shutdown_ios_simulator(self, udid: str) -> None:
        try:
            subprocess.run(
                ["xcrun", "simctl", "shutdown", udid],
                cwd=str(ROOT_DIR),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=15,
            )
        except subprocess.TimeoutExpired:
            print(f"Timed out shutting down iOS simulator {udid}; continuing", flush=True)

    def shutdown_ios_devices_except(self, keep_device_ids: set[str]) -> None:
        if not self.lazy_ios_boot_enabled():
            return
        for device_id, device in self.state.get("devices", {}).items():
            if device_id in keep_device_ids:
                continue
            if device.get("platform") == "ios" and device.get("udid"):
                self.shutdown_ios_simulator(device["udid"])
        quit_idle_ios_simulator_app()

    def boot_android(self) -> None:
        android_devices = [
            device for device in self.config.get("devices", [])
            if device.get("platform") == "android"
        ]
        if not android_devices:
            return
        avds = [
            device["avd"]
            for device in android_devices
            if device.get("avd")
        ]
        command = [str(ANDROID_EMULATORS)]
        serials: dict[str, str] = {}
        if avds:
            if self.config.get("android", {}).get("headless", True):
                command.append("--headless")
            if self.config.get("android", {}).get("wipe_data", False):
                command.append("--wipe-data")
            command.extend(avds)
            completed = run(command, env=self.scenario_env())
            for line in completed.stdout.splitlines():
                match = re.match(r"^(.+) (\S+)$", line.strip())
                if match:
                    serials[match.group(1)] = match.group(2)
        for device in android_devices:
            device_id = device["id"]
            entry = self.state["devices"].setdefault(device_id, {})
            entry["platform"] = "android"
            entry["run_id"] = device.get("run_id", device_id)
            entry["user"] = device.get("user", device_id)
            entry["avd"] = device.get("avd", "")
            for key in ("linked_to", "link_timeout_secs", "authorization_timeout_secs", "relay_drain_timeout_secs"):
                if key in device:
                    entry[key] = device[key]
            if device.get("serial"):
                entry["serial"] = device["serial"]
            elif device.get("avd") in serials:
                entry["serial"] = serials[device["avd"]]
            else:
                raise SystemExit(f"Unable to resolve serial for Android device {device_id}")
        self.save_state()

    def build_ios(self) -> None:
        ios_devices = [
            device for device in self.state.get("devices", {}).values()
            if device.get("platform") == "ios"
        ]
        if not ios_devices or not self.config.get("ios", {}).get("build", True):
            return
        run([str(IOS_BUILD), "ios-xcframework"], env=self.scenario_env())
        run([str(IOS_BUILD), "ios-xcodeproj"], env=self.scenario_env())

    def build_android(self) -> None:
        android_devices = [
            device for device in self.state.get("devices", {}).values()
            if device.get("platform") == "android"
        ]
        if not android_devices or not self.config.get("android", {}).get("build", True):
            return
        env = self.scenario_env()
        env["ANDROID_HOME"] = str(self.android_sdk_dir())
        env["IRIS_DEBUG_RELAYS"] = self.android_relay_url()
        env["IRIS_DEBUG_RELAY_SET_ID"] = str(self.relay_config()["set_id"])
        run(["./gradlew", ":app:assembleDebug", ":app:assembleDebugAndroidTest"], cwd=ROOT_DIR / "android", env=env)
        apk = ROOT_DIR / "android" / "app" / "build" / "outputs" / "apk" / "debug" / "app-debug.apk"
        test_apk = ROOT_DIR / "android" / "app" / "build" / "outputs" / "apk" / "androidTest" / "debug" / "app-debug-androidTest.apk"
        for device in android_devices:
            serial = device["serial"]
            run([str(self.adb()), "-s", serial, "install", "-r", "-d", str(apk)], env=env)
            run([str(self.adb()), "-s", serial, "install", "-r", "-d", "-t", str(test_apk)], env=env)

