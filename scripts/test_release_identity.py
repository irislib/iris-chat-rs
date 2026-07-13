import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
RELEASE_ALIAS = "irischat"
RELEASE_NPUB = "npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap"
OLD_RELEASE_NPUB = "npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm"


class ReleaseIdentityTests(unittest.TestCase):
    def read(self, relative_path: str) -> str:
        return (ROOT / relative_path).read_text()

    def test_release_scripts_select_the_dedicated_identity(self) -> None:
        common = self.read("scripts/release_common.sh")
        release = self.read("scripts/release")
        homebrew = self.read("packaging/homebrew/publish_tap.sh")
        zapstore = self.read("scripts/publish-zapstore-android.sh")

        self.assertIn(f'IRIS_RELEASE_OWNER_ALIAS_DEFAULT="{RELEASE_ALIAS}"', common)
        self.assertIn(f'IRIS_RELEASE_OWNER_NPUB_DEFAULT="{RELEASE_NPUB}"', common)
        self.assertIn("configure_release_htree_identity", common)
        self.assertIn(
            'IRIS_RELEASE_HTREE_CONFIG_DIR="${IRIS_RELEASE_HTREE_CONFIG_DIR:-$HOME/.hashtree/identities/$IRIS_RELEASE_OWNER_ALIAS}"',
            common,
        )
        self.assertIn("configure_release_htree_identity", release)
        self.assertIn("configure_release_htree_identity", homebrew)
        self.assertIn("IRIS_RELEASE_NOSTR_KEY_PATH", common)
        self.assertIn("configure_release_htree_identity", zapstore)
        self.assertIn("IRIS_RELEASE_NOSTR_KEY_PATH", zapstore)
        self.assertIn(f"htree://{RELEASE_ALIAS}/", homebrew)

    def test_updaters_follow_the_dedicated_release_owner(self) -> None:
        updater_paths = (
            "core/src/desktop_update.rs",
            "core/src/bin/iris_updater/mod.rs",
            "android/app/src/main/java/to/iris/chat/update/AndroidSelfUpdateManager.kt",
        )
        for path in updater_paths:
            with self.subTest(path=path):
                contents = self.read(path)
                self.assertIn(RELEASE_NPUB, contents)
                self.assertNotIn(OLD_RELEASE_NPUB, contents)

    def test_public_release_links_follow_the_dedicated_owner(self) -> None:
        public_paths = (
            "README.md",
            "RELEASE.md",
            "core/README.md",
            "core/Cargo.toml",
            "protocol-ffi/Cargo.toml",
            "chat-protocol/Cargo.toml",
            "docs/release-zapstore.md",
            "ios/Sources/Views.swift",
            "linux/src/screens/settings.rs",
            "windows/IrisChat/Views/SettingsView.xaml.cs",
            "packaging/homebrew/create_tap.sh",
            "packaging/homebrew/README.md",
            "scripts/cli_production_relay_e2e_docker",
            "scripts/test_cli_install_docker",
        )
        for path in public_paths:
            with self.subTest(path=path):
                contents = self.read(path)
                self.assertIn(RELEASE_NPUB, contents)
                self.assertNotIn(OLD_RELEASE_NPUB, contents)


if __name__ == "__main__":
    unittest.main()
