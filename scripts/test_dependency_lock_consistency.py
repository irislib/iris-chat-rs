#!/usr/bin/env python3

from pathlib import Path
import re
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
PLATFORM_LOCKS = (ROOT / "core" / "Cargo.lock", ROOT / "linux" / "Cargo.lock")
EXPECTED = {
    "hashtree-blossom": "0.3.0",
    "hashtree-resolver": "0.3.0",
    "hashtree-updater": "0.3.0",
    "nvpn-fips-core": "0.4.122",
    "nvpn-fips-endpoint": "0.4.122",
    "nvpn-fips-tcp": "0.2.6",
    "nvpn-fips-tcp-endpoint": "0.2.56",
    "hashtree-config": "0.2.83",
    "hashtree-core": "0.3.0",
    "hashtree-index": "0.3.0",
    "hashtree-lmdb": "0.3.0",
    "hashtree-fips-transport": "0.5.0",
    "hashtree-network": "0.3.0",
    "nostr-pubsub": "0.1.20",
    "nostr-pubsub-fips": "0.6.4",
    "nostr-pubsub-relay": "0.1.16",
    "nostr-pubsub-social-graph": "0.3.1",
    "nostr-social-graph": "0.2.1",
    "nostr-social-memory": "0.2.1",
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
    def test_native_roots_use_the_same_published_ice_fix(self):
        # This published archive contains the consent/candidate fixes formerly
        # carried by the local patch, plus the refreshed WebRTC foundation.
        expected_checksum = "95267a504a7516e22699364101a64f48cf733a41ca4f6a087bd7d39194073ebd"
        for consumer in ("core", "linux"):
            with self.subTest(consumer=consumer):
                manifest = (ROOT / consumer / "Cargo.toml").read_text(encoding="utf-8")
                self.assertNotRegex(manifest, r"(?m)^webrtc-ice\s*=", "Do not reintroduce the inactive old patch")
                lock = ROOT / consumer / "Cargo.lock"
                versions = package_versions(lock)
                self.assertNotIn("webrtc-ice", versions, "Do not ship a second ICE implementation")
                self.assertEqual(versions.get("nvpn-webrtc-ice"), {"0.17.3"})
                self.assertEqual(versions.get("nvpn-webrtc"), {"0.17.4"})
                self.assertEqual(versions.get("nvpn-mdns-sd"), {"0.19.2"})
                entries = [block for block in lock.read_text().split("[[package]]")
                           if re.search(r'(?m)^name = "nvpn-webrtc-ice"$', block)]
                self.assertEqual(len(entries), 1)
                self.assertIn(f'checksum = "{expected_checksum}"', entries[0])
                self.assertIn('source = "registry+https://github.com/rust-lang/crates.io-index"', entries[0])

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
                    self.assertEqual(
                        versions.get(package),
                        {expected} if isinstance(expected, str) else expected,
                        f"{package} in {lock_path}",
                    )
                self.assertIn(
                    "0.6.0",
                    versions.get("nostr-identity", set()),
                    f"direct nostr-identity release in {lock_path}",
                )

    def test_shipping_tls_verifier_and_roots_include_trust_fixes(self):
        for lock_path in PLATFORM_LOCKS:
            with self.subTest(lock=lock_path.relative_to(ROOT)):
                versions = package_versions(lock_path)
                self.assertEqual(versions.get("rustls-webpki"), {"0.103.16"})
                # 0.26.11 reexports the resolved 1.x roots for older consumers.
                self.assertEqual(versions.get("webpki-roots"), {"0.26.11", "1.0.10"})

    def test_core_manifest_pins_gated_fips_stack_exactly(self):
        manifest = (ROOT / "core" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-core = \{ version = "=0\.4\.122",', "fips-core must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-endpoint = \{ version = "=0\.4\.122" \}$', "fips-endpoint must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-tcp = \{ version = "=0\.2\.6" \}$', "fips-tcp must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nvpn-fips-tcp-endpoint = \{ version = "=0\.2\.56" \}$', "fips-tcp-endpoint must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-config = "=0\.2\.83"$', "Hashtree config must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-core = "=0\.3\.0"$', "Hashtree core must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-fips-transport = "=0\.5\.0"$', "Hashtree/FIPS transport must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^hashtree-network = "=0\.3\.0"$', "Hashtree network must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-identity = "=0\.6\.0"$', "nostr-identity must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub = "=0\.1\.20"$', "nostr-pubsub must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub-fips = "=0\.6\.4"$', "nostr-pubsub-fips must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-pubsub-relay = "=0\.1\.16"$', "nostr-pubsub-relay must stay on the gated release")
        self.assertRegex(manifest, r'(?m)^nostr-social-graph = "=0\.2\.1"$', "Direct graph consumer needs revision-aware lists")
        # Direct and pubsub/memory consumers share the reviewed revision-aware graph.
        for lock_path in PLATFORM_LOCKS:
            entries = [block for block in lock_path.read_text().split("[[package]]")
                       if re.search(r'(?m)^name = "nostr-social-graph"$', block)
                       and re.search(r'(?m)^version = "0.2.1"$', block)]
            self.assertEqual(len(entries), 1)
            self.assertIn('checksum = "c791a910d0dfe97088bb9ebac2b0551e21102ae4f272ad2a8b20cec27e188402"', entries[0])

    def test_revision_aware_provider_archives_are_the_accepted_tuple(self):
        checksums = {
            "nostr-pubsub-fips": "d37fc71dd2abca755b85ef3157813d4d833bf06765541e44af4d195d01370606",
            "nostr-pubsub-social-graph": "6f7f5d8b5ae252baf6bba6f1fafcd4d0df80a2a9b31eaccc3786016e35f25abd",
            "nostr-social-memory": "4b28fbf489df058c877fba6d660d041883683d40272b5dbb8cf56679f1c105a1",
        }
        for lock_path in PLATFORM_LOCKS:
            for name, checksum in checksums.items():
                with self.subTest(lock=lock_path.name, package=name):
                    entries = [block for block in lock_path.read_text().split("[[package]]")
                               if re.search(rf'(?m)^name = "{re.escape(name)}"$', block)]
                    self.assertEqual(len(entries), 1)
                    self.assertIn(f'checksum = "{checksum}"', entries[0])
                    self.assertIn('source = "registry+https://github.com/rust-lang/crates.io-index"', entries[0])

    def test_tracing_is_optional_and_only_exposed_by_stack_fixture(self):
        manifest = (ROOT / "core" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertRegex(manifest, r'(?m)^tracing = \{ version = "=0\.1\.44", optional = true \}$')
        features = re.search(r'(?ms)^\[features\]\n(.*?)(?=^\[|\Z)', manifest).group(1)
        self.assertRegex(features, r'(?m)^stack-fixture = \["dep:tracing"\]$')
        self.assertEqual(features.count('dep:tracing'), 1)
        self.assertNotRegex(features, r'(?m)^default\s*=')

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
