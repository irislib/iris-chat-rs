#!/usr/bin/env python3
"""Check release versions and package metadata without running source builds.

Requires Python 3.11+ for the standard-library TOML parser.
"""

import argparse
import copy
from datetime import date
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TAG = re.compile(r"v(\d{4})\.([1-9]\d?)\.([1-9]\d?)(?:\.([1-9]\d?))?")
SEMVER = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)")
VERSION_FIELDS = ("IRIS_APP_VERSION_NAME", "IRIS_APP_VERSION_CODE")
# The manifest's PowerShell recipe uses explicit, semicolon-separated literal
# assignments. Read those assignments, not arbitrary version strings in commands.
POWERSHELL_VERSION = re.compile(
    r"(?:^|;)\s*\$env:(IRIS_APP_VERSION_NAME|IRIS_APP_VERSION_CODE)"
    r"\s*=\s*(['\"])([^'\"]*)\2\s*(?=;|$)", re.IGNORECASE
)


def release_versions(tag):
    match = TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"unsupported release tag: {tag}")
    year, month, day, revision = (int(value or 0) for value in match.groups())
    date(year, month, day)
    code = year * 1000000 + month * 10000 + day * 100 + revision
    if not 0 < code <= 2100000000:
        raise ValueError("release version code is out of range")
    return (year, month, day, revision), (year, month, day * 100 + revision), code


def build_versions(commands):
    values = {field: [] for field in VERSION_FIELDS}
    for command in commands:
        if not isinstance(command, list) or not command or not all(isinstance(arg, str) for arg in command):
            raise ValueError("build commands must be argument arrays")
        if command[0] == "env":
            for argument in command[1:]:
                if "=" not in argument:
                    break
                name, value = argument.split("=", 1)
                if name in values:
                    values[name].append(value)
        elif command[0].lower() in ("pwsh", "pwsh.exe", "powershell", "powershell.exe"):
            options = [argument.lower() for argument in command]
            if "-command" in options:
                script = command[options.index("-command") + 1]
                for name, _, value in POWERSHELL_VERSION.findall(script):
                    values[name.upper()].append(value)
    return values


def validate(manifest, config, tag):
    app_version, package_version, version_code = release_versions(tag)
    if config.get("version_scheme") != "date-revision":
        raise ValueError("Haps release configuration must use date-revision versions")
    seen = set()
    for published in config["packages"]:
        name, target = published["name"], published["target"]
        label = f"{name}/{target}"
        if (name, target) in seen:
            raise ValueError(f"duplicate published package: {label}")
        seen.add((name, target))
        selected = manifest if name == manifest["package"]["name"] else manifest["packages"][name]
        document = copy.deepcopy(selected)
        package = document["package"]
        build = document.get("build", {})
        override = document.get("targets", {}).get(target, {})
        package.update(override.get("package", {}))
        build.update(override.get("build", {}))
        if package["target"] == "host":
            package["target"] = target
        match = SEMVER.fullmatch(package["version"])
        if match is None or tuple(map(int, match.groups())) != package_version:
            raise ValueError(f"{label}: package version does not match {tag}")
        expected = {key: published[key] for key in ("name", "target", "description", "commands", "app", "desktop", "dependencies") if key in published}
        expected["version"] = ".".join(map(str, package_version))
        if package != expected:
            raise ValueError(f"{label}: metadata differs from haps-release.json")
        if not build.get("commands") or not build.get("artifacts") or build.get("unavailable"):
            raise ValueError(f"{label}: source build commands and artifacts are required")
        versions = build_versions(build["commands"])
        names = versions["IRIS_APP_VERSION_NAME"]
        try:
            names_match = names and all(release_versions("v" + value)[0] == app_version for value in names)
        except ValueError:
            names_match = False
        if not names_match:
            raise ValueError(f"{label}: IRIS_APP_VERSION_NAME does not match {tag}")
        if name == "iris-chat" and target == "x86_64-unknown-linux-gnu":
            # Haps audits a source build, not our Docker/DEB release pipeline.
            # Keep its reachable tools explicit: no auto-installed cargo-deb,
            # container provisioning, or release-packaging shell scripts.
            expected_commands = [[
                "env", f"IRIS_APP_VERSION_NAME={names[0]}", "cargo", "build",
                "--manifest-path", "linux/Cargo.toml", "--locked", "--release",
                "--bin", "iris-chat", "--target", "{target}",
                "--target-dir", "target/haps-linux",
            ]]
            if (build["commands"] != expected_commands or
                    build["artifacts"].get("usr/bin/iris-chat") !=
                    "target/haps-linux/{target}/release/iris-chat"):
                raise ValueError(f"{label}: source recipe must use the native locked Cargo build")
        codes = versions["IRIS_APP_VERSION_CODE"]
        if ("windows" in target and not codes) or any(not value.isascii() or not value.isdecimal() or int(value) != version_code for value in codes):
            raise ValueError(f"{label}: IRIS_APP_VERSION_CODE does not match {tag}")
    if not seen:
        raise ValueError("no published Haps packages configured")
    return len(seen)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--manifest", type=Path, default=ROOT / "haps.toml")
    parser.add_argument("--config", type=Path, default=ROOT / "haps-release.json")
    args = parser.parse_args()
    try:
        manifest = tomllib.loads(args.manifest.read_text(encoding="utf-8"))
        config = json.loads(args.config.read_text(encoding="utf-8"))
        count = validate(manifest, config, args.tag)
    except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
        parser.exit(1, f"Haps manifest validation failed: {error}\n")
    print(f"Validated {count} Haps package variants for {args.tag}.")


if __name__ == "__main__":
    main()
