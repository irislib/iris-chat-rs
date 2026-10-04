#!/usr/bin/env python3
"""Build-only, public-source updater diagnostics. Never runs a network client."""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import tarfile
import time
import tomllib
import urllib.request

HERE = Path(__file__).resolve().parent
FLOOR = 5 * 1024**3


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_inputs():
    inputs = json.loads((HERE / "inputs.json").read_text())
    for name, expected in inputs["payload_sha256"].items():
        assert Path(name).name == name, "Non-flat input path"
        assert digest(HERE / name) == expected, f"Input hash mismatch: {name}"
    return inputs


def unpack_crate(archive, destination, package_root):
    # Registry archives may contain only regular files and directories.
    with tarfile.open(archive) as tar:
        for member in tar.getmembers():
            parts = Path(member.name).parts
            if (not parts or parts[0] != package_root or ".." in parts
                    or Path(member.name).is_absolute()
                    or not (member.isfile() or member.isdir())):
                raise ValueError("Unsafe archive member")
        tar.extractall(destination, filter="data")


def lock_shape(lock, patches):
    rows = []
    for row in lock["package"]:
        row = dict(row)
        if (row["name"], row["version"]) in patches:
            row.pop("source", None)
            row.pop("checksum", None)
        rows.append(json.dumps(row, sort_keys=True))
    return collections.Counter(rows)


def check_platform(role, system, machine):
    expected = ("Linux", "x86_64") if role == "client" else ("Darwin", "arm64")
    if (system, machine) != expected:
        raise RuntimeError(f"Wrong diagnostic platform: {system}/{machine}, expected {expected}")


def run(args, cwd=None, env=None, timeout=120):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True, timeout=timeout)


def prepare(role, root, inputs):
    root.mkdir(parents=True, exist_ok=False)
    source = root / "source"
    spec = inputs["sources"][role]
    run(["git", "init", "-q", str(source)])
    run(["git", "-C", str(source), "fetch", "--depth=1", spec["repository"], spec["commit"]])
    run(["git", "-C", str(source), "checkout", "--detach", "FETCH_HEAD"])
    assert run(["git", "-C", str(source), "rev-parse", "HEAD"]).strip() == spec["commit"]
    patch = "provider-base.patch" if role == "provider" else "client.patch"
    run(["git", "-C", str(source), "apply", "--check", str(HERE / patch)])
    run(["git", "-C", str(source), "apply", str(HERE / patch)])
    if role == "client":
        shutil.copy2(HERE / "fleet_trace.rs", source / "core/src/fleet_update_trace.rs")
    manifest = source / spec["manifest"]
    lock = manifest.parent / "Cargo.lock"
    baseline = tomllib.loads(lock.read_text())
    packages = [p for p in inputs["packages"]
                if p["role"] == role or (role == "client" and p["role"] == "updater")]
    config = root / "overrides.toml"
    rows = ["[patch.crates-io]"]
    for package in packages:
        name, version = package["name"], package["version"]
        matching = [p for p in baseline["package"] if (p["name"], p["version"]) == (name, version)]
        assert len(matching) == 1 and matching[0]["checksum"] == package["sha256"]
        archive = root / f"{name}-{version}.crate"
        url = f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
        with urllib.request.urlopen(url, timeout=60) as response, archive.open("xb") as output:
            shutil.copyfileobj(response, output)
        assert digest(archive) == package["sha256"], "Registry archive hash mismatch"
        vendor = root / "vendor"
        vendor.mkdir(exist_ok=True)
        unpack_crate(archive, vendor, f"{name}-{version}")
        package_dir = vendor / f"{name}-{version}"
        patch = HERE / f'{package["role"]}-registry.patch'
        run(["git", "apply", "--check", str(patch)], cwd=package_dir)
        run(["git", "apply", str(patch)], cwd=package_dir)
        shutil.copy2(HERE / "fleet_trace.rs", package_dir / "src/fleet_trace.rs")
        rows.append(f'{name} = {{ path = {json.dumps(str(package_dir))} }}')
    config.write_text("\n".join(rows) + "\n")
    env = os.environ.copy()
    # No caller-specific resolver routes, signers, stores, or release credentials.
    for key in list(env):
        if key.startswith(("IRIS_", "HTREE_", "FLEET_")):
            env.pop(key)
    env.update(CARGO_TARGET_DIR=str(root / "target"), CARGO_BUILD_JOBS="1",
               CARGO_INCREMENTAL="0", CARGO_TERM_COLOR="never",
               RUSTFLAGS=f"--remap-path-prefix={root}=/diagnostic --remap-path-prefix={Path.home()}/.cargo=/cargo")
    if role == "client":
        env.update(IRIS_APP_VERSION_NAME="2026.10.4.2", IRIS_BUILD_GIT_SHA=spec["commit"],
                   IRIS_BUILD_CHANNEL="diagnostic")
    command = ["cargo", "metadata", "--format-version=1", "--manifest-path", str(manifest),
               "--config", str(config)]
    metadata = json.loads(run(command, env=env, timeout=300))
    after = tomllib.loads(lock.read_text())
    patch_keys = {(p["name"], p["version"]) for p in packages}
    assert lock_shape(baseline, patch_keys) == lock_shape(after, patch_keys), "Dependency graph changed"
    for package in packages:
        matches = [p for p in metadata["packages"]
                   if (p["name"], p["version"]) == (package["name"], package["version"])]
        assert len(matches) == 1 and matches[0]["source"] is None, "Override not selected"
    run(command + ["--locked"], env=env, timeout=300)
    return spec, source, manifest, config, lock, env


