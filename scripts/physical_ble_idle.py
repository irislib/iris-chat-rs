#!/usr/bin/env python3
"""Opt-in real iPhone/Android Bluetooth delivery and foreground idle budget gate.

Consumes development build-for-testing products; never builds a release, erases
an account, or auto-selects a phone. Raw logs stay in an owner-only artifact dir.
"""
from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import math
import os
from pathlib import Path
import plistlib
import re
import signal
import socket
import subprocess
import time
import uuid
from urllib.parse import unquote, urlsplit

from native_lab import acquire, release
from android_ble_preservation import BlePreservation


TEST = "FipsBlePhysicalUITests/testSendAndReceiveReceiptOverFipsBle"
PACKAGE = "to.iris.chat.blegate"
BUNDLE = "fi.siriusbusiness.irischat"
BACKGROUND_SECONDS = 75
MAX_RECEIVER_HOLD_MS = 420_000
BACKGROUND_RELEASE_DELAY = 40
BACKGROUND_ENTRY_TIMEOUT = 240
IOS_LAUNCH_SURVIVAL_SECONDS = 15


def receiver_lifetime(seconds, background):
    if not math.isfinite(seconds) or not 10 <= seconds <= 120:
        raise ValueError("Use finite 10–120 second idle intervals")
    # Three intervals include XCTest's discarded warmup. Keep the background
    # window in sync with FipsBlePhysicalUITests and respect the APK's hold cap.
    extra = BACKGROUND_SECONDS if background else 0
    hold_ms = math.ceil((seconds * 3 + 50 + extra) * 1000)
    if hold_ms > MAX_RECEIVER_HOLD_MS:
        raise ValueError("Receiver hold exceeds Android's 420-second limit; reduce --sample-seconds")
    return {"idleHoldMs": hold_ms, "backgroundSeconds": extra, "postIosWaitSeconds": 90 + extra}


def complete_log_records(output, final=False):
    records, separator, partial = output.rpartition("\n")
    if final and partial.startswith(("INSTRUMENTATION_STATUS:", "IRIS_FIPS_BACKGROUND_")):
        raise RuntimeError("Incomplete final background protocol record")
    return records + "\n" if separator else ""


class BackgroundRelease:
    """A single owned token, acknowledged before iOS launch and released while hidden."""
    def __init__(self, token):
        parsed = uuid.UUID(token)
        if str(parsed) != token or parsed.version != 4:
            raise ValueError("Background release requires a canonical random UUID")
        self.token = token
        self.relative_file = f"files/ble-background-release/{token}.ready"
        self.acknowledged = self.release_attempted = self.released = False
        self.started_at = self.observed_at = self.entered_ms = self.released_at = None

    def acknowledge(self, output, final=False):
        output = complete_log_records(output, final)
        def fields(key):
            matches = list(re.finditer(r"^INSTRUMENTATION_STATUS: " + key + r"([^\r\n]*)", output, re.MULTILINE))
            if any(not match.group(1).startswith("=") for match in matches):
                raise RuntimeError("Malformed completed background acknowledgement")
            return matches
        support, tokens, ready = (fields(key) for key in
                                  ("background_release_support", "background_release_token", "fips_ble_ready"))
        if len(support) > 1 or len(tokens) > 1 or len(ready) > 1:
            raise RuntimeError("Repeated background barrier acknowledgement")
        if (support and support[0].group(1) != "=private-file-v1") or (tokens and tokens[0].group(1) != "=" + self.token):
            raise RuntimeError("Android acknowledged a different background barrier")
        if not ready:
            return False
        if (not support or not tokens or ready[0].group(1) != "=true"
                or max(support[0].start(), tokens[0].start()) >= ready[0].start()):
            raise RuntimeError("Android fixture lacks the exact background barrier acknowledgement before readiness")
        self.acknowledged = True
        return True

    def start(self, now):
        if not self.acknowledged or self.started_at is not None:
            raise RuntimeError("Background barrier must start once after acknowledgement")
        self.started_at = now

    def observe(self, output, now, completed=False):
        if self.started_at is None:
            raise RuntimeError("Background barrier has not started")
        output = complete_log_records(output, completed)
        def marker(name):
            values = re.findall(r"^IRIS_FIPS_BACKGROUND_" + name + r"([^\r\n]*)", output, re.MULTILINE)
            if len(values) > 1 or values and not re.fullmatch(r"=[0-9]{1,16}", values[0]):
                raise RuntimeError("Repeated or malformed iOS background marker")
            return int(values[0][1:]) if values else None
        entered, leaving = marker("ENTERED"), marker("LEAVING")
        if entered is None:
            if self.observed_at is not None or leaving is not None or completed or now >= self.started_at + BACKGROUND_ENTRY_TIMEOUT:
                raise RuntimeError("Missing iOS background-entry marker before its deadline")
            return False
        if self.observed_at is None:
            if now >= self.started_at + BACKGROUND_ENTRY_TIMEOUT:
                raise RuntimeError("iOS background entry exceeded its deadline")
            self.observed_at, self.entered_ms = now, entered
        if entered != self.entered_ms or leaving is not None and leaving <= entered:
            raise RuntimeError("iOS background interval changed or is invalid")
        if leaving is not None and not self.released or completed and (not self.released or leaving is None):
            raise RuntimeError("iOS left background or exited before the barrier completed")
        elapsed = now - self.observed_at
        if not self.released and elapsed > BACKGROUND_RELEASE_DELAY + 5:
            raise RuntimeError("Host missed the bounded background-release window")
        return not self.released and elapsed >= BACKGROUND_RELEASE_DELAY

    def mark_released(self, now):
        if self.released or self.observed_at is None or not BACKGROUND_RELEASE_DELAY <= now - self.observed_at <= BACKGROUND_RELEASE_DELAY + 10:
            raise RuntimeError("Background release did not finish inside its bounded window")
        self.released, self.released_at = True, now

    def evidence(self):
        return {"support": "private-file-v1", "token": self.token, "enteredMs": self.entered_ms,
                "observedMonotonic": self.observed_at, "releasedMonotonic": self.released_at,
                "releaseDelaySeconds": BACKGROUND_RELEASE_DELAY, "acknowledged": self.acknowledged,
                "releaseAttempted": self.release_attempted, "released": self.released}


