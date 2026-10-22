#!/usr/bin/env python3

import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "scripts/validate-haps-manifest.py"


@unittest.skipUnless(sys.version_info >= (3, 11), "Haps TOML checks require Python 3.11+")
class HapsManifestTests(unittest.TestCase):
    def setUp(self):
        import tomllib

        spec = importlib.util.spec_from_file_location("haps_manifest_check", CHECKER)
        self.checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.checker)
        self.text = (ROOT / "haps.toml").read_text()
        self.manifest = tomllib.loads(self.text)
        self.config = json.loads((ROOT / "haps-release.json").read_text())
        self.tag = next(line[3:] for line in (ROOT / "RELEASE_NOTES.md").read_text().splitlines()
                        if line.startswith("## v"))

    def validate(self, manifest=None, tag=None):
        return self.checker.validate(manifest or self.manifest, self.config, tag or self.tag)

    def test_all_published_package_variants_match_current_release(self):
        self.assertEqual(self.validate(), 6)

    def test_rejects_stale_package_and_target_override_versions(self):
        for package in ("default", "iris", "override"):
            with self.subTest(package=package):
                manifest = copy.deepcopy(self.manifest)
                if package == "default":
                    target = manifest["package"]
                elif package == "iris":
                    target = manifest["packages"]["iris"]["package"]
                else:
                    target = manifest["targets"]["aarch64-apple-darwin"]["package"]
                target["version"] = "2026.1.100"
                with self.assertRaisesRegex(ValueError, "version"):
                    self.validate(manifest)

    def test_rejects_stale_unix_and_windows_build_versions(self):
        for published in self.config["packages"]:
            name, target = published["name"], published["target"]
            with self.subTest(package=name, target=target):
                manifest = copy.deepcopy(self.manifest)
                document = manifest if name == "iris-chat" else manifest["packages"][name]
                command = document["targets"][target]["build"]["commands"][0]
                document["targets"][target]["build"]["commands"][0] = [
                    arg.replace(self.tag[1:], "2026.1.1") for arg in command
                ]
                with self.assertRaisesRegex(ValueError, "IRIS_APP_VERSION_NAME"):
                    self.validate(manifest)

    def test_rejects_stale_windows_version_code(self):
        manifest = copy.deepcopy(self.manifest)
        command = manifest["targets"]["x86_64-pc-windows-msvc"]["build"]["commands"][0]
        command[-1] = command[-1].replace("IRIS_APP_VERSION_CODE = '", "IRIS_APP_VERSION_CODE = '9")
        with self.assertRaisesRegex(ValueError, "IRIS_APP_VERSION_CODE"):
            self.validate(manifest)

    def test_rejects_missing_or_conflicting_version_assignment(self):
        for value in (None, "IRIS_APP_VERSION_NAME=2026.1.1"):
            with self.subTest(value=value):
                manifest = copy.deepcopy(self.manifest)
                command = manifest["targets"]["aarch64-apple-darwin"]["build"]["commands"][0]
                if value is None:
                    command[:] = [arg for arg in command if not arg.startswith("IRIS_APP_VERSION_NAME=")]
                else:
                    command.insert(1, value)
                with self.assertRaisesRegex(ValueError, "IRIS_APP_VERSION_NAME"):
                    self.validate(manifest)

    def test_rejects_changed_published_layout(self):
        for changed in ("command", "missing-desktop", "icon"):
            with self.subTest(changed=changed):
                manifest = copy.deepcopy(self.manifest)
                package = manifest["targets"]["x86_64-pc-windows-msvc"]["package"]
                if changed == "command":
                    package["commands"]["iris-chat"] = "IrisChat/IrisChat.exe"
                elif changed == "missing-desktop":
                    package.pop("desktop", None)
                else:
                    package["desktop"]["icon"] = "IrisChat.ico"
                with self.assertRaisesRegex(ValueError, "metadata"):
                    self.validate(manifest)

    def test_windows_desktop_entry_uses_packaged_application_icon(self):
        target = "x86_64-pc-windows-msvc"
        desktop = {"name": "Iris Chat", "command": "iris-chat", "icon": "Resources/IrisChat.ico"}
        published = next(package for package in self.config["packages"]
                         if package["name"] == "iris-chat" and package["target"] == target)
        self.assertEqual(published.get("desktop"), desktop)
        self.assertEqual(self.manifest["targets"][target]["package"].get("desktop"), desktop)
        project_dir = ROOT / "windows/IrisChat"
        project = ET.parse(project_dir / "IrisChat.csproj").getroot()
        icon = project.findtext("PropertyGroup/ApplicationIcon")
        self.assertEqual(icon.replace("\\", "/"), desktop["icon"])
        content = project.find(f"ItemGroup/Content[@Include='{icon}']")
        self.assertIsNotNone(content, "The shortcut icon must be included in the published app.")
        for field in ("CopyToOutputDirectory", "CopyToPublishDirectory"):
            self.assertEqual(content.findtext(field), "PreserveNewest")
        self.assertTrue((project_dir / desktop["icon"]).is_file())

    def test_validates_date_and_revision_mapping(self):
        self.assertEqual(self.checker.release_versions("v2026.10.22.3"),
                         ((2026, 10, 22, 3), (2026, 10, 2203), 2026102203))
        self.assertEqual(self.checker.release_versions("v2026.10.22"),
                         ((2026, 10, 22, 0), (2026, 10, 2200), 2026102200))
        for tag in ("v2026.2.30", "v2026.10.22.0", "v2026.10.22.100"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                self.checker.release_versions(tag)

    def test_cli_fails_a_deliberately_stale_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "haps.toml"
            manifest.write_text(self.text.replace('version = "', 'version = "9', 1))
            result = subprocess.run([sys.executable, str(CHECKER), "--tag", self.tag,
                                     "--manifest", str(manifest)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("version", result.stderr)


if __name__ == "__main__":
    unittest.main()
