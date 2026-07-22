#!/usr/bin/env python3

"""Render release notes for both Hashtree and GitHub publication."""

import argparse
import re
import sys
from pathlib import Path
from typing import Iterable, List, Optional, Pattern, Sequence, Tuple
from urllib.parse import quote


TAG_RE = re.compile(r"^v(?P<version>\d+(?:\.\d+){2,3}(?:-[0-9A-Za-z.-]+)?)$")
PatternLabel = Tuple[str, Pattern[str]]

COMMON_DOWNLOADS: Sequence[PatternLabel] = (
    ("Iris Chat for macOS (Apple Silicon)", re.compile(r"^iris-chat-v.*-macos-arm64\.dmg$")),
    ("Iris Chat for Windows", re.compile(r"^iris-chat-v.*-windows-x64-setup\.exe$")),
    (
        "Iris Chat for Android",
        re.compile(r"^(?:iris-chat-v.*-android-arm64|IrisChat-release-.*)\.apk$"),
    ),
    ("Iris Chat for Debian/Ubuntu (.deb)", re.compile(r"^iris-chat-v.*-linux-x64\.deb$")),
)

CLI_DOWNLOADS: Sequence[PatternLabel] = (
    ("macOS Apple Silicon CLI", re.compile(r"^iris-aarch64-apple-darwin\.tar\.gz$")),
    ("macOS Intel CLI", re.compile(r"^iris-x86_64-apple-darwin\.tar\.gz$")),
    ("Linux x64 CLI", re.compile(r"^iris-x86_64-unknown-linux-gnu\.tar\.gz$")),
    ("Linux ARM64 CLI", re.compile(r"^iris-aarch64-unknown-linux-gnu\.tar\.gz$")),
)

ASSET_DESCRIPTIONS: Sequence[PatternLabel] = (
    ("macOS Apple Silicon updater archive", re.compile(r"^iris-chat-v.*-macos-arm64\.app\.tar\.gz$")),
    ("macOS Apple Silicon disk image", re.compile(r"^iris-chat-v.*-macos-arm64\.dmg$")),
    ("Windows x64 installer", re.compile(r"^iris-chat-v.*-windows-x64-setup\.exe$")),
    ("Windows x64 portable zip", re.compile(r"^iris-chat-v.*-windows-x64\.zip$")),
    (
        "Android APK",
        re.compile(r"^(?:iris-chat-v.*-android-arm64|IrisChat-release-.*)\.apk$"),
    ),
    (
        "Android App Bundle",
        re.compile(r"^(?:iris-chat-v.*-android-arm64|IrisChat-release-.*)\.aab$"),
    ),
    ("Linux x64 Debian package", re.compile(r"^iris-chat-v.*-linux-x64\.deb$")),
    ("Linux x64 portable tarball", re.compile(r"^iris-chat-v.*-linux-x64\.tar\.gz$")),
    ("iPhone/iPad IPA", re.compile(r"^(?:iris-chat-v.*-ios|IrisChat)\.ipa$")),
    ("iOS Xcode archive", re.compile(r"^iris-chat-v.*-ios\.xcarchive\.zip$")),
    *CLI_DOWNLOADS,
)


def release_section(changelog: str, version: str) -> str:
    heading = f"## {version}"
    lines = changelog.splitlines()
    try:
        start = lines.index(heading) + 1
    except ValueError as error:
        raise ValueError(f"CHANGELOG.md has no release section for {version}") from error

    end = len(lines)
    for index in range(start, len(lines)):
        if lines[index].startswith("## "):
            end = index
            break

    body = "\n".join(lines[start:end]).strip()
    if not body:
        raise ValueError(f"CHANGELOG.md release section for {version} is empty")
    return body


def asset_reference(name: str, asset_base_url: str) -> str:
    encoded_name = quote(name, safe="")
    if asset_base_url:
        return f"[{name}]({asset_base_url.rstrip('/')}/{encoded_name})"
    return f"[{name}](assets/{encoded_name})"