def bounded_build(command, root, env, log_path, timeout=3600):
    if shutil.disk_usage(root).free < FLOOR:
        raise RuntimeError("Disk floor before build")
    started = time.monotonic()
    with log_path.open("wb") as log:
        process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            while process.poll() is None:
                if (time.monotonic() - started > timeout or shutil.disk_usage(root).free < FLOOR
                        or log_path.stat().st_size > 16 * 1024**2):
                    raise RuntimeError("Diagnostic build time/disk/log bound reached")
                time.sleep(2)
            if process.returncode:
                raise RuntimeError(f"Diagnostic build failed: exit {process.returncode}")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--role", choices=("client", "provider"))
    parser.add_argument("--root", type=Path)
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()
    inputs = verify_inputs()
    if args.verify_only:
        print("Immutable public-source diagnostic inputs verified")
        return
    if args.role is None or args.root is None:
        parser.error("--role and a fresh --root are required")
    check_platform(args.role, platform.system(), platform.machine())
    root = args.root.resolve()
    spec, source, manifest, config, lock, env = prepare(args.role, root, inputs)
    output = root / "output"
    output.mkdir()
    helper_test = root / "helper-test"
    run(["rustc", "--edition=2021", "--test", str(HERE / "fleet_trace.rs"), "-o", str(helper_test)])
    tests = run([str(helper_test)])
    (output / "helper-tests.txt").write_text(tests)
    command = ["cargo", "build", "--locked", "--manifest-path", str(manifest), "--config", str(config),
               "-p", spec["package"], "--bin", spec["bin"], "--jobs", "1"]
    if spec["profile"] == "release":
        command.append("--release")
    log = output / "build.log"
    bounded_build(command, root, env, log)
    profile = "release" if spec["profile"] == "release" else "debug"
    binary = root / "target" / profile / spec["bin"]
    final = output / f'{spec["bin"]}-{args.role}-DIAGNOSTIC'
    shutil.copy2(binary, final)
    shutil.copy2(lock, output / "Cargo.lock")
    # Version/help only: never initialize a wallet, identity, listener or provider.
    version = run([str(final), "--version"], env=env, timeout=15).strip()
    receipt = {"diagnostic_only": True, "role": args.role, "source": spec["commit"],
               "profile": spec["profile"], "input_manifest_sha256": digest(HERE / "inputs.json"),
               "binary_sha256": digest(final), "binary_bytes": final.stat().st_size,
               "lock_sha256": digest(lock), "version": version,
               "rustc": run(["rustc", "--version"]).strip(),
               "platform": platform.system(), "architecture": platform.machine(),
               "glibc": run(["getconf", "GNU_LIBC_VERSION"]).strip() if platform.system() == "Linux" else None,
               "binary_format": run(["file", "-b", str(final)]).strip(),
               "helper_tests_passed": 3, "dependency_graph_unchanged": True,
               "integration_tested": False, "network_probe_run": False,
               "provider_fixture_limit": inputs["provider_test_fixture"]}
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
