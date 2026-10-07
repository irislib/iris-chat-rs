"""Keep the saved Application stopped across temporary media-test APK updates."""
import hashlib
import re
import subprocess


def debug_downgrade_packages(aapt2, backups):
    """Explicit opt-in is limited to the paired, debuggable media test apps."""
    installing, restoring = set(), set()
    assert [row[0] for row in backups] == ["to.iris.chat.debug", "to.iris.chat.test"]
    for package, apk, backup in backups:
        versions = []
        for path in (apk, backup):
            output = subprocess.check_output([str(aapt2), "dump", "badging", str(path)], text=True)
            match = re.search(r"^package: name='([^']+)'([^\n]*)", output, re.M)
            assert match and match[1] == package, "Unexpected media APK package"
            assert re.search(r"^application-debuggable(?:\s|$)", output, re.M), "Downgrade requires debuggable APKs"
            version = re.search(r"versionCode='([0-9]+)'", match[2])
            versions.append(int(version[1]) if version else 0)
        if versions[0] < versions[1]: installing.add(package)
        if versions[1] < versions[0]: restoring.add(package)
    return installing, restoring


class MediaInstall:
    def __init__(self, adb, backups, user, package_dump, *, downgrade_install=(), downgrade_restore=()):
        self.adb, self.backups, self.user = adb, backups, user
        self.package = "to.iris.chat.debug"
        assert [row[0] for row in backups] == [self.package, "to.iris.chat.test"]
        self.downgrade_install, self.downgrade_restore = set(downgrade_install), set(downgrade_restore)
        assert self.downgrade_install | self.downgrade_restore <= {self.package, "to.iris.chat.test"}
        assert user.isdecimal()
        self.original_enabled = self.enabled_state(package_dump)
        assert self.original_enabled in (0, 1), "Media test app must be enabled before testing"
        self.components = ("to.iris.chat.push.BackgroundMessageService",
                           "to.iris.chat.push.BackgroundMessageRestoreReceiver")
        for component in self.components:
            assert not re.search(r"^\s+" + re.escape(component) + r"\s*$", package_dump, re.M), (
                "Media harness requires default background component settings")
        self.touched = False
        self.changed = []
        self.disabled = []

    def enabled_state(self, dump=None):
        if dump is None: dump = self.adb("shell", "dumpsys", "package", self.package)
        row = re.search(r"(?m)^\s*User " + self.user + r":([^\n]+)", dump)
        assert row, "Missing exact app user state"
        value = re.search(r"\benabled=(\d+)\b", row[1]); assert value
        return int(value[1])

    def set_enabled(self, state):
        command = {0: "default-state", 1: "enable", 3: "disable-user"}[state]
        self.adb("shell", "pm", command, "--user", self.user, self.package)
        assert self.enabled_state() == state, "Application enabled state did not match"

    def installed_digest(self, package):
        paths = self.adb("shell", "pm", "path", package).strip().splitlines()
        assert len(paths) == 1 and paths[0].startswith("package:")
        path = paths[0][8:]
        assert re.fullmatch(r"/data/app/[-A-Za-z0-9_./=+~]+/base\.apk", path)
        digest = self.adb("shell", "sha256sum", path).split()[0]
        assert re.fullmatch(r"[a-f0-9]{64}", digest)
        return digest

    def begin(self):
        self.adb("shell", "am", "force-stop", self.package)
        self.touched = True
        self.set_enabled(3)

    def install(self):
        assert self.touched and self.enabled_state() == 3
        for package, apk, backup in self.backups:
            self.changed.append((package, backup))  # Roll back even an uncertain install outcome.
            self.adb("install", "-r", *(["-d"] if package in self.downgrade_install else []), str(apk))
            assert self.enabled_state() == 3, "APK update enabled the saved Application"
            assert self.installed_digest(package) == hashlib.sha256(apk.read_bytes()).hexdigest()
        for component in self.components:
            self.disabled.append(component)
            self.adb("shell", "run-as", self.package, "pm", "disable", "--user", self.user,
                     self.package + "/" + component)
        self.set_enabled(self.original_enabled)

    def restore(self):
        if not self.touched: return
        self.adb("shell", "am", "force-stop", self.package)
        self.set_enabled(3)
        # Clear overrides while the new APK still declares these classes. The
        # original APK may predate background receiving and have neither class.
        for component in self.disabled:
            self.adb("shell", "run-as", self.package, "pm", "default-state", "--user", self.user,
                     self.package + "/" + component)
        for package, backup in self.changed:
            self.adb("install", "-r", *(["-d"] if package in self.downgrade_restore else []), str(backup))
            assert self.enabled_state() == 3
        for package, _, backup in self.backups:
            assert self.installed_digest(package) == hashlib.sha256(backup.read_bytes()).hexdigest(), "Original APK was not restored"
        self.set_enabled(self.original_enabled)
        self.adb("shell", "am", "force-stop", self.package)