def first_match(assets: Sequence[str], pattern: Pattern[str]) -> Optional[str]:
    return next((name for name in assets if pattern.fullmatch(name)), None)


def describe_asset(name: str) -> str:
    for label, pattern in ASSET_DESCRIPTIONS:
        if pattern.fullmatch(name):
            return label
    return name


def append_download_group(
    lines: List[str],
    heading: str,
    choices: Sequence[PatternLabel],
    assets: Sequence[str],
    used: set[str],
    asset_base_url: str,
) -> None:
    entries = []
    for label, pattern in choices:
        name = first_match(assets, pattern)
        if name is not None:
            used.add(name)
            entries.append(f"- {label}: {asset_reference(name, asset_base_url)}")
    if entries:
        lines.extend(["", heading, "", *entries])


def render(
    tag: str,
    commit: str,
    changelog: str,
    assets: Iterable[str],
    *,
    asset_base_url: str = "",
    install_url: str = "",
    built_lines: Sequence[str] = (),
    skipped_lines: Sequence[str] = (),
    verification_lines: Sequence[str] = (),
) -> str:
    match = TAG_RE.fullmatch(tag)
    if not match:
        raise ValueError(f"unsupported release tag: {tag}")

    version = match.group("version")
    changes = release_section(changelog, version)
    sorted_assets = sorted(set(assets))
    used: set[str] = set()
    lines = [f"# Iris Chat {tag}", "", "## Downloads"]

    append_download_group(
        lines,
        "### Most People Will Want",
        COMMON_DOWNLOADS,
        sorted_assets,
        used,
        asset_base_url,
    )

    cli_lines: List[str] = []
    if install_url:
        cli_lines.append(f"- Install script: `curl -fsSL {install_url} | sh`")
    for label, pattern in CLI_DOWNLOADS:
        name = first_match(sorted_assets, pattern)
        if name is not None:
            used.add(name)
            cli_lines.append(f"- {label}: {asset_reference(name, asset_base_url)}")
    if cli_lines:
        lines.extend(["", "### Command Line", "", *cli_lines])

    other_lines = [
        f"- {describe_asset(name)}: {asset_reference(name, asset_base_url)}"
        for name in sorted_assets
        if name not in used
    ]
    if other_lines:
        lines.extend(["", "### Other Files", "", *other_lines])

    lines.extend(["", "## Changes", "", changes, "", "## Release Build", ""])
    lines.append(f"- Built from commit `{commit}` for release `{tag}`.")
    lines.extend(f"- {line}" for line in built_lines if line)

    visible_skipped = [line for line in skipped_lines if line]
    if visible_skipped:
        lines.extend(["", "## Skipped or Not Built", ""])
        lines.extend(f"- {line}" for line in visible_skipped)

    visible_verification = [line for line in verification_lines if line]
    if visible_verification:
        lines.extend(["", "## Verification", ""])
        lines.extend(f"- {line}" for line in visible_verification)

    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--asset-dir", required=True, type=Path)
    parser.add_argument("--asset-base-url", default="")
    parser.add_argument("--install-url", default="")
    parser.add_argument("--changelog", required=True, type=Path)
    parser.add_argument("--built-line", action="append", default=[])
    parser.add_argument("--skipped-line", action="append", default=[])
    parser.add_argument("--verification-line", action="append", default=[])
    parser.add_argument("--out", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.asset_dir.is_dir():
        raise ValueError(f"asset directory does not exist: {args.asset_dir}")
    assets = [path.name for path in args.asset_dir.iterdir() if path.is_file()]
    notes = render(
        args.tag,
        args.commit,
        args.changelog.read_text(encoding="utf-8"),
        assets,
        asset_base_url=args.asset_base_url,
        install_url=args.install_url,
        built_lines=args.built_line,
        skipped_lines=args.skipped_line,
        verification_lines=args.verification_line,
    )
    if args.out:
        args.out.write_text(notes, encoding="utf-8")
    else:
        sys.stdout.write(notes)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
