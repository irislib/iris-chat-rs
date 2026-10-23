#!/usr/bin/env python3
"""Offline contract tests: no Cargo, registry requests, or subprocess execution."""
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify_ratchet_packages as gate


SOURCE = "a" * 40
CHECKSUM = "b" * 64


def package_lock(package, version, dependencies="", extra=""):
    return f'version = 4\n[[package]]\nname = "{package}"\nversion = "{version}"\n{dependencies}\n{extra}'.encode()


def fixture():
    name, version = gate.NDR, "0.0.172"
    original = f'[package]\nname = "{name}"\nversion = "{version}"\nedition.workspace = true\n[features]\ndefault = ["full"]\nfull = ["pairwise"]\npairwise = []\n'
    normalized = original.replace('edition.workspace = true', 'edition = "2021"')
    normalized = normalized.replace('[features]', 'build = false\nautolib = false\nautobins = false\nautoexamples = false\nautotests = false\nautobenches = false\nreadme = "README.md"\n[features]')
    normalized += '[lib]\nname = "nostr_double_ratchet"\npath = "src/lib.rs"\n'
    normalized += '[[test]]\nname = "regression"\npath = "tests/regression.rs"\n'
    lock = package_lock(name, version)
    contents = {"Cargo.toml": normalized.encode(), "Cargo.toml.orig": original.encode(),
        "Cargo.lock": lock, ".cargo_vcs_info.json": json.dumps({"git": {"sha1": SOURCE}, "path_in_vcs": f"rust/crates/{name}"}).encode(),
        "src/lib.rs": b"pub fn example() {}\n", "tests/regression.rs": b"#[test] fn regression() {}\n", "README.md": b"Example\n"}
    source = {f"rust/crates/{name}/{p}": b for p, b in contents.items() if p not in gate.GENERATED}
    source[f"rust/crates/{name}/Cargo.toml"] = original.encode()
    source["rust/Cargo.toml"] = b'[workspace.package]\nedition = "2021"\n[workspace.dependencies]\n'
    source["rust/Cargo.lock"] = lock
    data = {"source": SOURCE, "limits": {"archive_uncompressed_bytes": 1024**2}}
    return contents, source, data


def write_archive(path, contents, package=gate.NDR, version="0.0.172", extra=None):
    with tarfile.open(path, "w:gz") as archive:
        for relative, content in contents.items():
            item = tarfile.TarInfo(f"{package}-{version}/{relative}")
            item.size = len(content)
            archive.addfile(item, io.BytesIO(content))
        if extra is not None:
            archive.addfile(extra, io.BytesIO(b"x") if extra.isfile() else None)


