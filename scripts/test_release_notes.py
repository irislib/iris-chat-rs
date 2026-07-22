#!/usr/bin/env python3

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "scripts" / "render-release-notes.py"


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.temp = Path(self.temp_dir.name)
        self.changelog = self.temp / "CHANGELOG.md"
        self.changelog.write_text(
            "# Changelog\n\n"
            "## Unreleased\n\n- Later.\n\n"
            "## 2026.7.22\n\n- First fix.\n- Second fix.\n\n"
            "## 2026.7.21\n\n- Older.\n",
            encoding="utf-8",
        )
        self.assets = self.temp / "assets"
        self.assets.mkdir()

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def render(self, *extra_args: str) -> str:
        output = self.temp / "notes.md"
        subprocess.run(
            [
                str(RENDERER),
                "--tag",
                "v2026.7.22",
                "--commit",
                "0123456789abcdef",
                "--asset-dir",
                str(self.assets),
                "--changelog",
                str(self.changelog),
                *extra_args,
                "--out",
                str(output),
            ],
            check=True,
        )
        return output.read_text(encoding="utf-8")

    def add_asset(self, name: str) -> None:
        (self.assets / name).write_bytes(b"asset")

    def test_groups_everyday_downloads_before_advanced_files(self) -> None:
        names = [
            "IrisChat-release-2026.7.22+2026072200-0123456789ab.aab",
            "IrisChat-release-2026.7.22+2026072200-0123456789ab.apk",
            "IrisChat.ipa",
            "iris-chat-v2026.7.22-ios.xcarchive.zip",
            "iris-chat-v2026.7.22-linux-x64.deb",
            "iris-chat-v2026.7.22-linux-x64.tar.gz",
            "iris-chat-v2026.7.22-macos-arm64.app.tar.gz",
            "iris-chat-v2026.7.22-macos-arm64.dmg",
            "iris-chat-v2026.7.22-windows-x64-setup.exe",
            "iris-chat-v2026.7.22-windows-x64.zip",
            "iris-aarch64-apple-darwin.tar.gz",
            "iris-x86_64-apple-darwin.tar.gz",
            "iris-x86_64-unknown-linux-gnu.tar.gz",
        ]
        for name in reversed(names):
            self.add_asset(name)

        rendered = self.render()
        most_people = rendered.split("### Most People Will Want", 1)[1].split("### ", 1)[0]

        self.assertIn("Iris Chat for macOS (Apple Silicon)", most_people)
        self.assertIn("Iris Chat for Windows", most_people)
        self.assertIn("Iris Chat for Android", most_people)
        self.assertIn("Iris Chat for Debian/Ubuntu (.deb)", most_people)
        self.assertNotIn("App Bundle", most_people)
        self.assertNotIn("Xcode archive", most_people)
        self.assertIn("### Command Line", rendered)
        self.assertIn("### Other Files", rendered)
        self.assertIn("Android App Bundle", rendered)
        self.assertIn("iPhone/iPad IPA", rendered)
        self.assertIn("iOS Xcode archive", rendered)

    def test_self_publish_uses_relative_links_and_release_details(self) -> None:
        self.add_asset("iris-chat-v2026.7.22-macos-arm64.dmg")
        rendered = self.render(
            "--install-url",
            "https://upload.iris.to/releases/iris-chat-rs/v2026.7.22/install.sh",
            "--built-line",
            "Built signed macOS app.",
            "--skipped-line",
            "iOS was not requested.",
        )

        self.assertIn(
            "[iris-chat-v2026.7.22-macos-arm64.dmg](assets/iris-chat-v2026.7.22-macos-arm64.dmg)",
            rendered,
        )
        self.assertIn("curl -fsSL https://upload.iris.to/", rendered)
        self.assertIn("## Changes\n\n- First fix.\n- Second fix.", rendered)
        self.assertNotIn("Later.", rendered)
        self.assertNotIn("Older.", rendered)
        self.assertIn("Built signed macOS app.", rendered)
        self.assertIn("## Skipped or Not Built", rendered)

    def test_github_publish_uses_full_links_and_attestation_note(self) -> None:
        self.add_asset("iris-chat-v2026.7.22-windows-x64-setup.exe")
        self.add_asset("IrisChat-release-2026.7.22+2026072200-0123456789ab.apk")
        rendered = self.render(
            "--asset-base-url",
            "https://github.com/irislib/iris-chat-rs/releases/download/v2026.7.22",
            "--verification-line",
            "GitHub artifact attestations record build provenance for the release files.",
        )

        self.assertIn(
            "[iris-chat-v2026.7.22-windows-x64-setup.exe]"
            "(https://github.com/irislib/iris-chat-rs/releases/download/v2026.7.22/"
            "iris-chat-v2026.7.22-windows-x64-setup.exe)",
            rendered,
        )
        self.assertIn("Iris Chat for Android", rendered)
        self.assertIn("2026.7.22%2B2026072200-0123456789ab.apk", rendered)
        self.assertIn("## Verification", rendered)
        self.assertIn("GitHub artifact attestations", rendered)
        self.assertIn("`0123456789abcdef`", rendered)

    def test_rejects_tag_without_matching_changelog_section(self) -> None:
        self.changelog.write_text("# Changelog\n\n## Unreleased\n", encoding="utf-8")

        result = subprocess.run(
            [
                str(RENDERER),
                "--tag",
                "v2026.7.22",
                "--commit",
                "abc",
                "--asset-dir",
                str(self.assets),
                "--changelog",
                str(self.changelog),
            ],
            capture_output=True,
            text=True,
        )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("CHANGELOG.md has no release section", result.stderr)


if __name__ == "__main__":
    unittest.main()
