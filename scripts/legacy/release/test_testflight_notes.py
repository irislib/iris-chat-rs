#!/usr/bin/env python3

import pathlib
import sys
import tempfile
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from testflight_notes import DEFAULT_WHATS_NEW, resolved_what_to_test


class TestFlightNotesTests(unittest.TestCase):
    def write_notes(self, root: pathlib.Path, text: str) -> None:
        (root / "ZAPSTORE_RELEASE_NOTES.md").write_text(text, encoding="utf-8")

    def test_reads_matching_release_entry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            self.write_notes(
                root,
                """# Iris Chat 2026.5.20.1

- Group messages recover.
- Recovery retries survive restart.

# Iris Chat 2026.5.18.6

- Old entry.
""",
            )

            self.assertEqual(
                resolved_what_to_test("2026.5.20.1", root),
                "- Group messages recover.\n- Recovery retries survive restart.",
            )

    def test_explicit_text_wins(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            self.write_notes(root, "# Iris Chat 2026.5.20.1\n\n- From notes.\n")

            self.assertEqual(
                resolved_what_to_test(
                    "2026.5.20.1",
                    root,
                    explicit="Please test the manual override.",
                ),
                "Please test the manual override.",
            )

    def test_falls_back_when_release_entry_is_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            self.write_notes(root, "# Iris Chat 2026.5.18.6\n\n- Old entry.\n")

            self.assertEqual(resolved_what_to_test("2026.5.20.1", root), DEFAULT_WHATS_NEW)


if __name__ == "__main__":
    unittest.main()