def ancestor_pids():
    pid = os.getppid()
    ancestors = set()
    while pid > 1 and pid not in ancestors:
        ancestors.add(pid)
        try:
            pid = int(subprocess.check_output(["ps", "-o", "ppid=", "-p", str(pid)], text=True, timeout=5))
        except (ValueError, subprocess.SubprocessError):
            break
    return ancestors


def reserve_phone(resource):
    lock, owner = acquire(resource, stale_after=6 * 60 * 60)
    if lock is not None:
        return lock
    # verify-full already holds the phone on behalf of its sequential children.
    # Borrow only a live ancestor's reservation, and never release that lock.
    if owner and owner.get("host") == socket.gethostname() and owner.get("pid") in ancestor_pids():
        return None
    raise RuntimeError("Selected phone is reserved by another native-lab run")


def evaluate_metrics(tests, seconds, max_cpu, max_writes):
    """Do not turn skipped tests, changed units, or incomplete intervals into green."""
    matches = [t for t in tests if t.get("testIdentifier", "").rstrip("()") == TEST]
    if len(matches) != 1 or len(matches[0].get("testRuns", [])) != 1:
        raise ValueError("Expected exactly one physical performance test run")
    metrics = matches[0]["testRuns"][0]["metrics"]

    def values(identifier, units):
        # Display names include the application name and may be localized.
        found = [m for m in metrics if m.get("identifier") == identifier]
        if len(found) != 1 or found[0]["unitOfMeasurement"] not in units:
            raise ValueError(f"Missing or unsupported metric: {identifier}")
        result = found[0]["measurements"]
        if len(result) != 2 or any(not math.isfinite(v) or v < 0 for v in result):
            raise ValueError(f"Invalid or incomplete measurements: {identifier}")
        return [v * units[found[0]["unitOfMeasurement"]] for v in result]

    elapsed = values("com.apple.dt.XCTMetric_Clock.time.monotonic", {"s": 1})
    cpu = values(f"com.apple.dt.XCTMetric_CPU-{BUNDLE}.time", {"s": 1})
    writes = values(f"com.apple.dt.XCTMetric_Disk-{BUNDLE}.logical_writes", {"B": 1, "kB": 1000, "MB": 1000000})
    if any(t < seconds * 0.95 or t > seconds * 1.20 for t in elapsed):
        raise ValueError("Idle interval was truncated or stalled")
    percents = [100 * c / t for c, t in zip(cpu, elapsed)]
    rates = [w / 1024**2 * 60 / t for w, t in zip(writes, elapsed)]
    failures = []
    for i, (percent, rate) in enumerate(zip(percents, rates), 1):
        if percent > max_cpu:
            failures.append(f"Interval {i}: CPU {percent:.2f}% > {max_cpu:.2f}% of one core")
        if rate > max_writes:
            failures.append(f"Interval {i}: writes {rate:.2f} > {max_writes:.2f} MiB/min")
    return {"ok": not failures, "cpuPercentOneCore": percents,
            "diskWriteMiBPerMinute": rates, "elapsedSeconds": elapsed,
            "maxCpuPercentOneCore": max_cpu, "maxDiskWriteMiBPerMinute": max_writes,
            "failures": failures}