class ContractTests(unittest.TestCase):
    def test_disabled_admission_rejected(self):
        data = json.loads(gate.INPUTS.read_text())
        data["enabled"] = False
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text(json.dumps(data))
            with self.assertRaisesRegex(AssertionError, "disabled"):
                gate.contract(path)

    def test_reviewed_contract_binds_exact_resource_helper(self):
        data = json.loads(gate.INPUTS.read_text())
        data["enabled"] = True
        data["qualification"].update(full_ci_accepted=True, allocation_accepted=True, source_and_workflow_reviewed=True)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text(json.dumps(data))
            self.assertEqual(gate.contract(path), data)
            data["resource_helper_sha256"] = "f" * 64
            path.write_text(json.dumps(data))
            with self.assertRaises(AssertionError):
                gate.contract(path)

    def test_source_and_workflow_review_required(self):
        data = json.loads(gate.INPUTS.read_text())
        data["enabled"] = True
        data["qualification"].update(full_ci_accepted=True, allocation_accepted=True, source_and_workflow_reviewed=False)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text(json.dumps(data))
            with self.assertRaises(AssertionError):
                gate.contract(path)

    def test_pending_ci_or_allocation_acceptance_is_rejected(self):
        for field in ("full_ci_accepted", "allocation_accepted"):
            data = json.loads(gate.INPUTS.read_text())
            data["enabled"] = True
            data["qualification"].update(full_ci_accepted=True, allocation_accepted=True, source_and_workflow_reviewed=True)
            data["qualification"][field] = False
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "input.json"
                path.write_text(json.dumps(data))
                with self.assertRaises(AssertionError):
                    gate.contract(path)

    def test_real_package_verification_command(self):
        for name in (gate.NDR, gate.PAIRWISE):
            command = gate.package_command(name)
            self.assertEqual(command[:3], ["cargo", "+1.99.1", "package"])
            self.assertIn("--locked", command)
            self.assertEqual(command[command.index("--jobs") + 1], "2")
            self.assertNotIn("--no-verify", command)
            self.assertNotIn("--allow-dirty", command)
            self.assertNotIn("publish", command)

    def test_registry_record_exact_checksum_version_and_not_yanked(self):
        row = {"name": gate.NDR, "vers": "0.0.172", "cksum": CHECKSUM, "yanked": False}
        self.assertEqual(gate.registry_record(json.dumps(row).encode(), gate.NDR, "0.0.172", CHECKSUM), row)
        for key, value in [("name", "other"), ("vers", "0.0.171"), ("cksum", "c" * 64), ("yanked", True)]:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                gate.registry_record(json.dumps(dict(row, **{key: value})).encode(), gate.NDR, "0.0.172", CHECKSUM)
        with self.assertRaises(AssertionError):
            gate.registry_record((json.dumps(row) + "\n" + json.dumps(row)).encode(), gate.NDR, "0.0.172", CHECKSUM)

    def test_workspace_dependencies_expand_and_only_unversioned_dev_path_is_omitted(self):
        workspace = {"workspace": {"dependencies": {gate.NDR: {"version": "0.0.172", "path": "crates/ndr", "default-features": False}}}}
        original = {"dependencies": {gate.NDR: {"workspace": True, "features": ["pairwise"]}},
            "dev-dependencies": {"helper": {"path": "../helper"}}}
        self.assertEqual(gate.expanded_dependencies(original, workspace, "dependencies"),
            {gate.NDR: {"version": "0.0.172", "default-features": False, "features": ["pairwise"]}})
        self.assertEqual(gate.expanded_dependencies(original, workspace, "dev-dependencies"), {})

    def test_original_and_normalized_manifest_pins(self):
        contents, source, _ = fixture()
        gate.inspect_manifest(contents, gate.NDR, "0.0.172", f"rust/crates/{gate.NDR}", source.__getitem__)
        for field in ("Cargo.toml", "Cargo.toml.orig"):
            changed = dict(contents)
            changed[field] = changed[field].replace(b'0.0.172', b'0.0.173')
            with self.subTest(field=field), self.assertRaises(AssertionError):
                gate.inspect_manifest(changed, gate.NDR, "0.0.172", f"rust/crates/{gate.NDR}", source.__getitem__)

    def test_lock_path_dependency_requires_admitted_registry_checksum(self):
        dependency = f'[[package]]\nname = "{gate.NDR}"\nversion = "0.0.172"\n'
        source_lock = package_lock(gate.PAIRWISE, "0.0.170", f'dependencies = ["{gate.NDR}"]', dependency)
        packaged = source_lock + f'source = "{gate.REGISTRY}"\nchecksum = "{CHECKSUM}"\n'.encode()
        expected = {gate.NDR: {"version": "0.0.172", "sha256": CHECKSUM}}
        result = gate.inspect_lock({"Cargo.lock": packaged}, gate.PAIRWISE, "0.0.170", lambda _: source_lock, expected)
        self.assertEqual(result["path_to_registry"], expected)
        for bad in ({}, {gate.NDR: {"version": "0.0.172", "sha256": "c" * 64}}):
            with self.subTest(pins=bad), self.assertRaises(AssertionError):
                gate.inspect_lock({"Cargo.lock": packaged}, gate.PAIRWISE, "0.0.170", lambda _: source_lock, bad)

    def test_normalized_manifest_rejects_unreviewed_build_or_dependency_override(self):
        contents, source, _ = fixture()
        for payload in (contents["Cargo.toml"].replace(b'build = false', b'build = "src/lib.rs"'),
            contents["Cargo.toml"] + b'\n[patch.crates-io]\nserde = { path = "other" }\n'):
            changed = dict(contents, **{"Cargo.toml": payload})
            with self.assertRaises(AssertionError):
                gate.inspect_manifest(changed, gate.NDR, "0.0.172", f"rust/crates/{gate.NDR}", source.__getitem__)

    def test_lock_rejects_new_dependency_and_checksum_drift(self):
        dependency = f'[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "{gate.REGISTRY}"\nchecksum = "{CHECKSUM}"\n'
        source = package_lock(gate.NDR, "0.0.172", 'dependencies = ["serde"]', dependency)
        gate.inspect_lock({"Cargo.lock": source}, gate.NDR, "0.0.172", lambda _: source, {})
        for packaged in (source.replace(CHECKSUM.encode(), b"c" * 64), source.replace(b'1.0.0', b'1.0.1')):
            with self.assertRaises(AssertionError):
                gate.inspect_lock({"Cargo.lock": packaged}, gate.NDR, "0.0.172", lambda _: source, {})

    def test_ndr_lock_omits_only_its_path_only_dev_dependency(self):
        source = package_lock(gate.NDR, "0.0.172", f'dependencies = ["{gate.CODEC}"]',
            f'[[package]]\nname = "{gate.CODEC}"\nversion = "0.0.168"\n')
        packaged = package_lock(gate.NDR, "0.0.172")
        gate.inspect_lock({"Cargo.lock": packaged}, gate.NDR, "0.0.172", lambda _: source, {})
        with self.assertRaises(AssertionError):
            gate.inspect_lock({"Cargo.lock": source}, gate.NDR, "0.0.172", lambda _: source, {})

    def test_archive_rejects_unsafe_or_duplicate_members_and_size_excess(self):
        contents, _, _ = fixture()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.crate"
            for name, kind in [("../escape", tarfile.REGTYPE), ("src/lib.rs", tarfile.REGTYPE), ("link", tarfile.SYMTYPE)]:
                extra = tarfile.TarInfo(f"{gate.NDR}-0.0.172/{name}")
                extra.type = kind
                extra.size = 1 if kind == tarfile.REGTYPE else 0
                extra.linkname = "src/lib.rs"
                write_archive(path, contents, extra=extra)
                with self.subTest(member=name), self.assertRaises(AssertionError):
                    gate.archive_contents(path, gate.NDR, "0.0.172", 1024**2)
            write_archive(path, contents)
            with self.assertRaises(AssertionError):
                gate.archive_contents(path, gate.NDR, "0.0.172", 8)

    def test_archive_requires_complete_source_inventory_and_exact_bytes(self):
        contents, source, data = fixture()
        tracked = "\n".join(p for p in source if p.startswith(f"rust/crates/{gate.NDR}/"))
        with tempfile.TemporaryDirectory() as directory, patch.object(gate, "source_bytes", side_effect=lambda root, commit, path: source[path]), patch.object(gate, "git", return_value=tracked):
            path = Path(directory) / "test.crate"
            write_archive(path, contents)
            result = gate.inspect_archive(path, gate.NDR, "0.0.172", Path(directory), data, {})
            self.assertEqual(result["inventory"], result["source_mapping"])
            self.assertEqual(len(result["inventory"]), len(contents))
            self.assertTrue(result["all_source_bytes_match_git"])
            for removed in ("src/lib.rs", "tests/regression.rs", "README.md"):
                changed = dict(contents)
                del changed[removed]
                write_archive(path, changed)
                with self.subTest(removed=removed), self.assertRaises(AssertionError):
                    gate.inspect_archive(path, gate.NDR, "0.0.172", Path(directory), data, {})
            changed = dict(contents, **{"src/lib.rs": b"unqualified source"})
            write_archive(path, changed)
            with self.assertRaises(AssertionError):
                gate.inspect_archive(path, gate.NDR, "0.0.172", Path(directory), data, {})

    def test_archive_requires_exact_clean_vcs_source(self):
        contents, source, data = fixture()
        with tempfile.TemporaryDirectory() as directory, patch.object(gate, "source_bytes", side_effect=lambda root, commit, path: source[path]):
            path = Path(directory) / "test.crate"
            for vcs in ({"git": {"sha1": "d" * 40}, "path_in_vcs": f"rust/crates/{gate.NDR}"},
                {"git": {"sha1": SOURCE, "dirty": True}, "path_in_vcs": f"rust/crates/{gate.NDR}"}):
                changed = dict(contents, **{".cargo_vcs_info.json": json.dumps(vcs).encode()})
                write_archive(path, changed)
                with self.assertRaises(AssertionError):
                    gate.inspect_archive(path, gate.NDR, "0.0.172", Path(directory), data, {})


if __name__ == "__main__":
    unittest.main()
