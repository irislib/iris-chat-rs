#!/usr/bin/env python3
"""Bounded source-package qualification; never publishes or reads credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import subprocess
import tarfile
import time
import tomllib
import urllib.request

SOURCE = "cca9525d209ec51038c588ef7e55460ffa1614aa"
RUST_TOOLCHAIN = "1.99.1"
CORE_RUN = 39939972967
PROTOCOL = "0.3.5"
CLI = "0.1.47"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
GIB = 1024**3


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_info(path):
    data = path.read_bytes()
    return {"file": path.name, "bytes": len(data), "sha256": digest(data)}


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], timeout=30)


def clean_source(root):
    assert git(root, "rev-parse", "HEAD").decode().strip() == SOURCE
    assert not git(root, "status", "--porcelain", "--untracked-files=normal"), "Source checkout is dirty"


def read_source(root, relative):
    return git(root, "show", f"{SOURCE}:{relative}")


def download(url, destination, limit=8 * 1024**2):
    request = urllib.request.Request(url, headers={"User-Agent": "iris-chat-source-package-verification"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read(limit + 1)
    assert len(data) <= limit, "Registry response exceeds evidence size limit"
    destination.write_bytes(data)
    return data


def inspect_archive(archive, package, version, subdir, source_reader, protocol_checksum=None):
    """Bind every ordinary archive file to Git; allow only Cargo's generated files."""
    prefix = f"{package}-{version}"
    contents = {}
    with tarfile.open(archive, "r:gz") as tar:
        for member in tar:
            path = PurePosixPath(member.name)
            assert not path.is_absolute() and ".." not in path.parts
            assert path.parts[0] == prefix and len(path.parts) > 1
            if member.isdir():
                continue
            assert member.isfile(), "Links and special archive members are forbidden"
            relative = str(PurePosixPath(*path.parts[1:]))
            assert relative not in contents, "Duplicate archive member"
            contents[relative] = tar.extractfile(member).read()
    vcs = json.loads(contents[".cargo_vcs_info.json"])
    assert vcs["git"]["sha1"] == SOURCE and not vcs["git"].get("dirty", False)
    assert vcs["path_in_vcs"] == subdir
    manifest = tomllib.loads(contents["Cargo.toml"].decode())
    assert manifest["package"]["name"] == package and manifest["package"]["version"] == version
    assert contents["Cargo.toml.orig"] == source_reader(f"{subdir}/Cargo.toml")
    inventory = []
    for relative, data in contents.items():
        if relative not in {"Cargo.toml", "Cargo.toml.orig", "Cargo.lock", ".cargo_vcs_info.json"}:
            assert data == source_reader(f"{subdir}/{relative}"), f"Source bytes differ: {relative}"
        inventory.append({"file": relative, "bytes": len(data), "sha256": digest(data)})
    packaged_lock = tomllib.loads(contents["Cargo.lock"].decode())
    source_lock = tomllib.loads(source_reader(f"{subdir}/Cargo.lock").decode())
    expected = source_lock["package"]
    if package == "iris-chat":
        dependency = manifest["dependencies"]["iris-chat-protocol"]
        assert dependency["version"] == f"={PROTOCOL}" and "path" not in dependency
        assert any(b["name"] == "iris" and b["path"] == "src/bin/iris.rs" for b in manifest["bin"])
        assert manifest["package"]["default-run"] == "iris"
        for item in expected:
            if item["name"] == "iris-chat-protocol":
                assert item["version"] == PROTOCOL and "source" not in item
                item.update(source=REGISTRY, checksum=protocol_checksum)
        for required in ["README.md", "build.rs", "src/bin/iris.rs", "assets/emoji-shortcodes.tsv", "assets/socialGraph.bin"]:
            assert required in contents, f"Required package file missing: {required}"
    canonical = lambda items: sorted(json.dumps(p, sort_keys=True) for p in items)
    assert canonical(packaged_lock["package"]) == canonical(expected), "Unexpected package lock dependency change"
    return {"archive": file_info(archive), "vcs": vcs, "normalized_manifest_sha256": digest(contents["Cargo.toml"]),
            "lock_sha256": digest(contents["Cargo.lock"]), "dependency_count": len(expected),
            "source_mapping": sorted(inventory, key=lambda entry: entry["file"]),
            "all_source_bytes_match_git": True, "lock_changes": "Only the exact protocol path dependency becomes its checksum-pinned registry package." if package == "iris-chat" else "None"}