def internet_addresses(output, dummy_is_virtual):
    unexpected = []
    for line in output.splitlines():
        match = re.match(r"^\d+:\s+(\S+)\s+(inet6?)\s+(\S+)\s+scope\s+(\S+)", line)
        if not match:
            if line.strip():
                unexpected.append(line)
            continue
        interface, family, address, scope = match.groups()
        parsed = ipaddress.ip_interface(address).ip
        if interface == "lo" and parsed.is_loopback:
            continue
        if (interface == "dummy0" and dummy_is_virtual and family == "inet6"
                and scope == "link" and parsed.is_link_local):
            continue
        unexpected.append(line)
    return unexpected


def android_fields(output):
    return dict(re.findall(r"^INSTRUMENTATION_STATUS: ([^=\n]+)=(.*)$", output, re.MULTILINE))


def require_android_success(output, expected):
    fields = android_fields(output)
    if ("INSTRUMENTATION_CODE: -1" not in output or "FAILURES!!!" in output
            or "INSTRUMENTATION_FAILED" in output
            or any(fields.get(k) != v for k, v in expected.items())):
        raise RuntimeError("Android test did not confirm the required evidence; see private log")
    return fields


def require_isolated_instrumentation(output):
    expected = (f"instrumentation:{PACKAGE}.test/androidx.test.runner.AndroidJUnitRunner "
                f"(target={PACKAGE})")
    if expected not in output.splitlines():
        raise ValueError("The test runner must target only the isolated .blegate app")


def evaluate_background_reply(directory, reply):
    status = require_android_success((directory / "receiver.log").read_text(),
        {"background_reply_received": "true", "background_reply": reply})
    log = (directory / "ios-test.log").read_text()
    def timestamp(marker):
        values = re.findall(marker + r"=(\d+)", log)
        if len(values) != 1:
            raise ValueError("Missing or repeated iPhone background interval")
        return int(values[0])
    entered = timestamp("IRIS_FIPS_BACKGROUND_ENTERED")
    leaving = timestamp("IRIS_FIPS_BACKGROUND_LEAVING")
    sent, received = int(status["background_sent_ms"]), int(status["background_received_ms"])
    if not (entered + 30_000 < sent <= received < leaving - 3_000 and leaving - entered >= 74_000):
        raise ValueError("Receipt must arrive after suspension time and before iPhone foregrounding")
    return {"backgroundReplyReceivedWhileHidden": True,
            "backgroundDeliveryMs": received - sent}


def evaluate_capture(directory, probe, seconds, max_cpu, max_writes, background_reply=None):
    summary = json.loads((directory / "test-summary.log").read_text())
    if (summary.get("result") != "Passed" or summary.get("totalTestCount") != 1
            or summary.get("passedTests") != 1 or summary.get("failedTests") != 0
            or summary.get("skippedTests") != 0 or summary.get("testFailures")):
        raise ValueError("Require one passed physical test with no skips or failures")
    require_android_success((directory / "receiver.log").read_text(),
                            {"seen": "true", "accepted": "true", "message": probe})
    checks = [json.loads(line) for line in (directory / "network-checks.jsonl").read_text().splitlines()]
    # The controller samples every ~5 seconds. Allow command latency but reject
    # missing coverage, long gaps, or a claimed success that contains IP routes.
    times = [check["time"] for check in checks]
    start, end = summary["startTime"], summary["finishTime"]
    if (len(times) < 2 or not math.isfinite(start) or not math.isfinite(end) or start >= end
            or any(not math.isfinite(t) for t in times)
            or any(not 0 < b - a <= 15 for a, b in zip(times, times[1:]))
            or not start - 90 <= times[0] <= start + 15 or not end - 15 <= times[-1] <= end + 15):
        raise ValueError("Network isolation evidence does not cover the physical test")
    if any(check.get("ok") is not True or check.get("ipv4Routes") != ""
           or check.get("ipv6Routes") != "" for check in checks):
        raise ValueError("Android had an IP path or failed network isolation")
    metrics = json.loads((directory / "metrics.log").read_text())
    result = evaluate_metrics(metrics, seconds, max_cpu, max_writes)
    result.update(physicalTestPassed=True, exactMessageSeen=True, androidOfflineChecks=len(checks))
    if background_reply:
        result.update(evaluate_background_reply(directory, background_reply))
    return result


