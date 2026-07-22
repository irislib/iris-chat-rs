#!/usr/bin/env python3

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "scripts" / "render-github-release-notes.py"


class GitHubReleaseNotesTests(unittest.TestCase):
    def test_renders_matching_changelog_section_and_sorted_assets(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            changelog = temp / "CHANGELOG.md"
            changelog.write_text(
                "# Changelog\n\n"
                "## Unreleased\n\n- Later.\n\n"
                "## 2026.7.22\n\n- First fix.\n- Second fix.\n\n"
                "## 2026.7.21\n\n- Older.\n",
                encoding="utf-8",
            )
            assets = temp / "assets"
            assets.mkdir()
            (assets / "zeta.zip").write_bytes(b"z")
            (assets / "alpha.apk").write_bytes(b"a")
            output = temp / "notes.md"

            subprocess.run(
                [
                    str(RENDERER),
                    "--tag",
                    "v2026.7.22",
                    "--commit",
                    "0123456789abcdef",
                    "--asset-dir",
                    str(assets),
                    "--changelog",
                    str(changelog),
                    "--out",
                    str(output),
                ],
                check=True,
            )

            rendered = output.read_text(encoding="utf-8")
            self.assertIn("# Iris Chat v2026.7.22", rendered)
            self.assertIn("- First fix.\n- Second fix.", rendered)
            self.assertNotIn("Later.", rendered)
            self.assertNotIn("Older.", rendered)
            self.assertLess(rendered.index("`alpha.apk`"), rendered.index("`zeta.zip`"))
            self.assertIn("`0123456789abcdef`", rendered)

    def test_rejects_tag_without_matching_changelog_section(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            changelog = temp / "CHANGELOG.md"
            changelog.write_text("# Changelog\n\n## Unreleased\n", encoding="utf-8")
            assets = temp / "assets"
            assets.mkdir()

            result = subprocess.run(
                [
                    str(RENDERER),
                    "--tag",
                    "v2026.7.22",
                    "--commit",
                    "abc",
                    "--asset-dir",
                    str(assets),
                    "--changelog",
                    str(changelog),
                ],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn("CHANGELOG.md has no release section", result.stderr)


if __name__ == "__main__":
    unittest.main()