def preflight(args):
    root, out = args.source.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    clean_source(root)
    assert args.toolchain == RUST_TOOLCHAIN, "Use the corrected core run's exact Rust release"
    assert re.fullmatch(r"[0-9a-f]{64}", args.protocol_sha256), "An accepted protocol archive checksum is required"
    toolchains = {}
    for tool in ["rustc", "cargo"]:
        text = subprocess.check_output([tool, f"+{args.toolchain}", "--version", "--verbose"], timeout=60).decode()
        assert re.search(rf"^{tool} \d+\.\d+\.\d+", text)
        if tool == "rustc":
            assert re.search(rf"^rustc {re.escape(args.toolchain)}(?:\s|$)", text)
        (out / f"{tool}-version.txt").write_text(text)
        toolchains[tool] = text
    index = download("https://index.crates.io/ir/is/iris-chat-protocol", out / "protocol-index.jsonl")
    records = [json.loads(line) for line in index.splitlines()]
    item = next(p for p in records if p["vers"] == PROTOCOL)
    assert item["cksum"] == args.protocol_sha256 and not item["yanked"]
    archive = out / f"iris-chat-protocol-{PROTOCOL}.crate"
    download(f"https://static.crates.io/crates/iris-chat-protocol/iris-chat-protocol-{PROTOCOL}.crate", archive)
    assert file_info(archive)["sha256"] == args.protocol_sha256
    protocol = inspect_archive(archive, "iris-chat-protocol", PROTOCOL, "chat-protocol", lambda p: read_source(root, p))
    manifest = tomllib.loads(read_source(root, "core/Cargo.toml").decode())
    assert manifest["package"]["version"] == CLI and manifest["dependencies"]["iris-chat-protocol"]["version"] == f"={PROTOCOL}"
    evidence = {"schema": "iris-source-package-preflight/v1", "source": SOURCE,
                "source_tree": git(root, "rev-parse", "HEAD^{tree}").decode().strip(),
                "core_tree": git(root, "rev-parse", "HEAD:core").decode().strip(),
                "protocol_tree": git(root, "rev-parse", "HEAD:chat-protocol").decode().strip(),
                "core_qualification_run": CORE_RUN, "toolchain": args.toolchain, "toolchain_readback": toolchains,
                "protocol_checksum": args.protocol_sha256, "registry_index_record": item,
                "registry_protocol_archive": protocol, "source_clean": True}
    save_json(out / "preflight.json", evidence)
    print(json.dumps({"preflight": "passed", "source": SOURCE, "toolchain": args.toolchain, "protocol_checksum": args.protocol_sha256}))


def memory_total():
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemTotal:"):
            return int(line.split()[1]) * 1024
    raise RuntimeError("Cannot read host memory capacity")


def group_rss(pgid):
    rows = subprocess.check_output(["ps", "-eo", "pgid=,rss="], timeout=10).decode().splitlines()
    return sum(int(parts[1]) * 1024 for row in rows if len(parts := row.split()) == 2 and int(parts[0]) == pgid)