def evaluate_saved_run(directory, max_cpu, max_writes):
    original = json.loads((directory / "result.json").read_text())
    restored = original.get("restoration", {})
    required = ("wifi_on", "mobile_data", "bluetooth_unchanged", "ios_normal_launch",
                "android_preferences", "android_permissions", "android_account_stopped", "android_account_preserved")
    if any(restored.get(k) is not True for k in required) or any(v is not True for v in restored.values()):
        raise ValueError("Saved run has missing or failed device restoration")
    data = plistlib.loads((directory / "physical-idle.xctestrun").read_bytes())
    targets = ([t for c in data["TestConfigurations"] for t in c.get("TestTargets", [])]
               if "TestConfigurations" in data else [v for v in data.values() if isinstance(v, dict)])
    environments = [t.get("EnvironmentVariables", {}) for t in targets]
    environments = [e for e in environments if e.get("IRIS_FIPS_IDLE_METRICS") == "1"]
    if len(environments) != 1:
        raise ValueError("Expected one configured physical idle test")
    env = environments[0]
    if not original.get("testRunId") or original["testRunId"] != env.get("IRIS_FIPS_PHYSICAL_RUN_ID"):
        raise ValueError("Saved result belongs to a different test run")
    seconds = float(env["IRIS_FIPS_IDLE_SECONDS"])
    if not 10 <= seconds <= 120:
        raise ValueError("Invalid saved measurement interval")
    result = evaluate_capture(directory, env["IRIS_FIPS_PHYSICAL_MESSAGE"], seconds, max_cpu, max_writes,
                              env.get("IRIS_FIPS_BACKGROUND_REPLY"))
    launch = original.get("iosNormalLaunchSurvival")
    if launch is not None:
        duration = launch.get("observedSeconds") if isinstance(launch, dict) else None
        if (not isinstance(launch, dict) or launch.get("contract") != "same-executable-pid-15s-v1"
                or launch.get("passed") is not True or restored.get("ios_normal_launch_survived") is not True
                or type(duration) not in (int, float) or not math.isfinite(duration)
                or duration < IOS_LAUNCH_SURVIVAL_SECONDS):
            raise ValueError("Saved iPhone launch has no successful survival measurement")
    else:
        launch = {"contract": "legacy-launch-command-only", "survivalChecked": False}
    evidence = ("result.json", "physical-idle.xctestrun", "test-summary.log", "metrics.log",
                "receiver.log", "network-checks.jsonl")
    result.update(evaluationMode="saved-capture", restoration=restored,
                  iosLaunchVerification=launch,
                  originalResultOk=original.get("ok"), originalError=original.get("error"),
                  evidenceSha256={name: hashlib.sha256((directory / name).read_bytes()).hexdigest()
                                  for name in evidence})
    return result


def write_json(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n")


def ios_process(record):
    """Validate CoreDevice's actual process record, not a display-name match."""
    if not isinstance(record, dict):
        raise ValueError("Missing iPhone process record")
    pid, executable = record.get("processIdentifier"), record.get("executable")
    if type(pid) is not int or pid <= 0 or not isinstance(executable, str):
        raise ValueError("Invalid iPhone process identifier or executable")
    url = urlsplit(executable)
    path = Path(unquote(url.path))
    if (url.scheme != "file" or url.netloc or url.query or url.fragment
            or not path.is_absolute() or ".." in path.parts):
        raise ValueError("Invalid iPhone process executable URL")
    return pid, executable, path


def ios_command_result(path):
    document = json.loads(path.read_text())
    if (not isinstance(document, dict) or not isinstance(document.get("info"), dict)
            or document["info"].get("outcome") != "success" or not isinstance(document.get("result"), dict)):
        raise ValueError("iPhone process command did not return successful JSON")
    return document["result"]


def verify_ios_launch(command, device, directory, evidence):
    """Launch once and observe the same app PID; this is not an upgrade test."""
    evidence.update(contract="same-executable-pid-15s-v1", passed=False,
                    requiredSeconds=IOS_LAUNCH_SURVIVAL_SECONDS, processSamples=[])
    launch = directory / "ios-normal-account.json"
    command(["xcrun", "devicectl", "device", "process", "launch", "--device", device,
             "--terminate-existing", BUNDLE, "--json-output", str(launch)], "ios-normal-account",
            env={key: value for key, value in os.environ.items() if not key.startswith("DEVICECTL_CHILD_")})
    launched = ios_command_result(launch)
    pid, executable, path = ios_process(launched.get("process"))
    if path.name != "IrisChat" or path.parent.name != "IrisChat.app":
        raise ValueError("Launch returned a different app or an extension process")
    evidence.update(processIdentifier=pid, launchEvidence=launch.name)
    device_identifier = launched.get("deviceIdentifier")
    if device_identifier is not None and (not isinstance(device_identifier, str) or not device_identifier):
        raise ValueError("Launch returned an invalid iPhone identifier")
    deadline = time.monotonic() + IOS_LAUNCH_SURVIVAL_SECONDS + 10
    first_observed = None
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError("iPhone launch survival observation exceeded its budget")
        sample = directory / f"ios-normal-process-{len(evidence['processSamples']):02d}.json"
        evidence["processSamples"].append(sample.name)
        command(["xcrun", "devicectl", "device", "info", "processes", "--device", device,
                 "--search", "IrisChat",
                 "--json-output", str(sample)], sample.stem, timeout=min(5, remaining))
        inventory = ios_command_result(sample)
        if device_identifier is not None and inventory.get("deviceIdentifier") != device_identifier:
            raise ValueError("Process inventory belongs to a different iPhone")
        records = inventory.get("runningProcesses")
        if not isinstance(records, list):
            raise ValueError("iPhone process inventory is malformed")
        observed = [ios_process(record)[:2] for record in records]
        if observed.count((pid, executable)) != 1 or sum(item[0] == pid for item in observed) != 1:
            raise RuntimeError("The launched iPhone app process disappeared, restarted, or changed executable")
        now = time.monotonic()
        if now > deadline:
            raise RuntimeError("iPhone process observation exceeded its budget")
        if first_observed is None:
            first_observed = now
        elapsed = now - first_observed
        evidence["observedSeconds"] = elapsed
        if elapsed >= IOS_LAUNCH_SURVIVAL_SECONDS:
            evidence["passed"] = True
            return
        time.sleep(min(1, IOS_LAUNCH_SURVIVAL_SECONDS - elapsed))


def stop(child):
    if child is not None and child.poll() is None:
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=10)


