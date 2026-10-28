#!/usr/bin/env python3
"""Exercise Android input guards with small local fixtures; no installers run."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("android_release_inputs", ROOT / "scripts/android-release-inputs.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
PINS = json.loads(MODULE.PINS.read_text())


class AndroidReleaseInputTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def zip_fixture(self, extras=(), source_properties="Pkg.Revision=2\nAndroidVersion.ApiLevel=36\n"):
        path = self.root / "sdk.zip"
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("android-fixture/source.properties", source_properties)
            info = zipfile.ZipInfo("android-fixture/bin/tool")
            info.external_attr = (stat.S_IFREG | 0o755) << 16
            archive.writestr(info, b"never execute")
            for name, content, mode in extras:
                info = zipfile.ZipInfo(name)
                info.external_attr = mode << 16
                archive.writestr(info, content)
        return path, {**PINS["sdk_linux"][0], "bytes": path.stat().st_size,
                      "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}

    def crate_fixture(self, link=False, bad_lock=False):
        path = self.root / "crate.tar.gz"
        root = "cargo-ndk-4.1.2"
        lock = b"fixture frozen registry lock\n"
        with tarfile.open(path, "w:gz") as archive:
            for name, content in (("Cargo.toml", b"fixture manifest"), ("Cargo.lock", lock)):
                info = tarfile.TarInfo(root + "/" + name)
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))
            if link:
                info = tarfile.TarInfo(root + "/link")
                info.type = tarfile.SYMTYPE
                info.linkname = "Cargo.lock"
                archive.addfile(info)
        return path, {**PINS["cargo_ndk"], "bytes": path.stat().st_size,
                      "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                      "bundled_lock": {"path": root + "/Cargo.lock", "bytes": len(lock),
                                       "sha256": "b" * 64 if bad_lock else hashlib.sha256(lock).hexdigest()}}

    def test_reviewed_pins_are_complete_and_no_latest_resolution_remains(self):
        MODULE.check_pins(PINS)
        self.assertEqual(PINS["jdk"]["version"], "17.0.20+8")
        self.assertEqual(PINS["jdk"]["sha256"], "be7668bc030d578b83d6d5ef9221d6d6729bbbca8cf94a7d52e16ac68b5a5a35")
        self.assertEqual([p["sha256"] for p in PINS["sdk_linux"]], [
            "37607369a28c5b640b3a7998868d45898ebcb777565a0e85f9acf36f29631d2e",
            "5d9ac77fb6ff43d9da518a337b4fcf8f9097113df531d99ccefe80ef7ce8250b",
            "dfb20d396df28ca02a8c708314b814a4d961dc9074f9a161932746f815aa552f"])
        for key, change in (("jdk", {"version": "17"}), ("cargo_ndk", {"sha256": ""})):
            pins = copy.deepcopy(PINS)
            pins[key].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                MODULE.check_pins(pins)

    def test_sdk_verifies_before_parse_and_preserves_safe_internal_links_and_modes(self):
        path, pin = self.zip_fixture([("android-fixture/bin/alias", b"tool", stat.S_IFLNK | 0o777)])
        root = MODULE.unpack_sdk(path, pin, self.root / "good")
        self.assertEqual((root / "bin/tool").stat().st_mode & 0o777, 0o755)
        self.assertTrue((root / "bin/alias").is_symlink())
        self.assertEqual((root / "bin/alias").read_bytes(), b"never execute")
        path.write_bytes(b"tampered")
        with patch.object(MODULE.zipfile, "ZipFile") as parser:
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                MODULE.unpack_sdk(path, pin, self.root / "bad")
            parser.assert_not_called()
        self.assertFalse((self.root / "bad").exists())

    def test_sdk_rejects_path_escape_special_files_and_link_traversal(self):
        cases = [[("../escape", b"bad", stat.S_IFREG | 0o644)],
                 [("/absolute", b"bad", stat.S_IFREG | 0o644)],
                 [("android-fixture/fifo", b"", stat.S_IFIFO | 0o600)],
                 [("android-fixture/link", b"../outside", stat.S_IFLNK | 0o777)],
                 [("android-fixture/link", b"/outside", stat.S_IFLNK | 0o777)],
                 [("android-fixture/link", b"bin", stat.S_IFLNK | 0o777),
                  ("android-fixture/link/child", b"bad", stat.S_IFREG | 0o644)]]
        for index, extras in enumerate(cases):
            with self.subTest(extras=extras):
                path, pin = self.zip_fixture(extras)
                with self.assertRaises(ValueError):
                    MODULE.unpack_sdk(path, pin, self.root / str(index))
                self.assertFalse((self.root / str(index)).exists())

    def test_sdk_requires_exact_revision_and_api_level(self):
        path, pin = self.zip_fixture(source_properties="Pkg.Revision=3\nAndroidVersion.ApiLevel=36\n")
        with self.assertRaisesRegex(ValueError, "source.properties"):
            MODULE.unpack_sdk(path, pin, self.root / "wrong-sdk")

    @unittest.skipUnless(hasattr(tarfile, "data_filter"), "hosted installer requires Python tarfile.data_filter")
    def test_crate_requires_bundled_lock_and_rejects_links(self):
        for index, (link, wrong_lock) in enumerate(((False, False), (True, False), (False, True))):
            path, pin = self.crate_fixture(link, wrong_lock)
            destination = self.root / str(index)
            if link or wrong_lock:
                with self.assertRaises(ValueError):
                    MODULE.unpack_cargo_ndk(path, pin, destination)
            else:
                source = MODULE.unpack_cargo_ndk(path, pin, destination)
                self.assertEqual(MODULE.digest(source / "Cargo.lock"), pin["bundled_lock"]["sha256"])

    def test_crate_checksum_failure_precedes_parsing(self):
        path, pin = self.crate_fixture()
        path.write_bytes(b"tampered")
        with patch.object(MODULE.tarfile, "open") as parser:
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                MODULE.unpack_cargo_ndk(path, pin, self.root / "bad")
            parser.assert_not_called()

    def test_no_installation_or_download_outside_hosted_linux(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}), \
                patch.object(MODULE.SHARED, "download_native") as download:
            with self.assertRaisesRegex(ValueError, "hosted Linux"):
                MODULE.install(PINS, self.root / "work", self.root / "rust")
            download.assert_not_called()

    def test_wrong_rust_receipt_or_binary_cannot_launch_tools(self):
        rust = self.root / "rust"
        rust.mkdir()
        receipt = {"passed": False}
        (rust / "receipt.json").write_text(json.dumps(receipt))
        with patch.object(MODULE.SHARED, "validate_install") as check, \
                patch.object(MODULE.SHARED, "checkout_commit", return_value="a" * 40):
            with self.assertRaisesRegex(ValueError, "successful pinned Rust"):
                MODULE.rust_environment(rust)
            receipt.update(passed=True, sourceCommit="a" * 40,
                           pinsSHA256=MODULE.digest(MODULE.SHARED.PINS),
                           installerSHA256=MODULE.digest(ROOT / "scripts/release-build-inputs.py"))
            (rust / "receipt.json").write_text(json.dumps(receipt))
            with patch.dict(os.environ, {"CARGO": "/wrong/cargo"}), \
                    self.assertRaisesRegex(ValueError, "executable differs"):
                MODULE.rust_environment(rust)
            check.assert_not_called()

    def test_host_prerequisites_ignore_ambient_cmake_and_pkg_config(self):
        with patch.object(MODULE.shutil, "which", side_effect=lambda name: "/ambient/bin/" + name), \
                patch.object(MODULE.Path, "is_file", return_value=True), \
                patch.object(MODULE.os, "access", return_value=True), \
                patch.object(MODULE, "digest", return_value="a" * 64), \
                patch.object(MODULE.subprocess, "check_output", return_value="fixture version") as probe:
            versions = MODULE.host_prerequisites()
        self.assertEqual(versions["cmake"]["path"], "/usr/bin/cmake")
        self.assertEqual(versions["pkg-config"]["path"], "/usr/bin/pkg-config")
        commands = [call.args[0] for call in probe.call_args_list]
        self.assertIn(["/usr/bin/cmake", "--version"], commands)
        self.assertIn(["/usr/bin/pkg-config", "--version"], commands)
        for package in ("cmake", "libdbus-1-dev", "pkg-config", "pkgconf", "pkgconf-bin"):
            self.assertIn(package, commands[0])

    @unittest.skipUnless(hasattr(tarfile, "data_filter"), "hosted installer requires Python tarfile.data_filter")
    def test_install_uses_verified_path_locked_cargo_and_exports_only_after_success(self):
        work = self.root / "work"
        envfile, pathfile = self.root / "env", self.root / "path"
        cached_artifact = self.root / "home/.gradle/caches/modules-2/fixture.jar"
        cached_artifact.parent.mkdir(parents=True)
        cached_artifact.write_bytes(b"retained dependency")
        generated_state = self.root / "home/.gradle/caches/9.3.1/kotlin-dsl/stale"
        generated_state.parent.mkdir(parents=True)
        generated_state.write_bytes(b"previous generated state")
        pins = copy.deepcopy(PINS)
        pins["sdk_linux"] = [pins["sdk_linux"][0]]
        path, crate = self.crate_fixture()
        pins["cargo_ndk"] = crate
        cargo_command = []
        def download(pin, destination):
            if destination.name == "cargo-ndk.crate":
                destination.write_bytes(path.read_bytes())
            else:
                destination.write_bytes(b"fixture")
            return destination
        def unpack_jdk(archive, pin, destination):
            home = destination / pin["root"]
            (home / "bin").mkdir(parents=True)
            (home / "bin/java").write_text("fixture")
            (home / "release").write_text('JAVA_RUNTIME_VERSION="17.0.20+8"\n')
            return home / "bin/java"
        def unpack_sdk(archive, pin, destination):
            destination.mkdir()
            (destination / "source.properties").write_text("fixture")
            return destination
        def cargo(command, **kwargs):
            cargo_command.extend(command)
            self.assertEqual(kwargs["timeout"], 900)
            self.assertEqual(kwargs["env"]["CMAKE"], "/usr/bin/cmake")
            self.assertEqual(kwargs["env"]["PKG_CONFIG"], "/usr/bin/pkg-config")
            self.assertFalse(envfile.exists())
            binary = work / "cargo-ndk/bin/cargo-ndk"
            binary.parent.mkdir(parents=True)
            binary.write_text("fixture")
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_ENV": str(envfile), "GITHUB_PATH": str(pathfile)}), \
                patch.object(MODULE.Path, "home", return_value=self.root / "home"), \
                patch.object(MODULE.platform, "system", return_value="Linux"), \
                patch.object(MODULE.platform, "machine", return_value="x86_64"), \
                patch.object(MODULE, "check_pins"), \
                patch.object(MODULE, "rust_environment", return_value=({"CARGO": "/verified/bin/cargo"}, "b" * 64)), \
                patch.object(MODULE, "host_prerequisites", return_value={}), \
                patch.object(MODULE.SHARED, "checkout_commit", return_value="a" * 40), \
                patch.object(MODULE.SHARED, "download_native", side_effect=download), \
                patch.object(MODULE.SHARED, "unpack_native", side_effect=unpack_jdk), \
                patch.object(MODULE, "unpack_sdk", side_effect=unpack_sdk), \
                patch.object(MODULE.subprocess, "run", side_effect=cargo), \
                patch.object(MODULE.subprocess, "check_output", side_effect=["OpenJDK Runtime 17.0.20+8", "cargo-ndk 4.1.2"]):
            MODULE.install(pins, work, self.root / "rust")
        self.assertEqual(cargo_command, ["/verified/bin/cargo", "install", "--path", str(work / "cargo-ndk-source/cargo-ndk-4.1.2"),
                                         "--locked", "--root", str(work / "cargo-ndk"), "--jobs", "2"])
        self.assertIn("ANDROID_HOME=" + str(work / "sdk"), envfile.read_text())
        self.assertIn("GRADLE_USER_HOME=" + str(work / "gradle-home"), envfile.read_text())
        self.assertIn("CMAKE=/usr/bin/cmake\n", envfile.read_text())
        self.assertIn("PKG_CONFIG=/usr/bin/pkg-config\n", envfile.read_text())
        cache = work / "gradle-home/caches"
        self.assertTrue(cache.is_dir())
        self.assertFalse(cache.is_symlink())
        self.assertTrue((cache / "modules-2").is_symlink())
        self.assertTrue((cache / "modules-2/fixture.jar").samefile(cached_artifact))
        self.assertFalse((cache / "9.3.1").exists())
        self.assertEqual(generated_state.read_bytes(), b"previous generated state")
        self.assertFalse((work / "gradle-home/wrapper").exists())
        self.assertTrue(json.loads((work / "receipt.json").read_text())["passed"])

    def test_workflow_uses_one_pinned_installer_after_cache_and_keeps_receipts(self):
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        android = workflow.split("\n  android:\n", 1)[1].split("\n  ios:\n", 1)[0]
        for floating in ("setup-java@", "sdkmanager", "cmdline-tools/latest", "cargo install cargo-ndk", "apt-get update"):
            self.assertNotIn(floating, android)
        self.assertIn("pinned-apt.sh ubuntu-noble cmake libdbus-1-dev pkg-config pkgconf pkgconf-bin", android)
        self.assertIn("android-release-inputs.py --install", android)
        self.assertIn("iris-release-android/receipt.json", android)
        self.assertLess(android.index("Cache Cargo and Gradle"), android.index("Install checksum-verified Android"))
        self.assertIn("android/gradle/verification-metadata.xml", android)
        self.assertIn("android/gradle/wrapper/gradle-wrapper.properties", android)
        self.assertIn("android/gradle/wrapper/gradle-wrapper.jar", android)


if __name__ == "__main__":
    unittest.main()
