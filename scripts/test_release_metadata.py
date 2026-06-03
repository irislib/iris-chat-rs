#!/usr/bin/env python3

import pathlib
import unittest


ROOT_DIR = pathlib.Path(__file__).resolve().parents[1]


class ReleaseMetadataTests(unittest.TestCase):
    def test_linux_deb_uses_embedded_updater(self) -> None:
        cargo = (ROOT_DIR / "linux" / "Cargo.toml").read_text()

        self.assertIn('["target/release/iris-chat", "usr/bin/", "755"]', cargo)
        self.assertNotIn('["target/release/iris", "usr/bin/iris", "755"]', cargo)

    def test_linux_release_does_not_stage_update_helper(self) -> None:
        script = (ROOT_DIR / "scripts" / "linux-release").read_text()

        self.assertNotIn("IRIS_HELPER_PATH", script)
        self.assertNotIn('"$BUNDLE/iris"', script)

    def test_linux_update_check_uses_embedded_core(self) -> None:
        settings = (ROOT_DIR / "linux" / "src" / "screens" / "settings.rs").read_text()

        self.assertIn("iris_chat_core::iris_desktop_update_check", settings)
        self.assertNotIn("iris_update_helper_path", settings)


if __name__ == "__main__":
    unittest.main()