class Gate:
    def __init__(self, args):
        self.args = args
        self.out = args.artifact_dir.resolve()
        self.out.mkdir(parents=True, exist_ok=False)
        self.ios = self.android = None
        self.background_release = None
        self.snapshot = {}
        self.locks = []
        self.restored = {}
        self.result = {"ok": False}
        self.run_id = "ble-idle-" + uuid.uuid4().hex
        self.probe = "idle-probe-" + uuid.uuid4().hex
        self.saved_account = BlePreservation(self)

    def command(self, argv, label, timeout=30, *, env=None):
        with (self.out / (label + ".log")).open("w") as log:
            try:
                result = subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT, timeout=timeout, env=env)
            except subprocess.TimeoutExpired:
                raise RuntimeError(f"{label} timed out; see private log") from None
        if result.returncode:
            raise RuntimeError(f"{label} failed; see private log")
        return (self.out / (label + ".log")).read_text()

    def adb(self, *args):
        return self.command(["adb", "-s", self.args.android_serial, "shell", *args], "android-command").strip()

    def action_command(self, action, extra=(), test_class="RealRelayHarnessTest"):
        return ["adb", "-s", self.args.android_serial, "shell", "am", "instrument", "-w", "-r",
                "-e", "class", "to.iris.chat." + test_class + "#" + action,
                *self.saved_account.arguments(), *extra,
                PACKAGE + ".test/androidx.test.runner.AndroidJUnitRunner"]

    def action(self, action, expected, extra=(), test_class="RealRelayHarnessTest"):
        output = self.command(self.action_command(action, extra, test_class), action, 180)
        return require_android_success(output, expected)

    def preflight(self):
        if self.args.android_serial.startswith("emulator-"):
            raise ValueError("A physical Android peer is required")
        if self.adb("getprop", "ro.kernel.qemu") == "1":
            raise ValueError("An emulator cannot establish the physical Bluetooth proof")
        self.command(["xcrun", "devicectl", "list", "devices", "--json-output",
                      str(self.out / "devices.json")], "devices")
        devices = json.loads((self.out / "devices.json").read_text())["result"]["devices"]
        matches = [d for d in devices if self.args.iphone in (
            d.get("identifier"), d.get("deviceProperties", {}).get("name"),
            d.get("hardwareProperties", {}).get("udid"))
            and d.get("hardwareProperties", {}).get("reality") == "physical"]
        if len(matches) != 1:
            raise ValueError("Select exactly one paired physical iPhone by name or identifier")
        self.phone = matches[0]
        self.iphone = self.phone["hardwareProperties"]["udid"]
        for resource in ("ios-device:" + self.iphone, "android:" + self.args.android_serial):
            lock = reserve_phone(resource)
            if lock is not None:
                self.locks.append(lock)
        # A disconnected CoreDevice tunnel may reconnect here; Wi-Fi is supported.
        self.command(["xcrun", "devicectl", "device", "info", "details", "--device", self.iphone,
                      "--json-output", str(self.out / "iphone-details.json")], "iphone-details")
        if "package:" + PACKAGE not in self.adb("pm", "list", "packages", PACKAGE).splitlines():
            raise ValueError("Install the isolated .blegate development app and test APK first")
        require_isolated_instrumentation(self.adb("pm", "list", "instrumentation"))
        snapshot = {k: self.adb("settings", "get", "global", k)
                    for k in ("wifi_on", "mobile_data", "bluetooth_on")}
        if any(snapshot[k] not in ("0", "1") for k in ("wifi_on", "mobile_data")):
            raise ValueError("Cannot safely capture Android radio settings")
        if snapshot["bluetooth_on"] != "1":
            raise ValueError("Enable Bluetooth on the selected Android peer first")
        self.snapshot = snapshot
        write_json(self.out / "network-before.json", self.snapshot)
        self.saved_account.capture()

    def offline(self):
        addresses = self.adb("ip", "-o", "addr", "show", "up")
        routes = self.adb("ip", "route")
        routes6 = self.adb("ip", "-6", "route")
        dummy = ("dummy0" in addresses and
                 self.adb("readlink", "-f", "/sys/class/net/dummy0") == "/sys/devices/virtual/net/dummy0")
        ok = (not internet_addresses(addresses, dummy) and not routes and not routes6
              and self.adb("settings", "get", "global", "wifi_on") == "0"
              and self.adb("settings", "get", "global", "mobile_data") == "0"
              and self.adb("settings", "get", "global", "bluetooth_on") == "1")
        with (self.out / "network-checks.jsonl").open("a") as f:
            f.write(json.dumps({"time": time.time(), "ok": ok, "addresses": addresses,
                                "ipv4Routes": routes, "ipv6Routes": routes6}) + "\n")
        return ok

    def configure(self, peer, device):
        if not re.fullmatch(r"[a-f0-9]{64}", device) or device not in self.saved_account.before["devices"]:
            raise ValueError("The Bluetooth target must be the saved account's current device")
        source = self.args.xctestrun.resolve()
        def rebase(value):
            if isinstance(value, str):
                return value.replace("__TESTROOT__", str(source.parent))
            if isinstance(value, list):
                return [rebase(v) for v in value]
            if isinstance(value, dict):
                return {k: rebase(v) for k, v in value.items()}
            return value
        data = rebase(plistlib.loads(source.read_bytes()))
        targets = ([t for c in data["TestConfigurations"] for t in c.get("TestTargets", [])]
                   if "TestConfigurations" in data else [v for v in data.values() if isinstance(v, dict)])
        matches = [t for t in targets if "IrisChatUITests" in str(t.get("TestBundlePath", ""))]
        if len(matches) != 1:
            raise ValueError("Expected one IrisChatUITests build-for-testing target")
        target = matches[0]
        # A previously configured run must not inject an explicit normal-account
        # data path into the freshly isolated UI test.
        for key in ("EnvironmentVariables", "UITargetAppEnvironmentVariables"):
            target[key] = {k: v for k, v in target.get(key, {}).items()
                           if not k.startswith(("IRIS_UI_TEST_", "IRIS_FIPS_"))}
        environment = {"IRIS_FIPS_PHYSICAL_PEER_NPUB": peer,
                       "IRIS_FIPS_PHYSICAL_PEER_DEVICE_HEX": device,
                       "IRIS_FIPS_PHYSICAL_RUN_ID": self.run_id,
                       "IRIS_FIPS_PHYSICAL_MESSAGE": self.probe,
                       "IRIS_FIPS_IDLE_METRICS": "1",
                       "IRIS_FIPS_IDLE_SECONDS": str(self.args.sample_seconds)}
        if getattr(self.args, "background", False):
            environment["IRIS_FIPS_BACKGROUND_REPLY"] = self.probe + "-reply"
        target.setdefault("EnvironmentVariables", {}).update(environment)
        target["OnlyTestIdentifiers"] = [TEST]
        target["TestExecutionOrdering"] = "lexical"
        configured = self.out / "physical-idle.xctestrun"
        configured.write_bytes(plistlib.dumps(data))
        self.result["xctestrunSha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
        self.result["testRunId"] = self.run_id
        artifacts = {}
        for key in ("TestHostPath", "UITargetAppPath", "TestBundlePath"):
            raw = target.get(key, "").replace("__TESTHOST__", target.get("TestHostPath", ""))
            path = Path(raw)
            if path.is_dir() and (path / "Info.plist").is_file():
                info = plistlib.loads((path / "Info.plist").read_bytes())
                binary = path / info["CFBundleExecutable"]
                artifacts[key] = {"sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                                  "bundleVersion": info.get("CFBundleVersion")}
        if "UITargetAppPath" not in artifacts or "TestBundlePath" not in artifacts:
            raise ValueError("Require local app and test products for artifact provenance")
        self.result["builtArtifacts"] = artifacts
        return configured

    def execute(self):
        lifetime = receiver_lifetime(self.args.sample_seconds, getattr(self.args, "background", False))
        self.result["receiverLifetime"] = lifetime
        self.preflight()
        self.saved_account.started = True
        identity = self.action("report_logged_in_identity", {"app_package": PACKAGE,
            "public_key_hex": self.saved_account.before["owner"]})
        peer = identity.get("npub", "")
        if not peer.startswith("npub1"):
            raise ValueError("The isolated Android account did not report its identity")
        self.action("set_read_receipts_from_args", {"read_receipts_enabled": "true"}, ["-e", "enabled", "true"])
        self.action("disable_relays_and_report", {"relay_count": "0"})
        configured = self.configure(peer, identity.get("device_public_key_hex", ""))
        self.adb("svc", "wifi", "disable")
        self.adb("svc", "data", "disable")
        self.adb("settings", "put", "global", "mobile_data", "0")
        deadline = time.monotonic() + 30
        while not self.offline():
            if time.monotonic() > deadline:
                raise RuntimeError("Android still has an IP path; Bluetooth proof would be invalid")
            time.sleep(1)
        barrier = BackgroundRelease(str(uuid.uuid4())) if self.args.background else None
        self.background_release = barrier
        with (self.out / "receiver.log").open("w") as log:
            extra = (["-e", "background_reply", self.probe + "-reply",
                      "-e", "background_release_token", barrier.token] if barrier else [])
            self.android = subprocess.Popen(self.action_command(
                "accept_and_mark_incoming_message_seen_from_args",
                ["-e", "message", self.probe, "-e", "timeout_ms", "240000",
                 "-e", "idle_hold_ms", str(lifetime["idleHoldMs"])] + extra),
                stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 90
        while True:
            receiver_output = (self.out / "receiver.log").read_text()
            ready = barrier.acknowledge(receiver_output) if barrier else "fips_ble_ready=true" in receiver_output
            if ready:
                break
            if self.android.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Android Bluetooth receiver did not become ready")
            time.sleep(1)
        print("Bluetooth peer has no IP path. Running message/receipt and idle measurements.", flush=True)
        result_path = self.out / "PhysicalIdle.xcresult"
        command = ["xcodebuild", "-xctestrun", str(configured), "-destination", "platform=iOS,id=" + self.iphone,
                   "-resultBundlePath", str(result_path), "-only-testing:IrisChatUITests/" + TEST,
                   "-parallel-testing-enabled", "NO", "test-without-building"]
        with (self.out / "ios-test.log").open("w") as log:
            self.ios = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        if barrier:
            barrier.start(time.monotonic())
        deadline = time.monotonic() + 420 + self.args.sample_seconds * 3
        while self.ios.poll() is None:
            if time.monotonic() > deadline:
                raise RuntimeError("Physical iPhone test timed out")
            if not self.offline():
                raise RuntimeError("Android gained an IP path or lost Bluetooth during measurement")
            if self.android.poll() is not None:
                raise RuntimeError("Bluetooth peer exited before iPhone idle measurement completed")
            if barrier and barrier.observe((self.out / "ios-test.log").read_text(), time.monotonic()):
                prefix = ["adb", "-s", self.args.android_serial, "shell", "run-as", PACKAGE]
                self.command(prefix + ["test", "!", "-e", barrier.relative_file], "background-release-absent", 5)
                if self.ios.poll() is not None or self.android.poll() is not None:
                    raise RuntimeError("A physical test process exited before background release")
                barrier.observe((self.out / "ios-test.log").read_text(), time.monotonic())
                barrier.release_attempted = True
                self.command(prefix + ["touch", barrier.relative_file], "background-release", 5)
                barrier.mark_released(time.monotonic())
                self.result["backgroundBarrier"] = barrier.evidence()
            time.sleep(5)
        if self.ios.returncode:
            raise RuntimeError("Physical iPhone UI test failed; see ios-test.log")
        if barrier:
            barrier.observe((self.out / "ios-test.log").read_text(), time.monotonic(), completed=True)
        try:
            self.android.wait(timeout=lifetime["postIosWaitSeconds"])
        except subprocess.TimeoutExpired:
            raise RuntimeError("Android did not finish its bounded idle hold") from None
        if self.android.returncode:
            raise RuntimeError("Android receiver process failed; see private log")
        if barrier:
            barrier.acknowledge((self.out / "receiver.log").read_text(), final=True)
        self.command(["xcrun", "xcresulttool", "get", "test-results", "summary",
                      "--path", str(result_path)], "test-summary")
        self.command(["xcrun", "xcresulttool", "get", "test-results", "metrics",
                      "--path", str(result_path)], "metrics")
        self.result.update(evaluate_capture(self.out, self.probe, self.args.sample_seconds,
                                           self.args.max_cpu_percent, self.args.max_write_mib_per_minute,
                                           self.probe + "-reply" if self.args.background else None))

    def restore(self):
        for label, child in (("ios_controller_stopped", self.ios), ("android_controller_stopped", self.android)):
            try:
                stop(child)
            except Exception:
                self.restored[label] = False
        if self.background_release is not None and self.background_release.release_attempted:
            try:
                self.command(["adb", "-s", self.args.android_serial, "shell", "run-as", PACKAGE,
                              "rm", "-f", self.background_release.relative_file], "background-release-cleanup", 5)
                self.restored["background_release_file"] = True
            except Exception:
                self.restored["background_release_file"] = False
        if self.background_release is not None:
            self.result["backgroundBarrier"] = self.background_release.evidence()
        self.saved_account.restore()
        if self.snapshot:
            for key, service in (("wifi_on", "wifi"), ("mobile_data", "data")):
                try:
                    self.adb("svc", service, "enable" if self.snapshot[key] == "1" else "disable")
                    if key == "mobile_data":
                        self.adb("settings", "put", "global", key, self.snapshot[key])
                    deadline = time.monotonic() + 20
                    while self.adb("settings", "get", "global", key) != self.snapshot[key]:
                        if time.monotonic() > deadline:
                            raise RuntimeError("Radio restoration timed out")
                        time.sleep(1)
                    self.restored[key] = True
                except Exception:
                    self.restored[key] = False
            try:
                self.restored["bluetooth_unchanged"] = self.adb("settings", "get", "global", "bluetooth_on") == self.snapshot["bluetooth_on"]
                self.adb("am", "force-stop", PACKAGE)
                self.restored["android_cleanup"] = True
            except Exception:
                self.restored["android_cleanup"] = False
        if self.ios is not None:
            evidence = self.result["iosNormalLaunchSurvival"] = {}
            try:
                verify_ios_launch(self.command, self.iphone, self.out, evidence)
                self.restored["ios_normal_launch"] = True
                self.restored["ios_normal_launch_survived"] = True
            except Exception as error:
                evidence["error"] = str(error)
                self.restored["ios_normal_launch"] = False
                self.restored["ios_normal_launch_survived"] = False
        self.result["restoration"] = self.restored
        if any(v is not True for v in self.restored.values()):
            self.result["ok"] = False
            self.result["restorationError"] = "Some device settings could not be restored; check result.json"
        try:
            write_json(self.out / "result.json", self.result)
        finally:
            for lock in self.locks:
                release(lock)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--iphone", help="Explicit physical iPhone name or identifier")
    p.add_argument("--android-serial", help="Explicit physical Android adb serial")
    p.add_argument("--xctestrun", type=Path, help="Current-source device build-for-testing products")
    p.add_argument("--artifact-dir", type=Path, help="New private directory; existing runs are preserved")
    p.add_argument("--evaluate", type=Path, help="Recheck a saved run without accessing either phone")
    p.add_argument("--output", type=Path, help="New JSON result file for --evaluate; never overwrite evidence")
    p.add_argument("--sample-seconds", type=float, default=60)
    p.add_argument("--background", action="store_true", help="Also require delivery and a receipt while iPhone is backgrounded")
    p.add_argument("--max-cpu-percent", type=float, default=5)
    p.add_argument("--max-write-mib-per-minute", type=float, default=5)
    args = p.parse_args()
    if not 10 <= args.sample_seconds <= 120 or any(not math.isfinite(v) or v <= 0 for v in (args.max_cpu_percent, args.max_write_mib_per_minute)):
        p.error("Use 10–120 second intervals and finite positive budgets")
    os.umask(0o077)
    if args.evaluate:
        if not args.output or any((args.iphone, args.android_serial, args.xctestrun, args.artifact_dir)):
            p.error("--evaluate requires --output and cannot be combined with device-run inputs")
        # Reserve the output before reading evidence so accidental reuse cannot
        # replace original results, including when evaluation fails.
        with args.output.open("x") as output:
            try:
                result = evaluate_saved_run(args.evaluate, args.max_cpu_percent, args.max_write_mib_per_minute)
            except (OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
                result = {"ok": False, "evaluationMode": "saved-capture", "error": str(error)}
            output.write(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result, indent=2))
        return 0 if result["ok"] else 1
    if args.output or not all((args.iphone, args.android_serial, args.xctestrun, args.artifact_dir)):
        p.error("A device run requires --iphone, --android-serial, --xctestrun and --artifact-dir")
    try:
        receiver_lifetime(args.sample_seconds, args.background)
    except ValueError as error:
        p.error(str(error))
    gate = Gate(args)
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt()
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    try:
        gate.execute()
    except (Exception, KeyboardInterrupt) as error:
        gate.result.update(ok=False, error=str(error) or "Interrupted")
    finally:
        gate.restore()
    print(json.dumps(gate.result, indent=2))
    return 0 if gate.result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
