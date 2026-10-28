#!/usr/bin/env python3
"""Install reviewed Android build inputs in an owned directory, on hosted CI only.

--check is source-only. Downloads reuse the release installer's bounded verifier;
no SDK manager, mutable Java resolver, system SDK changes or rustup are used.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import platform
import posixpath
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import urllib.parse
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PINS = Path(__file__).with_suffix(".json")
SPEC = importlib.util.spec_from_file_location("release_build_inputs", ROOT / "scripts/release-build-inputs.py")
SHARED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SHARED)
require, digest = SHARED.require, SHARED.digest
HOST_TOOL_EXPORTS = {"CMAKE": "/usr/bin/cmake", "PKG_CONFIG": "/usr/bin/pkg-config"}
HOST_PACKAGES = ("cmake", "libdbus-1-dev", "pkg-config", "pkgconf", "pkgconf-bin")


def archive_pin(pin):
    return {**pin, "size": pin["bytes"]}


def check_pins(pins):
    require(pins["schema"] == 1, "Unknown Android input schema")
    packages = {pin["package"] for pin in pins["sdk_linux"]}
    require(packages == {"platforms;android-36", "build-tools;36.0.0", "ndk;28.2.13676358"},
            "Unexpected Android SDK package set")
    require(pins["cargo_ndk"]["version"] == "4.1.2", "Unexpected cargo-ndk version")
    require(pins["jdk"]["version"] == "17.0.20+8", "Unexpected reviewed JDK version")
    entries = [*pins["sdk_linux"], pins["cargo_ndk"], pins["jdk"]]
    for pin in entries:
        SHARED.native_url(pin["url"])
        parsed = urllib.parse.urlsplit(pin["url"])
        require(not parsed.query and "/latest/" not in parsed.path,
                "Input URL must identify a fixed release")
        require(re.fullmatch("[0-9a-f]{64}", pin["sha256"]) is not None and
                0 < pin["bytes"] <= 1024**3, "Input requires a fixed size and SHA-256")
    for pin in pins["sdk_linux"]:
        require(pin["url"].startswith("https://dl.google.com/android/repository/") and
                pin["format"] == "zip" and "Pkg.Revision" in pin["source_properties"],
                "Invalid SDK source binding")
    crate = pins["cargo_ndk"]
    require(crate["url"] == "https://static.crates.io/crates/cargo-ndk/cargo-ndk-4.1.2.crate" and
            crate["bundled_lock"]["path"] == "cargo-ndk-4.1.2/Cargo.lock" and
            re.fullmatch("[0-9a-f]{64}", crate["bundled_lock"]["sha256"]) is not None,
            "Invalid cargo-ndk archive/lock binding")
    wrapper = properties(ROOT / "android/gradle/wrapper/gradle-wrapper.properties")
    require(wrapper.get("distributionUrl", "").replace(r"\:", ":") == pins["gradle"]["distribution_url"] and
            wrapper.get("distributionSha256Sum") == pins["gradle"]["distribution_sha256"] and
            digest(ROOT / "android/gradle/wrapper/gradle-wrapper.jar") == pins["gradle"]["wrapper_jar_sha256"],
            "Gradle wrapper differs from reviewed distribution/JAR pins")


def properties(path):
    return {key.strip(): value.strip().strip('"') for line in Path(path).read_text().splitlines()
            if "=" in line and not line.lstrip().startswith("#")
            for key, value in [line.split("=", 1)]}


def safe_name(name):
    require(name and not name.startswith("/") and "\\" not in name and ":" not in name and
            not {".", ".."}.intersection(name.rstrip("/").split("/")), "Unsafe archive path")
    return PurePosixPath(name)


def unpack_sdk(archive_path, pin, destination):
    SHARED.verify_native(archive_path, archive_pin(pin))  # Before even ZIP parsing.
    require(not destination.exists(), "SDK extraction directory already exists")
    with zipfile.ZipFile(archive_path) as archive:
        members = archive.infolist()
        require(len(members) <= 100000 and sum(m.file_size for m in members) <= 8 * 1024**3,
                "SDK extraction exceeds its bound")
        names = [safe_name(member.filename) for member in members]
        roots = {name.parts[0] for name in names}
        require(len(roots) == 1 and len(set(map(str, names))) == len(names),
                "SDK archive requires one root and unique paths")
        root = next(iter(roots))
        links = {}
        for member, name in zip(members, names):
            mode = stat.S_IFMT(member.external_attr >> 16)
            require(mode in (0, stat.S_IFREG, stat.S_IFDIR, stat.S_IFLNK), "Special SDK archive file")
            if mode == stat.S_IFLNK:
                require(member.file_size <= 4096, "Oversized SDK symlink")
                target = archive.read(member).decode("utf-8")
                require(target and not target.startswith("/") and "\\" not in target and ":" not in target,
                        "Absolute SDK symlink")
                resolved = posixpath.normpath(posixpath.join(str(name.parent), target))
                require(resolved.startswith(root + "/"), "SDK symlink escapes its package")
                links[str(name)] = target
        require(not any(str(name).startswith(link + "/") for name in names for link in links),
                "SDK archive writes through a symlink")
        destination.mkdir()
        for member, name in zip(members, names):
            if str(name) in links:
                continue
            target = destination / str(name)
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(member) as source, target.open("xb") as output:
                    shutil.copyfileobj(source, output, length=1024 * 1024)
                mode = (member.external_attr >> 16) & 0o777
                if mode:
                    target.chmod(mode)
        for name, target in links.items():
            link = destination / name
            link.parent.mkdir(parents=True, exist_ok=True)
            link.symlink_to(target)
        package_root = (destination / root).resolve()
        for name in links:
            resolved = (destination / name).resolve(strict=True)
            require(package_root in resolved.parents, "Resolved SDK symlink escapes its package")
    package = destination / root
    actual = properties(package / "source.properties")
    require(all(actual.get(key) == value for key, value in pin["source_properties"].items()),
            "Installed SDK source.properties differs from reviewed package")
    return package


def unpack_cargo_ndk(archive_path, pin, destination):
    SHARED.verify_native(archive_path, archive_pin(pin))
    require(hasattr(tarfile, "data_filter"), "Python with tarfile.data_filter is required")
    require(not destination.exists(), "Crate extraction directory already exists")
    root = "cargo-ndk-" + pin["version"]
    with tarfile.open(archive_path, "r:gz") as archive:
        members = archive.getmembers()
        require(len(members) <= 10000 and sum(m.size for m in members) <= 2 * 1024**2,
                "Cargo-ndk source extraction exceeds its bound")
        for member in members:
            require(safe_name(member.name).parts[0] == root and
                    (member.isfile() or member.isdir()), "Unexpected crate path or link")
        destination.mkdir()
        archive.extractall(destination, filter="data")
    lock = destination / pin["bundled_lock"]["path"]
    require(lock.stat().st_size == pin["bundled_lock"]["bytes"] and
            digest(lock) == pin["bundled_lock"]["sha256"], "Bundled cargo-ndk lockfile checksum mismatch")
    return destination / root


def rust_environment(rust_work):
    receipt = json.loads((rust_work / "receipt.json").read_text())
    require(receipt.get("passed") is True and receipt["sourceCommit"] == SHARED.checkout_commit() and
            receipt["pinsSHA256"] == digest(SHARED.PINS) and
            receipt["installerSHA256"] == digest(ROOT / "scripts/release-build-inputs.py"),
            "Android requires this source's successful pinned Rust installation")
    prefix = rust_work / "toolchain"
    env = dict(os.environ)
    for key, binary in (("CARGO", "cargo"), ("RUSTC", "rustc"), ("RUSTDOC", "rustdoc")):
        require(env.get(key) == str(prefix / "bin" / binary), "Pinned Rust executable differs: " + key)
    SHARED.validate_install(prefix, "x86_64-unknown-linux-gnu",
                            ["x86_64-unknown-linux-gnu", "aarch64-linux-android"], [], env)
    return env, digest(rust_work / "receipt.json")


def host_prerequisites():
    versions = {"packages": subprocess.check_output(
        ["dpkg-query", "-W", "-f=${Package}=${Version}\n", *HOST_PACKAGES],
        text=True, timeout=30).strip()}
    paths = {"cc": shutil.which("cc"), "cmake": HOST_TOOL_EXPORTS["CMAKE"],
             "pkg-config": HOST_TOOL_EXPORTS["PKG_CONFIG"]}
    for name, path in paths.items():
        require(path is not None and Path(path).is_file() and os.access(path, os.X_OK),
                "Missing hosted prerequisite: " + name)
        versions[name] = {"path": path, "sha256": digest(path), "version": subprocess.check_output(
            [path, "--version"], text=True, timeout=30).strip()}
    return versions


def install(pins, work, rust_work):
    check_pins(pins)
    require(os.environ.get("GITHUB_ACTIONS") == "true" and platform.system() == "Linux" and
            platform.machine() == "x86_64", "Android installation requires hosted Linux x64 CI")
    require(hasattr(tarfile, "data_filter"), "Python with tarfile.data_filter is required")
    require(work.is_absolute() and rust_work.is_absolute() and not any(c in str(work) for c in "\r\n"),
            "Absolute safe work directories are required")
    require(not (ROOT / "android/local.properties").exists(), "Preexisting SDK override requires review")
    env, rust_receipt_sha = rust_environment(rust_work)
    work.mkdir(mode=0o700, parents=True, exist_ok=False)
    archives, sdk = work / "archives", work / "sdk"
    archives.mkdir()
    sdk.mkdir()
    receipt = {"passed": False, "pinsSHA256": digest(PINS), "installerSHA256": digest(Path(__file__)),
               "sharedInstallerSHA256": digest(ROOT / "scripts/release-build-inputs.py"),
               "sourceCommit": SHARED.checkout_commit(), "rustReceiptSHA256": rust_receipt_sha,
               "sdk": [], "externalAssumptions": pins["external_assumptions"]}
    try:
        receipt["hostPrerequisites"] = host_prerequisites()
        env.update(HOST_TOOL_EXPORTS)
        jdk_pin = archive_pin(pins["jdk"])
        jdk_archive = SHARED.download_native(jdk_pin, archives / "jdk.tar.gz")
        java = SHARED.unpack_native(jdk_archive, jdk_pin, work / "jdk")
        java_home = java.parent.parent
        release = properties(java_home / "release")
        require(release.get("JAVA_RUNTIME_VERSION") == pins["jdk"]["version"], "Installed JDK version mismatch")
        version = subprocess.check_output([str(java), "-version"], stderr=subprocess.STDOUT,
                                          text=True, timeout=30).strip()
        require(pins["jdk"]["version"] in version, "JDK executable version mismatch")
        receipt["jdk"] = {"input": pins["jdk"], "version": version}
        for index, pin in enumerate(pins["sdk_linux"]):
            archive = SHARED.download_native(archive_pin(pin), archives / f"sdk-{index}.zip")
            package = unpack_sdk(archive, pin, work / f"sdk-extract-{index}")
            target = sdk / pin["package"].replace(";", "/")
            target.parent.mkdir(parents=True, exist_ok=True)
            package.rename(target)
            receipt["sdk"].append({"input": pin, "sourcePropertiesSHA256": digest(target / "source.properties")})
        crate_pin = pins["cargo_ndk"]
        crate = SHARED.download_native(archive_pin(crate_pin), archives / "cargo-ndk.crate")
        source = unpack_cargo_ndk(crate, crate_pin, work / "cargo-ndk-source")
        cargo_prefix = work / "cargo-ndk"
        subprocess.run([env["CARGO"], "install", "--path", str(source), "--locked", "--root",
                        str(cargo_prefix), "--jobs", "2"], env=env, check=True, timeout=900)
        cargo_ndk = cargo_prefix / "bin/cargo-ndk"
        ndk_version = subprocess.check_output([str(cargo_ndk), "--version"], env=env, text=True, timeout=30).strip()
        require(ndk_version == "cargo-ndk " + crate_pin["version"], "cargo-ndk executable version mismatch")
        receipt["cargoNdk"] = {"input": crate_pin, "version": ndk_version, "binarySHA256": digest(cargo_ndk)}
        # Reuse dependency artifacts, but never execute a restored wrapper distribution
        # without its original ZIP proof. The checked wrapper verifies its fresh ZIP.
        gradle_home = work / "gradle-home"
        gradle_home.mkdir()
        dependency_cache = Path.home() / ".gradle/caches"
        dependency_cache.mkdir(parents=True, exist_ok=True)
        (gradle_home / "caches").symlink_to(dependency_cache, target_is_directory=True)
        exports = {"JAVA_HOME": str(java_home), "ANDROID_HOME": str(sdk), "ANDROID_SDK_ROOT": str(sdk),
                   **HOST_TOOL_EXPORTS,
                   "GRADLE_USER_HOME": str(gradle_home),
                   "ANDROID_NDK_HOME": str(sdk / "ndk/28.2.13676358")}
        with open(os.environ["GITHUB_ENV"], "a") as output:
            for key, value in exports.items():
                output.write(key + "=" + value + "\n")
        with open(os.environ["GITHUB_PATH"], "a") as output:
            output.write(str(java.parent) + "\n" + str(cargo_ndk.parent) + "\n")
        receipt["passed"] = True
    finally:
        (work / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--install", action="store_true")
    parser.add_argument("--work", type=Path)
    parser.add_argument("--rust-work", type=Path)
    args = parser.parse_args()
    pins = json.loads(PINS.read_text())
    check_pins(pins)
    if args.install:
        require(args.work is not None and args.rust_work is not None, "--work and --rust-work are required")
        install(pins, args.work, args.rust_work)
    else:
        print("Reviewed Android SDK, JDK and cargo-ndk pins; no native process or download executed.")


if __name__ == "__main__":
    main()
