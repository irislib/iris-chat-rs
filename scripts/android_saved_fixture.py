"""One private, reusable normal-network sender for an existing saved receiver."""
import hashlib
import json
import os
import queue
import re
import stat
import subprocess
import threading
import time

from android_background_delivery_e2e import wait_for


def normal_fixture_environment(environment, relay):
    assert re.fullmatch(r"ws://127\.0\.0\.1:([0-9]+)", relay)
    assert 0 < int(relay.rsplit(":", 1)[1]) <= 65535
    forbidden = ("IRIS_CALL_ISOLATED_CONTROL", "IRIS_CALL_DESKTOP_CODEC", "IRIS_CALL_PUSH_SERVER_URL",
                 "IRIS_RUNTIME_DEBUG_SNAPSHOT", "IRIS_CHAT_SAME_HOST_HASHTREE")
    assert not any(key in environment for key in forbidden), "Unexpected fixture policy override"
    assert not any(key.startswith(("IRIS_FIPS_", "IRIS_CHAT_FIPS_", "IRIS_UPDATE_")) for key in environment), "Unexpected networking override"
    return {**environment, "IRIS_DEMO_RELAYS": relay}


def private_file(path):
    info = path.lstat()
    assert stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600
    assert 0 < info.st_size <= 8192
    return path.read_bytes()


def validate_pair(value, owner, device, relay, secret_sha):
    assert type(value) is dict and set(value) == {"version", "mode", "receiver", "relay", "sender", "secret_receipt_sha256"}
    assert type(value["version"]) is int and value["version"] == 1 and value["mode"] == "normal"
    assert value["receiver"] == {"owner": owner, "device": device} and value["relay"] == relay
    assert value["secret_receipt_sha256"] == secret_sha
    assert type(value["sender"]) is dict and set(value["sender"]) == {"owner", "device", "device_npub"}
    for key in ("owner", "device"):
        assert re.fullmatch(r"[a-f0-9]{64}", value["sender"][key])
    assert re.fullmatch(r"npub1[a-z0-9]+", value["sender"]["device_npub"])
    return value["sender"]


