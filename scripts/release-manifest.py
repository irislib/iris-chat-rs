#!/usr/bin/env python3

"""Create and verify immutable Iris Chat release manifests."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import sys
from datetime import date, datetime
from pathlib import Path


TAG_RE = re.compile(
    r"^v(?P<year>\d{4})\.(?P<month>[1-9]\d?)\.(?P<day>[1-9]\d?)"
    r"(?:\.(?P<corrective>[1-9]\d?))?$"
)
PLATFORM_POLICY = Path(__file__).resolve().parents[1] / "release-platforms.json"


def excluded_platforms(tag: str) -> list[str]:
    """Only explicit, committed per-tag iOS omissions may alter the inventory."""
    require_tag(tag)
    policy = json.loads(PLATFORM_POLICY.read_text(encoding="utf-8"))
    if not isinstance(policy, dict) or set(policy) != {
        "schema_version", "excluded_platforms_by_tag"
    } or policy.get("schema_version") != 1:
        raise ValueError("unsupported release platform policy")
    overrides = policy["excluded_platforms_by_tag"]
    if not isinstance(overrides, dict):
        raise ValueError("invalid release platform overrides")
    for release_tag, exclusions in overrides.items():
        require_tag(release_tag)
        if exclusions not in ([], ["ios"]):
            raise ValueError(f"only ios may be excluded from a release: {release_tag}")
    return overrides.get(tag, [])


def asset_specs(tag: str) -> dict[str, tuple[str, str, str]]:
    specs = {
        f"iris-chat-{tag}-android-arm64.apk": ("android", "arm64", "apk"),
        f"iris-chat-{tag}-android-arm64.aab": ("android", "arm64", "aab"),
        f"iris-chat-{tag}-ios.ipa": ("ios", "universal", "ipa"),
        f"iris-chat-{tag}-ios.xcarchive.zip": ("ios", "universal", "xcarchive"),
        f"iris-chat-{tag}-macos-arm64.dmg": ("macos", "arm64", "dmg"),
        f"iris-chat-{tag}-macos-arm64.app.tar.gz": ("macos", "arm64", "updater"),
        f"iris-chat-{tag}-windows-x64-setup.exe": ("windows", "x64", "installer"),
        f"iris-chat-{tag}-windows-x64.zip": ("windows", "x64", "portable"),
        f"iris-chat-{tag}-linux-x64.deb": ("linux", "x64", "deb"),
        f"iris-chat-{tag}-linux-x64.tar.gz": ("linux", "x64", "portable"),
        f"iris-{tag}-aarch64-apple-darwin.tar.gz": ("cli", "macos-arm64", "archive"),
        f"iris-{tag}-x86_64-apple-darwin.tar.gz": ("cli", "macos-x64", "archive"),
        f"iris-{tag}-x86_64-unknown-linux-gnu.tar.gz": (
            "cli",
            "linux-x64",
            "archive",
        ),
    }
    excluded = excluded_platforms(tag)
    return {name: spec for name, spec in specs.items() if spec[0] not in excluded}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_tag(tag: str) -> None:
    match = TAG_RE.fullmatch(tag)
    if match is None:
        raise ValueError(f"unsupported stable release tag: {tag}")
    try:
        date(
            int(match.group("year")),
            int(match.group("month")),
            int(match.group("day")),
        )
    except ValueError as error:
        raise ValueError(
            f"unsupported stable release tag: {tag} ({error})"
        ) from error


def create_manifest(tag: str, commit: str, asset_dir: Path, output: Path) -> None:
    require_tag(tag)
    specs = asset_specs(tag)
    actual = {path.name for path in asset_dir.iterdir() if path.is_file()}
    expected = set(specs)
    if actual != expected:
        missing = sorted(expected - actual)
        unexpected = sorted(actual - expected)
        details = []
        if missing:
            details.append(f"missing: {', '.join(missing)}")
        if unexpected:
            details.append(f"unexpected: {', '.join(unexpected)}")
        raise ValueError("release asset set mismatch (" + "; ".join(details) + ")")

    assets = []
    for name in sorted(expected):
        path = asset_dir / name
        platform, architecture, kind = specs[name]
        assets.append(
            {
                "name": name,
                "path": f"assets/{name}",
                "platform": platform,
                "architecture": architecture,
                "kind": kind,
                "size": path.stat().st_size,
                "sha256": sha256(path),
            }
        )
    manifest = {
        "schema_version": 1,
        "tag": tag,
        "version": tag.removeprefix("v"),
        "commit": commit,
        "excluded_platforms": excluded_platforms(tag),
        "assets": assets,
    }
    output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def verify_manifest(
    tag: str,
    manifest_path: Path,
    asset_dir: Path,
    required_names: list[str],
    expected_commit: str | None = None,
) -> None:
    require_tag(tag)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported release manifest schema")
    if manifest.get("tag") != tag or manifest.get("version") != tag.removeprefix("v"):
        raise ValueError("release manifest tag/version mismatch")
    if expected_commit is not None and manifest.get("commit") != expected_commit:
        raise ValueError("release manifest commit mismatch")
    if manifest.get("excluded_platforms", []) != excluded_platforms(tag):
        raise ValueError("release manifest platform policy mismatch")

    specs = asset_specs(tag)
    entries = manifest.get("assets")
    if not isinstance(entries, list):
        raise ValueError("release manifest has no asset list")
    by_name: dict[str, dict[str, object]] = {}
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("name"), str):
            raise ValueError("release manifest contains an invalid asset entry")
        name = str(entry["name"])
        if name in by_name:
            raise ValueError(f"duplicate release manifest asset: {name}")
        if name not in specs:
            raise ValueError(f"unexpected release manifest asset: {name}")
        if entry.get("path") != f"assets/{name}":
            raise ValueError(f"unsafe release manifest path: {name}")
        by_name[name] = entry

    expected_manifest_names = set(specs)
    if set(by_name) != expected_manifest_names:
        raise ValueError("release manifest does not contain the canonical asset set")

    names = required_names or sorted(path.name for path in asset_dir.iterdir() if path.is_file())
    for name in names:
        entry = by_name.get(name)
        if entry is None:
            raise ValueError(f"required asset is absent from manifest: {name}")
        path = asset_dir / name
        if not path.is_file():
            raise ValueError(f"required asset was not downloaded: {name}")
        if path.stat().st_size != entry.get("size"):
            raise ValueError(f"release asset size mismatch: {name}")
        if sha256(path) != entry.get("sha256"):
            raise ValueError(f"release asset digest mismatch: {name}")


def stage_hashtree(
    tag: str,
    manifest_path: Path,
    asset_dir: Path,
    notes_path: Path,
    output_dir: Path,
    published_at: str,
) -> None:
    verify_manifest(tag, manifest_path, asset_dir, list(asset_specs(tag)))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    timestamp = int(
        datetime.fromisoformat(published_at.replace("Z", "+00:00")).timestamp()
    )
    output_assets = output_dir / "assets"
    output_assets.mkdir(parents=True, exist_ok=False)
    staged_assets = []
    for entry in manifest["assets"]:
        source = asset_dir / entry["name"]
        shutil.copy2(source, output_assets / entry["name"])
        staged = dict(entry)
        # The attested inventory describes file formats; the updater's `kind`
        # describes how to install them. Keep the immutable inventory intact
        # and translate only this generated discovery document.
        staged["kind"] = {
            "updater": "app-bundle",
            "installer": "nsis",
            "deb": "deb",
        }.get(entry["kind"], "archive")
        if entry["platform"] == "cli":
            staged["kind"] = "binary-archive"
            staged["executable"] = "iris/iris"
        staged_assets.append(staged)
    manifest_name = manifest_path.name
    shutil.copy2(manifest_path, output_assets / manifest_name)
    staged_assets.append(
        {
            "name": manifest_name,
            "path": f"assets/{manifest_name}",
            "platform": "metadata",
            "architecture": "none",
            "kind": "binary",
            "size": manifest_path.stat().st_size,
            "sha256": sha256(manifest_path),
        }
    )

    release = {
        "id": tag,
        "title": tag,
        "tag": tag,
        # Older desktop updaters require SemVer here, but compare the display
        # tag themselves. Retain the date revision in build metadata so they
        # can discover corrective releases without rejecting the document.
        "version": "+".join(manifest["version"].rsplit(".", 1))
        if manifest["version"].count(".") == 3 else manifest["version"],
        "commit": manifest["commit"],
        "created_at": timestamp,
        "published_at": timestamp,
        "draft": False,
        "prerelease": False,
        "notes_file": "notes.md",
        "assets": staged_assets,
    }
    (output_dir / "release.json").write_text(
        json.dumps(release, indent=2) + "\n", encoding="utf-8"
    )
    shutil.copy2(notes_path, output_dir / "notes.md")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    create = subparsers.add_parser("create")
    create.add_argument("--tag", required=True)
    create.add_argument("--commit", required=True)
    create.add_argument("--asset-dir", required=True, type=Path)
    create.add_argument("--out", required=True, type=Path)

    validate_tag = subparsers.add_parser("validate-tag")
    validate_tag.add_argument("--tag", required=True)

    inventory = subparsers.add_parser("list-assets")
    inventory.add_argument("--tag", required=True)
    for command in ("platform-enabled", "require-platform"):
        platform = subparsers.add_parser(command)
        platform.add_argument("--tag", required=True)
        platform.add_argument("--platform", required=True,
                              choices=["android", "ios", "macos", "windows", "linux", "cli"])

    verify = subparsers.add_parser("verify")
    verify.add_argument("--tag", required=True)
    verify.add_argument("--manifest", required=True, type=Path)
    verify.add_argument("--asset-dir", required=True, type=Path)
    verify.add_argument("--require-name", action="append", default=[])
    verify.add_argument("--commit")

    stage = subparsers.add_parser("stage-hashtree")
    stage.add_argument("--tag", required=True)
    stage.add_argument("--manifest", required=True, type=Path)
    stage.add_argument("--asset-dir", required=True, type=Path)
    stage.add_argument("--notes", required=True, type=Path)
    stage.add_argument("--out-dir", required=True, type=Path)
    stage.add_argument("--published-at", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.command == "create":
        create_manifest(args.tag, args.commit, args.asset_dir, args.out)
    elif args.command == "validate-tag":
        require_tag(args.tag)
    elif args.command == "list-assets":
        print(*sorted(asset_specs(args.tag)), sep="\n")
    elif args.command in ("platform-enabled", "require-platform"):
        enabled = args.platform not in excluded_platforms(args.tag)
        if args.command == "require-platform" and not enabled:
            raise ValueError(f"{args.platform} artifacts are excluded from {args.tag}; distribution is unavailable")
        if args.command == "platform-enabled":
            print("true" if enabled else "false")
    elif args.command == "verify":
        verify_manifest(
            args.tag,
            args.manifest,
            args.asset_dir,
            args.require_name,
            args.commit,
        )
    else:
        stage_hashtree(
            args.tag,
            args.manifest,
            args.asset_dir,
            args.notes,
            args.out_dir,
            args.published_at,
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