def bounded_package(root, out, target, toolchain, *, timeout_seconds=2100, poll_seconds=2):
    assert 0 < timeout_seconds <= 2100 and 0 < poll_seconds <= 2
    command = ["cargo", f"+{toolchain}", "package", "--manifest-path", "core/Cargo.toml", "--registry", "crates-io", "--locked", "--jobs", "2", "--color", "never"]
    candidate = target / "package" / f"iris-chat-{CLI}.crate"
    started = time.monotonic()
    process, exit_code, caught, reason = None, None, None, None
    peak_rss, minimum_free, limit = 0, None, None
    stage, cleanup_error = "resource preflight", None
    log = out / "cargo-package.log"
    try:
        minimum_free = shutil.disk_usage(target).free
        assert minimum_free >= 5 * GIB, "Less than 5 GiB free after cache restoration"
        limit = min(8 * GIB, memory_total() * 4 // 5)
        if candidate.is_file():
            candidate.unlink()  # Only the restored archive in this isolated hosted job.
        with log.open("wb") as output, (out / "resources.jsonl").open("w") as samples:
            stage = "package launch"
            process = subprocess.Popen(command, cwd=root, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            stage, next_report = "resource monitor", 0
            while process.poll() is None:
                elapsed = time.monotonic() - started
                rss, free = group_rss(process.pid), shutil.disk_usage(target).free
                peak_rss, minimum_free = max(peak_rss, rss), min(minimum_free, free)
                if elapsed >= timeout_seconds: reason = "Package time limit reached"
                elif rss > limit: reason = "Package process group exceeded memory budget"
                elif free < 2 * GIB: reason = "Free disk fell below 2 GiB"
                if elapsed >= next_report or reason:
                    sample = {"elapsed_seconds": round(elapsed, 1), "rss_bytes": rss, "free_disk_bytes": free, "stop_reason": reason}
                    samples.write(json.dumps(sample) + "\n"); samples.flush()
                    print(json.dumps(sample), flush=True); next_report = elapsed + 30
                if reason:
                    break
                time.sleep(poll_seconds)
    except BaseException as error:
        caught = error
        reason = f"{stage} failed: {type(error).__name__}: {error}"
    finally:
        if process is not None:
            try:
                if process.poll() is None:
                    # Popen created this new session; never signal the caller's group.
                    try: os.killpg(process.pid, signal.SIGTERM)
                    except ProcessLookupError: pass
                    try: process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        try: os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError: pass
                        process.wait(timeout=10)
                exit_code = process.wait(timeout=10)
            except BaseException as error:
                cleanup_error = f"{type(error).__name__}: {error}"
                reason = reason or "Owned package process cleanup failed"
        result = {"command": command, "exit_code": exit_code, "stop_reason": reason,
                  "elapsed_seconds": round(time.monotonic() - started, 1), "peak_rss_bytes": peak_rss,
                  "memory_limit_bytes": limit, "minimum_free_disk_bytes": minimum_free,
                  "jobs": 2, "timeout_seconds": timeout_seconds, "cleanup_error": cleanup_error,
                  "owned_process_reaped": process is not None and process.returncode is not None}
        if log.exists():
            result["log"] = file_info(log)
        try:
            if process is not None and candidate.is_file():
                shutil.copyfile(candidate, out / candidate.name)
                result["produced_archive"] = file_info(out / candidate.name)
        except OSError as error:
            result["archive_retention_error"] = str(error)
            reason = result["stop_reason"] = reason or "Failed to retain produced archive"
        save_json(out / "package-status.json", result)
    if log.exists():
        print("\n".join(log.read_text(errors="replace").splitlines()[-30:]))
    if caught is not None:
        raise RuntimeError(reason) from caught
    assert exit_code == 0 and reason is None and cleanup_error is None, f"Package failed: {result}"
    return result


def verify(args):
    root, out = args.source.resolve(), args.output.resolve()
    pre = json.loads((out / "preflight.json").read_text())
    assert pre["source"] == SOURCE
    clean_source(root)
    target = Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    target.mkdir(parents=True, exist_ok=True)
    result = bounded_package(root, out, target, pre["toolchain"])
    archive = target / "package" / f"iris-chat-{CLI}.crate"
    local = out / archive.name
    shutil.copyfile(archive, local)
    inspection = inspect_archive(local, "iris-chat", CLI, "core", lambda p: read_source(root, p), pre["protocol_checksum"])
    clean_source(root)
    receipt = {"schema": "iris-cli-source-package-verification/v1", "source": SOURCE,
               "preflight": file_info(out / "preflight.json"), "package_run": result,
               "package": inspection, "source_clean_after": True,
               "cache": {key: os.environ.get(key, "") for key in ["IRIS_CACHE_REQUESTED", "IRIS_CACHE_MATCHED", "IRIS_CACHE_HIT"]},
               "verification": "Real cargo package rebuilt the extracted archive; default CLI features only. No test suite or release-binary claim.",
               "published": False}
    save_json(out / "acceptance.json", receipt)
    (out / "SHA256SUMS").write_text(f"{file_info(local)['sha256']}  {local.name}\n")
    print(json.dumps({"package_verified": True, "archive": file_info(local)}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=["preflight", "verify"])
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--toolchain", default="")
    parser.add_argument("--protocol-sha256", default="")
    arguments = parser.parse_args()
    {"preflight": preflight, "verify": verify}[arguments.stage](arguments)
