"""Private JSON-line worker for one explicitly reserved macOS GUI login."""
from contextlib import closing
import base64
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import pwd
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time

from evidence import APP, CLI, sibling_retry_state, received_file, require, self_offer, sha256, unpack_product


def until(check, description, seconds=120):
    deadline = time.monotonic() + seconds
    while True:
        result = check()
        if result:
            return result
        require(time.monotonic() < deadline, "Timed out waiting for " + description)
        time.sleep(0.5)


class Actor:
    def __init__(self, root):
        self.root = root
        self.procs = {}
        self.paused = False
        self.owner = None
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("IRIS_")}

    def preflight(self, user, run):
        require(platform.system() == "Darwin" and platform.machine() == "arm64", "Need arm64 macOS")
        require(pwd.getpwuid(os.getuid()).pw_name == user and user != "root" and
                os.stat("/dev/console").st_uid == os.getuid(), "Need the specifically assigned disposable console")
        live = subprocess.run(["pgrep", "-u", str(os.getuid()), "-x", "Iris Chat"], capture_output=True)
        require(live.returncode == 1, "An Iris Chat process is already using this login")
        require(len(run) == 32 and all(c in "0123456789abcdef" for c in run), "Invalid fixture run ID")
        probe = subprocess.run(["osascript", "-l", "JavaScript", "-e",
                                'Application("System Events").applicationProcesses.byName("Dock").attributes.byName("AXRole").value()'],
                               capture_output=True, timeout=25)
        require(probe.returncode == 0 and probe.stdout.strip() == b"AXApplication",
                "Native Accessibility/Automation is unavailable in this login")
        self.run = run
        self.filename = "note-" + run + ".bin"
        self.folder = self.root / "chosen"
        self.folder.mkdir()
        boot = subprocess.check_output(["sysctl", "-n", "kern.bootsessionuuid"]).strip()
        require(boot, "Missing OS boot identity")
        return hashlib.sha256(run.encode() + boot).hexdigest()

    def bind(self, archive, spec, reserve_gib):
        products = unpack_product(Path(archive), self.root, spec, reserve_gib * 2**30)
        self.cli = products / CLI
        app = products / APP
        with (app / "Contents/Info.plist").open("rb") as f:
            name = plistlib.load(f)["CFBundleExecutable"]
        require(name == "Iris Chat", "Unexpected macOS product executable")
        self.app = app / "Contents/MacOS" / name
        require(os.access(self.app, os.X_OK) and os.access(self.cli, os.X_OK), "Missing executable product")
        return {"app": sha256(self.app), "cli": sha256(self.cli)}

    def start_service(self, name):
        require(name in ("main", "late") and name not in self.procs, "Unexpected service name")
        profile = self.root / name
        profile.mkdir()
        path = self.root / (name + ".log")
        with path.open("xb") as log:
            self.procs[name] = subprocess.Popen(
                [self.cli, "--json", "--data-dir", profile, "service", "run"],
                stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=self.env)
        def ready():
            require(self.procs[name].poll() is None, "Synthetic service exited")
            return b'"ready":true' in path.read_bytes()
        until(ready, "synthetic service readiness", 30)

    def cli_call(self, name, *args):
        require(self.procs[name].poll() is None, "Synthetic service exited")
        proc = subprocess.run([self.cli, "--json", "--data-dir", self.root / name,
                               "--no-background-sync", *args], capture_output=True, timeout=45, env=self.env)
        require(proc.returncode == 0, "Synthetic CLI operation failed")
        answer = json.loads(proc.stdout)
        require(answer.get("status") == "ok", "Synthetic CLI operation was rejected")
        return answer["data"]

    def create(self):
        self.start_service("main")
        self.cli_call("main", "account", "create", "--name", "Notes test")
        return self.authorized("main")

    def authorized(self, name):
        def ready():
            account = self.cli_call(name, "state").get("account") or {}
            return account if account.get("device_state") == "authorized" else None
        account = until(ready, "authorized linked account")
        if name == "main":
            self.owner = account["user_id"]
        return {"owner": account["user_id"], "device": account["device_id"]}

    def request(self, name):
        self.start_service(name)
        link = self.cli_call(name, "link", "create")
        if name == "late":
            self.procs[name].send_signal(signal.SIGSTOP)
            self.paused = True
            until(lambda: "T" in subprocess.check_output(["ps", "-p", str(self.procs[name].pid),
                                                          "-o", "state="], text=True), "paused sibling", 5)
        return link["url"]

    def approve(self, uri):
        data = self.cli_call("main", "link", "accept", uri)
        rows = data["device_roster"]["devices"]
        return sorted(r["device_id"] for r in rows if r["authorized"])

    def launch(self, devices):
        profile = self.root / "main"
        def signed_members():
            with closing(sqlite3.connect((profile / "core.sqlite3").resolve().as_uri() + "?mode=ro", uri=True)) as db, db:
                row = db.execute("SELECT devices_json FROM app_keys WHERE owner_pubkey_hex=?", (self.owner,)).fetchone()
            return row and sorted(d["identity_pubkey_hex"] for d in json.loads(row[0])) == sorted(devices)
        until(signed_members, "all three registered devices")
        self.cli_call("main", "service", "stop")
        require(self.procs["main"].wait(timeout=15) == 0, "CLI did not release its profile")
        # Only the newly created local CLI fixture uses this adapter. No profile
        # or long-lived secret crosses between OS instances or enters a receipt.
        bundle = json.loads((profile / "cli-account.json").read_text())
        require(bundle["owner_pubkey_hex"] == self.owner, "CLI fixture belongs to another account")
        with (profile / "account-secret.json").open("x") as f:
            json.dump({"ownerNsec": bundle["owner_nsec"], "ownerPubkeyHex": self.owner,
                       "deviceNsec": bundle["device_nsec"]}, f)
        env = dict(self.env, IRIS_UI_TEST_RUN_ID=self.run, IRIS_UI_TEST_DATA_DIR=str(profile),
                   IRIS_DISABLE_NOTIFICATIONS="1")
        with (self.root / "app.log").open("xb") as log:
            self.procs["app"] = subprocess.Popen([self.app], env=env, stdin=subprocess.DEVNULL,
                                                 stdout=log, stderr=log)
        self.ui("open-self")

    def ui(self, action, *args):
        app = self.procs["app"]
        require(app.poll() is None, "The candidate app is no longer open")
        proc = subprocess.run(["osascript", "-l", "JavaScript", Path(__file__).with_name("mac_ui.js"),
                               str(app.pid), action, *map(str, args)], capture_output=True, timeout=150)
        require(proc.returncode == 0 and proc.stdout.strip() == b"ok", f"Native UI step failed ({action})")

    def send(self):
        path = self.root / self.filename
        with path.open("xb") as out:
            out.write(os.urandom(32771))
        self.ui("send", path)
        return {"size": path.stat().st_size, "sha256": sha256(path)}

    def offer(self, owner, source, name):
        return until(lambda: self_offer(self.root / name, owner, source, self.filename), "file offer")

    def retry(self, owner, source, ready, late, message, stage):
        require(stage in ("receiver-complete", "drained"), "Unknown retry stage")
        until(lambda: sibling_retry_state(self.root / "main", owner, source, ready, late, message) == stage,
              stage + " sender retry state", 90)
        with closing(sqlite3.connect((self.root / "main" / "core.sqlite3").resolve().as_uri() + "?mode=ro",
                                     uri=True)) as db:
            row = db.execute("SELECT devices_json FROM app_keys WHERE owner_pubkey_hex=?", (owner,)).fetchone()
        require(row and {d["identity_pubkey_hex"] for d in json.loads(row[0])} == {source, ready, late},
                "Retry membership changed")

    def screenshot(self, stage):
        pid = self.procs["app"].pid
        tool = Path(__file__).with_name("mac_ui.js")
        result = subprocess.run(["osascript", "-l", "JavaScript", tool, str(pid), "window", ""],
                                capture_output=True, timeout=15)
        require(result.returncode == 0 and result.stdout.strip().isdigit(),
                "Screen Recording for the synthetic app window is unavailable")
        require(stage in ("offered", "completed"), "Unexpected screenshot stage")
        path = self.root / (stage + ".png")
        capture = subprocess.run(["screencapture", "-x", "-o", "-l", result.stdout.strip().decode(), path],
                                 capture_output=True, timeout=15)
        require(capture.returncode == 0 and path.is_file() and 8 < path.stat().st_size < 16 * 2**20,
                "Could not capture candidate window")
        return base64.b64encode(path.read_bytes()).decode()

    def accept(self, offer, screenshots=False):
        require(self.paused and self.procs["late"].poll() is None, "Late sibling was not paused")
        require(not list(self.folder.iterdir()), "Bytes present before acceptance")
        self.ui("offer", offer["transfer"])
        images = {"offered": self.screenshot("offered")} if screenshots else {}
        self.ui("accept", offer["transfer"], self.folder)
        self.ui("completed", offer["transfer"])
        if screenshots:
            images["completed"] = self.screenshot("completed")
        return {"sha256": received_file(self.folder, offer, self.filename), "images": images}

    def resume(self):
        require(self.paused, "Third device is not delayed")
        self.procs["late"].send_signal(signal.SIGCONT)
        self.paused = False
        return self.authorized("late")

    def check_live(self, offer, receiver):
        require(self.procs["app"].poll() is None, "App exited during the proof")
        if receiver:
            self.ui("completed", offer["transfer"])
            return received_file(self.folder, offer, self.filename)
        return True

    def close(self):
        if self.paused and self.procs["late"].poll() is None:
            self.procs["late"].send_signal(signal.SIGCONT)
        for proc in reversed(list(self.procs.values())):
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)


def main():
    os.umask(0o077)
    # macOS Unix socket paths are short. Use fresh paths exclusively.
    with tempfile.TemporaryDirectory(prefix="ntsf-", dir="/tmp") as path:
        actor = Actor(Path(path))
        permitted = {"preflight", "bind", "create", "request", "approve", "authorized", "launch", "send",
                     "offer", "retry", "accept", "resume", "check_live"}
        try:
            for line in sys.stdin:
                command, args = json.loads(line)
                require(command in permitted, "Unknown actor operation")
                result = getattr(actor, command)(*args)
                print(json.dumps({"ok": True, "result": result}), flush=True)
        except Exception as exc:
            # Never echo process arguments, bootstrap links, DB state or keys.
            detail = str(exc) if isinstance(exc, RuntimeError) else type(exc).__name__
            print(json.dumps({"ok": False, "error": detail}), flush=True)
        finally:
            actor.close()


if __name__ == "__main__":
    main()