class SavedFixture:
    def __init__(self, binary, directory, output, owner, device, relay):
        self.binary, self.directory, self.output = binary, directory, output
        self.owner, self.device, self.relay = owner, device, relay
        self.process = self.log = self.reader = None
        self.events = queue.Queue()
        self.ready = None
        self.new_identity = False
        self.checks = {}
        self.messages = []
        self.forced_idle = False

    def start(self):
        environment = normal_fixture_environment(os.environ, self.relay)
        for identity in (self.owner, self.device): assert re.fullmatch(r"[a-f0-9]{64}", identity)
        pair_path = self.directory / "receiver-pair.json"
        secret_path = self.directory / "fixture-account-bundle.json"
        expected = None
        if self.directory.exists():
            info = self.directory.lstat()
            assert stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o700
            # An incomplete prior attempt stays intact; never silently replace its identity.
            secret_sha = hashlib.sha256(private_file(secret_path)).hexdigest()
            expected = validate_pair(json.loads(private_file(pair_path)), self.owner, self.device, self.relay, secret_sha)
            arguments = ["--resume-normal", expected["owner"], expected["device_npub"]]
        else:
            self.directory.mkdir(mode=0o700)
            self.new_identity = True
            arguments = ["--persist-normal"]
        self.log = (self.output / "normal-fixture.log").open("w")
        self.process = subprocess.Popen([str(self.binary), str(self.directory), *arguments], env=environment,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True, bufsize=1)

        def read_events():
            try:
                for line in self.process.stdout:
                    self.events.put(json.loads(line))
            except Exception:
                self.events.put({"event": "fixture-error"})
            finally:
                self.events.put({"event": "fixture-stopped"})
        self.reader = threading.Thread(target=read_events, daemon=True); self.reader.start()
        self.ready = self.event("ready", timeout=60)
        sender = {key: self.ready[key] for key in ("owner", "device", "device_npub")}
        secret_sha = hashlib.sha256(private_file(secret_path)).hexdigest()
        pair = dict(version=1, mode="normal", receiver=dict(owner=self.owner, device=self.device),
                    relay=self.relay, sender=sender, secret_receipt_sha256=secret_sha)
        validate_pair(pair, self.owner, self.device, self.relay, secret_sha)
        if expected is not None:
            assert sender == expected, "Restored sender identity differs"
            assert json.loads(private_file(pair_path))["secret_receipt_sha256"] == secret_sha
        else:
            with os.fdopen(os.open(pair_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as file:
                json.dump(pair, file); file.flush(); os.fsync(file.fileno())
        self.command("accept " + self.owner); self.event("accepted")
        return self.ready

    def command(self, value):
        assert "\n" not in value and "\r" not in value
        assert self.process.poll() is None, "Normal fixture stopped"
        self.process.stdin.write(value + "\n"); self.process.stdin.flush()

    def event(self, kind, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = self.events.get(timeout=max(.01, deadline - time.monotonic()))
            assert value.get("event") not in ("fixture-error", "fixture-stopped"), "Normal fixture failed"
            if value.get("event") == kind: return value
        raise AssertionError("Normal fixture response missing")

    def status(self):
        self.command("status")
        return self.event("status")

    def wait_paired(self):
        wait_for("saved receiver device trusted by fixture", lambda: self.device in self.status()["call_authors"], timeout=90)

    def message(self, stage, notification):
        body = f"saved-fixture-{stage}-{time.time_ns()}"
        self.messages.append(dict(stage=stage, body=body))
        self.command(f"message {self.owner} {body}"); self.event("message-sent")
        wait_for(stage + " message notification", lambda: notification("iris_chat_message_alerts", body), timeout=90)
        self.checks[stage + "_message"] = True

    def ringing(self, kind, notification, stopped):
        assert not notification("incoming-calls"), "A previous incoming call is still present"
        self.command(f"call {self.owner} {kind}")
        wait_for(kind + " ringing", lambda: notification("incoming-calls"))
        self.command("end")
        wait_for(kind + " cancellation", lambda: not notification("incoming-calls"))
        wait_for("call-ended screen returns to background", stopped, timeout=10)

    def before_idle(self, notification, stopped, close_activity):
        self.wait_paired()
        self.message("screen_off", notification)
        for kind in ("voice", "video"):
            self.ringing(kind, notification, stopped)
            self.checks["screen_off_" + kind] = True
            close_activity()

    def after_idle(self, adb, package, notification, stopped, close_activity):
        assert "UPDATES STOPPED" not in adb("shell", "dumpsys", "battery")
        self.forced_idle = True
        try:
            adb("shell", "dumpsys", "battery", "unplug")
            adb("shell", "dumpsys", "deviceidle", "force-idle")
            assert adb("shell", "dumpsys", "deviceidle", "get", "deep").strip() == "IDLE"
            self.message("doze", notification)
            self.ringing("video", notification, stopped)
            self.checks["doze_video"] = True
        finally:
            self.restore_device(adb)
        close_activity()
        old_pid = adb("shell", "pidof", package).strip(); assert old_pid.isdecimal()
        adb("shell", "run-as", package, "kill", "-9", old_pid)
        wait_for("saved receiver process restored", lambda: (value := adb("shell", "pidof", package,
                 check=False).strip()).isdecimal() and value != old_pid, timeout=90)
        wait_for("restored receiving service", lambda: notification("background-receiving"))
        self.message("recreated", notification)

    def restore_device(self, adb):
        if not self.forced_idle: return
        errors = []
        for parts in (("deviceidle", "unforce"), ("battery", "reset")):
            try: adb("shell", "dumpsys", *parts)
            except Exception as error: errors.append(type(error).__name__)
        assert not errors, "Could not restore simulated idle/battery state"
        assert "UPDATES STOPPED" not in adb("shell", "dumpsys", "battery")
        self.forced_idle = False

    def stop(self):
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try: self.process.wait(timeout=10)
            except subprocess.TimeoutExpired: self.process.kill(); self.process.wait(timeout=10)
        if self.reader is not None: self.reader.join(timeout=2)
        if self.process is not None:
            self.process.stdin.close(); self.process.stdout.close()
        if self.log is not None: self.log.close()
