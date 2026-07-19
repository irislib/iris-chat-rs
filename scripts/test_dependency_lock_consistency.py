#!/usr/bin/env python3

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parent.parent
PLATFORM_LOCKS = (ROOT / "core" / "Cargo.lock", ROOT / "linux" / "Cargo.lock")
EXPECTED = {
    "fips-core": "0.4.11",
    "fips-tcp": "0.2.0",
    "fips-tcp-endpoint": "0.2.0",
    "hashtree-config": "0.2.83",
    "hashtree-core": "0.2.86",
    "hashtree-fips-transport": "0.4.9",
    "hashtree-network": "0.2.87",
    "nostr-pubsub": "0.1.13",
    "nostr-pubsub-fips": "0.4.3",
    "nostr-pubsub-relay": "0.1.11",
    "nostr-pubsub-social-graph": "0.2.2",
    "nostr-social-graph": "0.1.4",
}


def package_versions(lock_path: Path) -> dict[str, set[str]]:
    versions: dict[str, set[str]] = {}
    for block in lock_path.read_text(encoding="utf-8").split("[[package]]"):
        name = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
        version = re.search(r'^version = "([^"]+)"$', block, re.MULTILINE)
        if name and version:
            versions.setdefault(name.group(1), set()).add(version.group(1))
    return versions


class DependencyLockConsistencyTests(unittest.TestCase):
    def test_shipping_platform_locks_use_release_dependency_tuple(self):
        for lock_path in PLATFORM_LOCKS:
            with self.subTest(lock=lock_path.relative_to(ROOT)):
                versions = package_versions(lock_path)
                for package, expected in EXPECTED.items():
                    self.assertEqual(versions.get(package), {expected}, f"{package} in {lock_path}")
                self.assertIn(
                    "0.4.0",
                    versions.get("nostr-identity", set()),
                    f"direct nostr-identity release in {lock_path}",
                )

    def test_core_manifest_pins_gated_fips_stack_exactly(self):
        manifest = (ROOT / "core" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertRegex(manifest, r'(?m)^fips-core = \{ version = "=0\.4\.11",', "fips-core must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^fips-tcp = "0\.2\.0"$', "fips-tcp must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^fips-tcp-endpoint = "0\.2\.0"$', "fips-tcp-endpoint must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-config = "=0\.2\.83"$', "Hashtree config must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-core = "=0\.2\.86"$', "Hashtree core must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-fips-transport = "=0\.4\.9"$', "Hashtree/FIPS transport must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-network = "=0\.2\.87"$', "Hashtree network must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-identity = "=0\.4\.0"$', "nostr-identity must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub = "=0\.1\.13"$', "nostr-pubsub must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub-fips = "=0\.4\.3"$', "nostr-pubsub-fips must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub-relay = "=0\.1\.11"$', "nostr-pubsub-relay must stay on the gated release")

    def test_linux_build_inherits_the_pinned_core_manifest(self):
        manifest = (ROOT / "linux" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertRegex(
            manifest,
            r'(?m)^iris-chat-core = \{ package = "iris-chat", path = "\.\./core" \}$',
            "Linux must build the same pinned core dependency graph",
        )


if __name__ == "__main__":
    unittest.main()
