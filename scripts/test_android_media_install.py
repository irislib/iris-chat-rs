"""The actual media install guard must never start a saved Application."""
import hashlib
from pathlib import Path
import tempfile
import unittest

from android_media_install import MediaInstall


class MediaInstallTest(unittest.TestCase):
    def test_old_app_without_background_classes_updates_and_restores_while_disabled(self):
        self.exercise()

    def test_failed_test_apk_install_restores_old_app_and_enabled_state(self):
        self.exercise(fail_test=True)

    def test_explicitly_enabled_app_keeps_its_original_state(self):
        self.exercise(initial_enabled=1)

    def test_component_disable_failure_restores_original_packages(self):
        self.exercise(fail_component=True)

    def test_failed_original_app_restore_keeps_package_disabled(self):
        self.exercise(fail_restore=True)

    def test_fixture_start_failure_never_changes_original_apks(self):
        self.exercise(no_install=True)

    def exercise(self, fail_test=False, fail_component=False, fail_restore=False, initial_enabled=0, no_install=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); app = "to.iris.chat.debug"; test = "to.iris.chat.test"
            files = {}
            for name in ("old-app", "new-app", "old-test", "new-test"):
                files[name] = root / name; files[name].write_text(name)
            installed = {app: files["old-app"], test: files["old-test"]}
            enabled = initial_enabled; components = set(); commands = []; unsafe_starts = []
            def adb(*parts):
                nonlocal enabled
                commands.append(parts)
                if parts[:3] == ("shell", "dumpsys", "package"):
                    return f"User 0: installed=true enabled={enabled}\n" + "\n".join(components)
                if parts[:3] == ("shell", "am", "force-stop"): return ""
                if parts[:3] == ("shell", "pm", "path"):
                    return "package:/data/app/" + parts[-1] + "/base.apk\n"
                if parts[:2] == ("shell", "sha256sum"):
                    package = parts[-1].split("/")[-2]
                    return hashlib.sha256(installed[package].read_bytes()).hexdigest() + " file\n"
                if parts[:2] == ("install", "-r"):
                    path = Path(parts[-1]); package = test if "test" in path.name else app
                    if fail_test and path == files["new-test"]: raise RuntimeError("install failure")
                    if fail_restore and path == files["old-app"]: raise RuntimeError("restore failure")
                    if package == app and enabled != 3: unsafe_starts.append("package replaced")
                    installed[package] = path
                    return "Success"
                operation, target = parts[-4], parts[-1]
                if "/" not in target:
                    enabled = {"disable-user": 3, "default-state": 0, "enable": 1}[operation]
                    return ""
                component = target.split("/", 1)[1]
                assert installed[app] == files["new-app"], "Old app has no background components"
                if operation == "disable":
                    if fail_component and len(components) == 1: raise RuntimeError("component failure")
                    components.add(component)
                elif operation == "default-state": components.discard(component)
                else: raise AssertionError(parts)
                return ""
            backups = [(app, files["new-app"], files["old-app"]), (test, files["new-test"], files["old-test"])]
            guard = MediaInstall(adb, backups, "0", adb("shell", "dumpsys", "package", app))
            try:
                guard.begin()
                if fail_test or fail_component:
                    with self.assertRaisesRegex(RuntimeError, "install failure|component failure"): guard.install()
                elif not no_install:
                    guard.install()
                    self.assertEqual(initial_enabled, enabled)
                    self.assertEqual(2, len(components))
            finally:
                if fail_restore:
                    with self.assertRaisesRegex(RuntimeError, "restore failure"): guard.restore()
                else: guard.restore()
            self.assertEqual([], unsafe_starts)
            if fail_restore:
                self.assertEqual(3, enabled)
                return
            self.assertEqual(initial_enabled, enabled)
            self.assertEqual(set(), components)
            self.assertEqual({app: files["old-app"], test: files["old-test"]}, installed)


if __name__ == "__main__": unittest.main()
