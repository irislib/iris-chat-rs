#!/usr/bin/env python3
"""Hosted exact-source Cargo package gate. Never publishes or reads credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import time
import tomllib
import urllib.request

from verify_inbound_retry import TOOLCHAIN, bounded, git, save, sha

INPUTS = Path(__file__).with_name("ratchet_package_verification_input.json")
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
NDR = "nostr-double-ratchet"
PAIRWISE = f"{NDR}-pairwise"
CODEC = f"{PAIRWISE}-codec"
GENERATED = {"Cargo.toml", "Cargo.toml.orig", "Cargo.lock", ".cargo_vcs_info.json"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def info(path):
    return {"file": path.name, "bytes": path.stat().st_size, "sha256": sha(path)}


def contract(path=INPUTS):
    data = json.loads(path.read_text())
    assert data["schema"] == "ndr-source-package-input/v1"
    assert data["enabled"] is True, "Package gate disabled pending source and workflow review"
    assert data["repository"] == "irislib/nostr-double-ratchet"
    assert re.fullmatch(r"[0-9a-f]{40}", data["source"])
    assert data["toolchain"] == TOOLCHAIN == "1.99.1"
    assert data["packages"] == {NDR: "0.0.174", PAIRWISE: "0.0.172"}
    assert data["codec"]["package"] == CODEC and data["codec"]["version"] == "0.0.168"
    assert re.fullmatch(r"[0-9a-f]{64}", data["codec"]["sha256"])
    assert re.fullmatch(r"[0-9a-f]{40}", data["codec"]["vcs_source"])
    assert data["limits"] == {"jobs": 2, "maximum_memory_bytes": 5 * 1024**3,
        "total_seconds": 1500, "workflow_seconds": 1800,
        "registry_response_bytes": 16 * 1024**2, "archive_uncompressed_bytes": 64 * 1024**2}
    q = data["qualification"]
    assert all(q[key] is True for key in ("full_ci_accepted", "allocation_accepted", "kdf_reuse_accepted", "source_and_workflow_reviewed"))
    assert all(type(q[key]) is int and q[key] > 0 for key in ("full_ci_run", "allocation_run", "kdf_reuse_run"))
    assert sha(Path(__file__).with_name("verify_inbound_retry.py")) == data["resource_helper_sha256"]
    return data


def source_bytes(root, source, path):
    return subprocess.check_output(["git", "-C", str(root), "show", f"{source}:{path}"], timeout=30)


def source_proof(root, data):
    assert git(root, "rev-parse", "HEAD") == data["source"]
    assert not git(root, "status", "--porcelain"), "Source checkout must be clean"
    return {"commit": data["source"], "tree": git(root, "rev-parse", "HEAD^{tree}"),
        "rust_tree": git(root, "rev-parse", "HEAD:rust"), "lock_sha256": sha(root / "rust/Cargo.lock")}


def download(url, path, limit, deadline):
    remaining = deadline - time.time()
    assert remaining > 0, "Package deadline reached before registry readback"
    request = urllib.request.Request(url, headers={"User-Agent": "iris-ratchet-package-verification"})
    with urllib.request.urlopen(request, timeout=min(60, remaining)) as response:
        data = response.read(limit + 1)
    assert len(data) <= limit, "Registry response exceeds evidence limit"
    assert time.time() < deadline, "Package deadline reached during registry readback"
    path.write_bytes(data)
    return data


def archive_contents(path, package, version, limit):
    prefix, contents, total = f"{package}-{version}", {}, 0
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            name = PurePosixPath(member.name)
            assert not name.is_absolute() and ".." not in name.parts
            assert name.parts and name.parts[0] == prefix and len(name.parts) > 1
            if member.isdir():
                continue
            assert member.isfile(), "Links and special archive members are forbidden"
            relative = str(PurePosixPath(*name.parts[1:]))
            assert relative not in contents, "Duplicate archive member"
            total += member.size
            assert total <= limit and len(contents) < 10000, "Archive exceeds inventory limits"
            contents[relative] = archive.extractfile(member).read()
    assert GENERATED <= contents.keys(), "Cargo provenance files are missing"
    return contents


def expanded_dependencies(original, workspace, section):
    result = {}
    for name, entry in original.get(section, {}).items():
        item = {"version": entry} if isinstance(entry, str) else dict(entry)
        if item.pop("workspace", False):
            inherited = workspace["workspace"]["dependencies"][name]
            inherited = {"version": inherited} if isinstance(inherited, str) else dict(inherited)
            inherited.update(item)
            item = inherited
        if section == "dev-dependencies" and "path" in item and "version" not in item:
            continue  # Cargo omits unpublished path-only test helpers.
        item.pop("path", None)
        assert "version" in item and not ({"git", "registry", "registry-index"} & item.keys())
        result[name] = item
    return result


def inspect_manifest(contents, package, version, subdir, read):
    original_bytes = read(f"{subdir}/Cargo.toml")
    assert contents["Cargo.toml.orig"] == original_bytes, "Original manifest differs from Git"
    original = tomllib.loads(original_bytes.decode())
    workspace = tomllib.loads(read("rust/Cargo.toml").decode())
    normalized = tomllib.loads(contents["Cargo.toml"].decode())
    assert normalized["package"]["name"] == package and normalized["package"]["version"] == version
    expected_package = {}
    for key, value in original["package"].items():
        expected_package[key] = workspace["workspace"]["package"][key] if value == {"workspace": True} else value
    expected_package.update({key: False for key in ("build", "autolib", "autobins", "autoexamples", "autotests", "autobenches")})
    expected_package["readme"] = "README.md" if "README.md" in contents else False
    assert normalized["package"] == expected_package, "Normalized package metadata changed"
    assert normalized.keys() <= {"package", "features", "lib", "test", "dependencies", "dev-dependencies", "build-dependencies"}
    assert normalized.get("features", {}) == original.get("features", {})
    assert normalized["lib"] == {"name": package.replace("-", "_"), "path": "src/lib.rs"}
    expected_tests = [{"name": PurePosixPath(p).stem, "path": p} for p in contents
        if p.startswith("tests/") and p.endswith(".rs") and len(PurePosixPath(p).parts) == 2]
    assert sorted(normalized.get("test", []), key=lambda row: row["name"]) == sorted(expected_tests, key=lambda row: row["name"])
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        assert normalized.get(section, {}) == expanded_dependencies(original, workspace, section), f"Dependency manifest changed: {section}"
    return normalized


def lock_nodes(lock):
    result = {}
    for item in lock["package"]:
        key = (item["name"], item["version"])
        assert key not in result, "Ambiguous lock package identity"
        result[key] = item
    return result


def dependency_edges(item, nodes):
    edges = set()
    for dependency in item.get("dependencies", []):
        parts = dependency.split()
        assert 1 <= len(parts) <= 3
        matches = [key for key in nodes if key[0] == parts[0] and (len(parts) == 1 or key[1] == parts[1])]
        assert len(matches) == 1, f"Unresolved or ambiguous lock edge: {dependency}"
        if len(parts) == 3:
            assert parts[2] == f"({nodes[matches[0]].get('source')})"
        edges.add(matches[0])
    return edges


def inspect_lock(contents, package, version, read, registry_dependencies):
    packaged = lock_nodes(tomllib.loads(contents["Cargo.lock"].decode()))
    source = lock_nodes(tomllib.loads(read("rust/Cargo.lock").decode()))
    root = (package, version)
    assert root in packaged and root in source
    for key, item in packaged.items():
        assert key in source, f"Dependency version not present in source lock: {key}"
        expected = source[key]
        if key == root:
            assert "source" not in item and "checksum" not in item
        elif key[0] in registry_dependencies:
            accepted = registry_dependencies[key[0]]
            assert key[1] == accepted["version"] and "source" not in expected
            assert item["source"] == REGISTRY and item["checksum"] == accepted["sha256"]
        else:
            assert item.get("source") == expected.get("source") == REGISTRY
            assert item.get("checksum") == expected.get("checksum")
            assert re.fullmatch(r"[0-9a-f]{64}", item["checksum"])
        edges = dependency_edges(item, packaged)
        expected_edges = dependency_edges(expected, source)
        if key == root:
            if package == NDR:
                expected_edges.discard((CODEC, "0.0.168"))
            assert edges == expected_edges, "Root dependency graph changed"
        else:
            assert edges <= expected_edges, "Packaged lock introduced a dependency edge"
    reached, pending = set(), [root]
    while pending:
        key = pending.pop()
        if key not in reached:
            reached.add(key)
            pending.extend(dependency_edges(packaged[key], packaged))
    assert reached == packaged.keys(), "Archive lock includes unrelated packages"
    for name, accepted in registry_dependencies.items():
        assert (name, accepted["version"]) in packaged, "Expected registry dependency missing"
    return {"sha256": digest(contents["Cargo.lock"]), "packages": len(packaged),
        "source_versions_and_checksums_preserved": True,
        "projection": "Workspace-only packages, path-only dev helpers and unused feature edges may be omitted; registry versions/checksums cannot change.",
        "path_to_registry": registry_dependencies}


def inspect_archive(path, package, version, root, data, registry_dependencies, vcs_source=None):
    contents = archive_contents(path, package, version, data["limits"]["archive_uncompressed_bytes"])
    subdir = f"rust/crates/{package}"
    read = lambda p: source_bytes(root, data["source"], p)
    vcs = json.loads(contents[".cargo_vcs_info.json"])
    assert vcs["git"]["sha1"] == (vcs_source or data["source"]) and not vcs["git"].get("dirty", False)
    assert vcs["path_in_vcs"] == subdir
    manifest = inspect_manifest(contents, package, version, subdir, read)
    inventory = []
    for relative, payload in contents.items():
        if relative not in GENERATED:
            assert payload == read(f"{subdir}/{relative}"), f"Source bytes differ: {relative}"
        inventory.append({"file": relative, "bytes": len(payload), "sha256": digest(payload)})
    tracked = git(root, "ls-tree", "-r", "--name-only", data["source"], subdir).splitlines()
    required = {p[len(subdir) + 1:] for p in tracked} - {"Cargo.toml", "Cargo.lock"}
    assert required == contents.keys() - GENERATED, "Archive source inventory differs from the qualified crate tree"
    # An already-published codec's own historical lock is not used when it is
    # resolved as a dependency. Its exact archive checksum is still admitted;
    # the newly packaged pairwise lock must match this source and both checksums.
    lock = ({"sha256": digest(contents["Cargo.lock"]), "dependency_only": True}
        if package == CODEC else inspect_lock(contents, package, version, read, registry_dependencies))
    inventory.sort(key=lambda row: row["file"])
    return {"archive": info(path), "package": {"name": package, "version": version},
        "vcs": vcs, "manifest_sha256": digest(contents["Cargo.toml"]),
        "original_manifest_sha256": digest(contents["Cargo.toml.orig"]),
        "normalized_dependencies": {section: manifest.get(section, {}) for section in ("dependencies", "dev-dependencies", "build-dependencies")},
        "default_features": manifest.get("features", {}).get("default", []),
        "lock": lock,
        "inventory": inventory, "source_mapping": inventory, "all_source_bytes_match_git": True}


def registry_record(raw, package, version, checksum):
    rows = [json.loads(line) for line in raw.splitlines()]
    matches = [row for row in rows if row["vers"] == version]
    assert len(matches) == 1, "Registry version missing or duplicated"
    item = matches[0]
    assert item["name"] == package and item["cksum"] == checksum and item["yanked"] is False
    return item


def registry_readback(package, version, checksum, root, out, data, deadline, vcs_source=None):
    assert re.fullmatch(r"[0-9a-f]{64}", checksum), "Root-admitted archive checksum required"
    index_path = out / f"{package}-index.jsonl"
    raw = download(f"https://index.crates.io/{package[:2]}/{package[2:4]}/{package}", index_path,
        data["limits"]["registry_response_bytes"], deadline)
    record = registry_record(raw, package, version, checksum)
    archive = out / f"{package}-{version}.crate"
    download(f"https://static.crates.io/crates/{package}/{package}-{version}.crate", archive,
        data["limits"]["registry_response_bytes"], deadline)
    assert sha(archive) == checksum, "Public archive differs from accepted checksum"
    inspection = inspect_archive(archive, package, version, root, data, {}, vcs_source)
    return {"version": version, "sha256": checksum, "index": info(index_path), "record": record, "inspection": inspection}


def package_command(package):
    assert package in {NDR, PAIRWISE}
    return ["cargo", f"+{TOOLCHAIN}", "package", "--manifest-path", f"rust/crates/{package}/Cargo.toml",
        "--registry", "crates-io", "--locked", "--jobs", "2", "--color", "never"]


def preflight(out, package, checksum, data):
    assert os.environ.get("GITHUB_ACTIONS") == "true", "Only the reviewed hosted job may execute this gate"
    root = Path(__file__).resolve().parent.parent
    assert not git(root, "status", "--porcelain"), "Driver must be a clean committed checkout"
    assert not (out / "preflight.json").exists(), "Use a fresh evidence directory"
    assert (bool(re.fullmatch(r"[0-9a-f]{64}", checksum)) if package == PAIRWISE else checksum == "")
    save(out / "preflight.json", {"schema": "ndr-source-package-preflight/v1", "inputs": data,
        "inputs_sha256": sha(INPUTS), "driver_source": git(root, "rev-parse", "HEAD"),
        "package": package, "ndr_sha256": checksum, "deadline_unix": time.time() + 1500})
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        stream.write(f"repository={data['repository']}\nsource={data['source']}\npackage={package}\n")


def verify(root, out, data):
    assert os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_OS") == "Linux"
    pre = json.loads((out / "preflight.json").read_text())
    assert pre["schema"] == "ndr-source-package-preflight/v1"
    assert pre["inputs"] == data and pre["inputs_sha256"] == sha(INPUTS)
    assert time.time() < pre["deadline_unix"]
    package, deadline = pre["package"], pre["deadline_unix"]
    version = data["packages"][package]
    source = source_proof(root, data)
    save(out / "source.json", source)
    versions = {}
    for tool in ("rustc", "cargo"):
        value = subprocess.check_output([tool, f"+{TOOLCHAIN}", "--version", "--verbose"], timeout=60).decode()
        assert re.search(rf"^{tool} {re.escape(TOOLCHAIN)}(?:\s|$)", value)
        versions[tool] = value
    save(out / "toolchain.json", versions)
    dependencies, checksums = {}, {}
    if package == PAIRWISE:
        for name, accepted_version, checksum, vcs in [
            (NDR, data["packages"][NDR], pre["ndr_sha256"], None),
            (CODEC, data["codec"]["version"], data["codec"]["sha256"], data["codec"]["vcs_source"]),
        ]:
            dependencies[name] = registry_readback(name, accepted_version, checksum, root, out, data, deadline, vcs)
            checksums[name] = {"version": accepted_version, "sha256": checksum}
        save(out / "published-dependencies.json", dependencies)
    target = Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    assert not target.is_relative_to(root) and not out.is_relative_to(root), "Keep generated outputs outside source"
    target.mkdir(parents=True, exist_ok=True)
    candidate = target / "package" / f"{package}-{version}.crate"
    if candidate.exists():
        # Only this selected archive in the job-exclusive restored build cache.
        shutil.copyfile(candidate, out / f"prior-cached-{candidate.name}")
        candidate.unlink()
    run, text = bounded(package_command(package), root, out, "cargo-package", 1500, deadline, target)
    assert run["exit_code"] == 0 and run["owned_process_reaped"] is True, "Actual cargo package verification failed"
    assert f"Verifying {package} v{version}" in text, "Cargo did not verify the extracted crate"
    assert candidate.is_file(), "Cargo did not produce the selected archive"
    retained = out / candidate.name
    shutil.copyfile(candidate, retained)
    inspection = inspect_archive(retained, package, version, root, data, checksums)
    assert source_proof(root, data) == source
    save(out / "acceptance.json", {"schema": "ndr-source-package-verification/v1", "accepted": True,
        "source": source, "preflight": info(out / "preflight.json"), "toolchain": versions,
        "package_run": run, "archive": inspection, "published_dependencies": dependencies,
        "cache": {key: os.environ.get(key, "") for key in ("NDR_CACHE_REQUESTED", "NDR_CACHE_MATCHED", "NDR_CACHE_HIT")},
        "source_clean_after": True, "published": False,
        "interpretation": "Real cargo package built the extracted source archive with default features. No runtime-test or app release-binary acceptance."})
    (out / "SHA256SUMS").write_text(f"{sha(retained)}  {retained.name}\n")
    print(json.dumps({"accepted": True, "package": package, "version": version, "archive": info(retained)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "verify"))
    parser.add_argument("--package", choices=(NDR, PAIRWISE))
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ndr-sha256", default="")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    try:
        inputs = contract()
        if args.stage == "preflight":
            assert args.package is not None
            preflight(output, args.package, args.ndr_sha256, inputs)
        else:
            assert args.source is not None
            verify(args.source.resolve(), output, inputs)
    except BaseException as error:
        save(output / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
