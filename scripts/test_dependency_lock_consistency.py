#!/usr/bin/env python3

from pathlib import Path
import re
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
PLATFORM_LOCKS = (ROOT / "core" / "Cargo.lock", ROOT / "linux" / "Cargo.lock")
EXPECTED = {
    "hashtree-blossom": "0.2.83",
    "hashtree-resolver": "0.2.86",
    "hashtree-updater": "0.2.88",
    "nvpn-fips-core": "0.4.95",
    "nvpn-fips-endpoint": "0.4.95",
    "nvpn-fips-tcp": "0.2.5",
    "nvpn-fips-tcp-endpoint": "0.2.30",
    "hashtree-config": "0.2.83",
    "hashtree-core": "0.2.91",
    "hashtree-fips-transport": "0.4.28",
    "hashtree-network": "0.2.88",
    "nostr-pubsub": "0.1.16",
    "nostr-pubsub-fips": "0.5.22",
    "nostr-pubsub-relay": "0.1.13",
    "nostr-pubsub-social-graph": "0.2.3",
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


def local_protocol_issues(root: Path) -> list[str]:
    manifest = (root / "chat-protocol" / "Cargo.toml").read_text(encoding="utf-8")
    package = re.search(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)", manifest)
    version = re.search(r'^version\s*=\s*"([^"]+)"$', package.group(1), re.MULTILINE) if package else None
    if version is None:
        return ["chat-protocol/Cargo.toml must declare its package version"]
    expected = version.group(1)
    issues = []
    for consumer in ("core", "protocol-ffi"):
        manifest_path = root / consumer / "Cargo.toml"
        dependency = re.search(
            r'(?m)^iris-chat-protocol\s*=\s*\{([^}]+)\}',
            manifest_path.read_text(encoding="utf-8"),
        )
        for field, value in (("version", f"={expected}"), ("path", "../chat-protocol")):
            actual = re.search(rf'\b{field}\s*=\s*"([^"]+)"', dependency.group(1)) if dependency else None
            if actual is None or actual.group(1) != value:
                issues.append(f'{consumer}/Cargo.toml iris-chat-protocol {field} must be "{value}"')
    for consumer in ("core", "protocol-ffi", "chat-protocol", "linux"):
        lock_path = root / consumer / "Cargo.lock"
        if package_versions(lock_path).get("iris-chat-protocol") != {expected}:
            issues.append(f"{consumer}/Cargo.lock iris-chat-protocol must resolve only {expected}")
    return issues


class DependencyLockConsistencyTests(unittest.TestCase):
    def test_native_roots_use_the_same_vendored_ice_fix(self):
        vendor = ROOT / "core" / "vendor" / "webrtc-ice"
        provenance = (vendor / "UPSTREAM.toml").read_text(encoding="utf-8")
        self.assertIn('version = "0.17.1"', provenance)
        self.assertIn(
            'crate_sha256 = "23ede72a36e5dda685814c389b2b34ac60b3ed000a81789e93626e27180eb785"',
            provenance,
        )
        for license_name in ("LICENSE-MIT", "LICENSE-APACHE"):
            self.assertTrue((vendor / license_name).is_file())
        for consumer in ("core", "linux"):
            with self.subTest(consumer=consumer):
                manifest = (ROOT / consumer / "Cargo.toml").read_text(encoding="utf-8")
                patch = re.search(r"(?ms)^\[patch\.crates-io\]\s*\n(.*?)(?=^\[|\Z)", manifest)
                self.assertIsNotNone(patch, "Every native Cargo root must apply the ICE patch")
                dependency = re.search(r'(?m)^webrtc-ice\s*=\s*\{\s*path\s*=\s*"([^"]+)"\s*\}', patch.group(1))
                self.assertIsNotNone(dependency)
                self.assertEqual((ROOT / consumer / dependency.group(1)).resolve(), vendor.resolve())
                entries = [block for block in (ROOT / consumer / "Cargo.lock").read_text().split("[[package]]")
                           if re.search(r'(?m)^name = "webrtc-ice"$', block)]
                self.assertEqual(len(entries), 1, "Do not ship a second unpatched ICE version")
                self.assertRegex(entries[0], r'(?m)^version = "0\.17\.1"$')
                self.assertNotRegex(entries[0], r"(?m)^(source|checksum) =", "ICE must resolve to the local patch")

    def test_local_protocol_manifests_and_locks_match_package_version(self):
        self.assertEqual(local_protocol_issues(ROOT), [])

    def test_local_protocol_check_rejects_stale_pins_and_locks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for consumer in ("core", "protocol-ffi", "chat-protocol", "linux"):
                (root / consumer).mkdir()
                (root / consumer / "Cargo.lock").write_text(
                    '[[package]]\nname = "iris-chat-protocol"\nversion = "0.3.0"\n',
                    encoding="utf-8",
                )
            protocol = root / "chat-protocol" / "Cargo.toml"
            protocol.write_text('[package]\nname = "iris-chat-protocol"\nversion = "0.3.0"\n', encoding="utf-8")
            dependency = 'iris-chat-protocol = { version = "=0.3.0", path = "../chat-protocol" }\n'
            for consumer in ("core", "protocol-ffi"):
                (root / consumer / "Cargo.toml").write_text(dependency, encoding="utf-8")
            self.assertEqual(local_protocol_issues(root), [])
            ffi = root / "protocol-ffi" / "Cargo.toml"
            for stale in ("0.2.3", "^0.2.3", "0.3.0"):
                with self.subTest(ffi_constraint=stale):
                    ffi.write_text(dependency.replace("=0.3.0", stale), encoding="utf-8")
                    self.assertEqual(local_protocol_issues(root), [
                        'protocol-ffi/Cargo.toml iris-chat-protocol version must be "=0.3.0"',
                    ])
            ffi.write_text(dependency, encoding="utf-8")
            ffi_lock = root / "protocol-ffi" / "Cargo.lock"
            ffi_lock.write_text(ffi_lock.read_text(encoding="utf-8").replace("0.3.0", "0.2.3"), encoding="utf-8")
            self.assertEqual(local_protocol_issues(root), [
                "protocol-ffi/Cargo.lock iris-chat-protocol must resolve only 0.3.0",
            ])
            protocol.write_text(protocol.read_text(encoding="utf-8").replace("0.3.0", "0.3.1"), encoding="utf-8")
            issues = local_protocol_issues(root)
            self.assertEqual(len(issues), 6, "A package bump must update both pins and all four locks")
            self.assertTrue(all("0.3.1" in issue for issue in issues))

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
        self.assertRegex(manifest, r'(?m)^nvpn-fips-core = \{ version = "=0\.4\.95",', "fips-core must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-endpoint = \{ version = "=0\.4\.95" \}$', "fips-endpoint must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-tcp = \{ version = "=0\.2\.5" \}$', "fips-tcp must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-tcp-endpoint = \{ version = "=0\.2\.30" \}$', "fips-tcp-endpoint must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-config = "=0\.2\.83"$', "Hashtree config must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-core = "=0\.2\.91"$', "Hashtree core must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-fips-transport = "=0\.4\.28"$', "Hashtree/FIPS transport must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-network = "=0\.2\.88"$', "Hashtree network must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-identity = "=0\.4\.0"$', "nostr-identity must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub = "=0\.1\.16"$', "nostr-pubsub must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub-fips = "=0\.5\.22"$', "nostr-pubsub-fips must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub-relay = "=0\.1\.13"$', "nostr-pubsub-relay must stay on the gated release")

    def test_linux_build_inherits_the_pinned_core_manifest(self):
        manifest = (ROOT / "linux" / "Cargo.toml").read_text(encoding="utf-8")
        core = re.search(r'(?m)^iris-chat-core\s*=\s*\{([^}]+)\}', manifest)
        self.assertIsNotNone(core, "Linux must declare its core dependency")
        dependency = core.group(1)
        self.assertRegex(dependency, r'\bpackage\s*=\s*"iris-chat"')
        self.assertRegex(dependency, r'\bpath\s*=\s*"\.\./core"', "Linux must use the pinned local core")
        self.assertRegex(dependency, r'\bfeatures\s*=\s*\[[^\]]*"desktop-media"', "Linux releases must include calling")


if __name__ == "__main__":
    unittest.main()
