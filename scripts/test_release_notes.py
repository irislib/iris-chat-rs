#!/usr/bin/env python3

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "scripts" / "render-release-notes.py"


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.notes = self.root / "RELEASE_NOTES.md"
        self.notes.write_text(
            "# Iris Chat Release Notes\n\n"
            "## v2026.7.28\n\n"
            "### GitHub\n\n- Technical change.\n\n"
            "### Apple\n\n- Friendly Apple change.\n\n"
            "### Zapstore\n\n- Friendly Android change.\n",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.directory.cleanup()

    def run_renderer(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(RENDERER), "--notes", str(self.notes), *args],
            capture_output=True,
            text=True,
        )

    def test_validates_and_extracts_exact_channel(self) -> None:
        validation = self.run_renderer(
            "--channel", "validate", "--tag", "v2026.7.28"
        )
        self.assertEqual(validation.returncode, 0, validation.stderr)
        apple = self.run_renderer(
            "--channel", "apple", "--tag", "v2026.7.28"
        )
        self.assertEqual(apple.stdout, "- Friendly Apple change.\n")
        self.assertNotIn("Android", apple.stdout)

    def test_requires_every_channel(self) -> None:
        self.notes.write_text(
            "## v2026.7.28\n\n### GitHub\n\n- Change.\n",
            encoding="utf-8",
        )
        result = self.run_renderer(
            "--channel", "validate", "--tag", "v2026.7.28"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Apple, Zapstore", result.stderr)

    def test_rejects_non_release_date_tags(self) -> None:
        result = self.run_renderer(
            "--channel", "validate", "--tag", "v1.2.3"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported release tag", result.stderr)

    def test_accepts_same_day_corrective_tag(self) -> None:
        self.notes.write_text(
            "## v2026.7.28.1\n\n"
            "### GitHub\n\n- Fix.\n\n"
            "### Apple\n\n- Fix.\n\n"
            "### Zapstore\n\n- Fix.\n",
            encoding="utf-8",
        )
        result = self.run_renderer(
            "--channel", "validate", "--tag", "v2026.7.28.1"
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_impossible_date_and_zero_corrective_suffix(self) -> None:
        for tag in ("v2026.2.30", "v2026.7.28.0", "v2026.07.28"):
            with self.subTest(tag=tag):
                result = self.run_renderer(
                    "--channel", "validate", "--tag", tag
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unsupported release tag", result.stderr)

    def test_requires_exact_channel_order_and_names(self) -> None:
        for heading in ("### Zapstore", "### Store"):
            with self.subTest(heading=heading):
                self.notes.write_text(
                    "## v2026.7.28\n\n"
                    f"{heading}\n\n- Change.\n\n"
                    "### GitHub\n\n- Change.\n\n"
                    "### Apple\n\n- Change.\n",
                    encoding="utf-8",
                )
                result = self.run_renderer(
                    "--channel", "validate", "--tag", "v2026.7.28"
                )
                self.assertNotEqual(result.returncode, 0)

    def test_rejects_oversized_apple_notes(self) -> None:
        self.notes.write_text(
            "## v2026.7.28\n\n"
            "### GitHub\n\n- Change.\n\n"
            f"### Apple\n\n{'x' * 4001}\n\n"
            "### Zapstore\n\n- Change.\n",
            encoding="utf-8",
        )
        result = self.run_renderer(
            "--channel", "validate", "--tag", "v2026.7.28"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("4000", result.stderr)

    def test_github_renderer_uses_canonical_assets(self) -> None:
        assets = self.root / "assets"
        assets.mkdir()
        for name in (
            "iris-chat-v2026.7.28-android-arm64.apk",
            "iris-chat-v2026.7.28-macos-arm64.dmg",
            "iris-v2026.7.28-aarch64-apple-darwin.tar.gz",
        ):
            (assets / name).write_bytes(b"asset")
        result = self.run_renderer(
            "--channel",
            "github",
            "--tag",
            "v2026.7.28",
            "--commit",
            "0123456789abcdef",
            "--asset-dir",
            str(assets),
            "--asset-base-url",
            "https://example.invalid/v2026.7.28",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("iris-chat-v2026.7.28-android-arm64.apk", result.stdout)
        self.assertIn("iris-v2026.7.28-aarch64-apple-darwin.tar.gz", result.stdout)
        self.assertIn("GitHub artifact attestations", result.stdout)


if __name__ == "__main__":
    unittest.main()
