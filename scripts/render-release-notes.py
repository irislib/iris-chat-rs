#!/usr/bin/env python3

"""Validate and render channel-specific Iris Chat release notes."""

from __future__ import annotations

import argparse
import re
import sys
from datetime import date
from pathlib import Path
from typing import Iterable, Sequence
from urllib.parse import quote


TAG_RE = re.compile(
    r"^v(?P<year>\d{4})\.(?P<month>[1-9]\d?)\.(?P<day>[1-9]\d?)"
    r"(?:\.(?P<corrective>[1-9]\d?))?$"
)
RELEASE_HEADING_RE = re.compile(
    r"^## (v\d{4}\.[1-9]\d?\.[1-9]\d?(?:\.[1-9]\d?)?)$"
)
CHANNEL_HEADING_RE = re.compile(r"^### (GitHub|Apple|Zapstore)$")
CHANNELS = ("GitHub", "Apple", "Zapstore")


def require_tag(tag: str) -> None:
    match = TAG_RE.fullmatch(tag)
    if match is None:
        raise ValueError(f"unsupported release tag: {tag}")
    try:
        date(
            int(match.group("year")),
            int(match.group("month")),
            int(match.group("day")),
        )
    except ValueError as error:
        raise ValueError(f"unsupported release tag: {tag} ({error})") from error


def parse_notes(text: str) -> dict[str, dict[str, str]]:
    releases: dict[str, dict[str, str]] = {}
    current_tag: str | None = None
    current_channel: str | None = None
    buffer: list[str] = []

    def flush_channel() -> None:
        nonlocal buffer
        if current_tag is not None and current_channel is not None:
            body = "\n".join(buffer).strip()
            if current_channel in releases[current_tag]:
                raise ValueError(
                    f"duplicate {current_channel} section for {current_tag}"
                )
            releases[current_tag][current_channel] = body
        buffer = []

    for line in text.splitlines():
        release_match = RELEASE_HEADING_RE.fullmatch(line)
        if release_match:
            flush_channel()
            current_tag = release_match.group(1)
            require_tag(current_tag)
            current_channel = None
            if current_tag in releases:
                raise ValueError(f"duplicate release heading: {current_tag}")
            releases[current_tag] = {}
            continue

        channel_match = CHANNEL_HEADING_RE.fullmatch(line)
        if channel_match and current_tag is not None:
            flush_channel()
            next_channel = channel_match.group(1)
            expected_index = len(releases[current_tag])
            if expected_index >= len(CHANNELS) or next_channel != CHANNELS[expected_index]:
                expected = (
                    CHANNELS[expected_index]
                    if expected_index < len(CHANNELS)
                    else "<no additional section>"
                )
                raise ValueError(
                    f"{current_tag} expected ### {expected}, found ### {next_channel}"
                )
            current_channel = next_channel
            continue

        if current_tag is not None and line.startswith("### "):
            raise ValueError(f"{current_tag} has unsupported section heading: {line}")
        if current_tag is not None and line.startswith("## "):
            raise ValueError(f"unsupported release heading: {line}")
        if current_tag is not None and current_channel is None and line.strip():
            raise ValueError(f"{current_tag} has content before ### GitHub")

        if current_channel is not None:
            buffer.append(line)

    flush_channel()
    return releases


def validate_releases(releases: dict[str, dict[str, str]], tag: str | None) -> None:
    selected = [tag] if tag else list(releases)
    if tag and tag not in releases:
        raise ValueError(f"RELEASE_NOTES.md has no {tag} section")
    if not selected:
        raise ValueError("RELEASE_NOTES.md has no release sections")

    for release_tag in selected:
        require_tag(release_tag)
        sections = releases[release_tag]
        missing = [channel for channel in CHANNELS if not sections.get(channel)]
        if missing:
            raise ValueError(
                f"{release_tag} has missing or empty section(s): {', '.join(missing)}"
            )
        if len(sections["Apple"]) > 4000:
            raise ValueError(f"{release_tag} Apple notes exceed 4000 characters")


def asset_reference(name: str, asset_base_url: str) -> str:
    encoded_name = quote(name, safe="")
    if asset_base_url:
        return f"[{name}]({asset_base_url.rstrip('/')}/{encoded_name})"
    return f"[{name}](assets/{encoded_name})"


def first_match(assets: Sequence[str], pattern: str) -> str | None:
    compiled = re.compile(pattern)
    return next((name for name in assets if compiled.fullmatch(name)), None)


def append_download_group(
    lines: list[str],
    heading: str,
    choices: Sequence[tuple[str, str]],
    assets: Sequence[str],
    used: set[str],
    asset_base_url: str,
) -> None:
    entries: list[str] = []
    for label, pattern in choices:
        name = first_match(assets, pattern)
        if name is not None:
            used.add(name)
            entries.append(f"- {label}: {asset_reference(name, asset_base_url)}")
    if entries:
        lines.extend(["", heading, "", *entries])


def render_github(
    tag: str,
    commit: str,
    changes: str,
    assets: Iterable[str],
    asset_base_url: str,
) -> str:
    escaped_tag = re.escape(tag)
    sorted_assets = sorted(set(assets))
    used: set[str] = set()
    everyday = (
        ("Iris Chat for macOS (Apple Silicon)", rf"iris-chat-{escaped_tag}-macos-arm64\.dmg"),
        ("Iris Chat for Windows", rf"iris-chat-{escaped_tag}-windows-x64-setup\.exe"),
        ("Iris Chat for Android", rf"iris-chat-{escaped_tag}-android-arm64\.apk"),
        ("Iris Chat for Debian/Ubuntu", rf"iris-chat-{escaped_tag}-linux-x64\.deb"),
    )
    cli = (
        ("macOS Apple Silicon CLI", rf"iris-{escaped_tag}-aarch64-apple-darwin\.tar\.gz"),
        ("macOS Intel CLI", rf"iris-{escaped_tag}-x86_64-apple-darwin\.tar\.gz"),
        ("Linux x64 CLI", rf"iris-{escaped_tag}-x86_64-unknown-linux-gnu\.tar\.gz"),
    )

    lines = [f"# Iris Chat {tag}", "", "## Downloads"]
    append_download_group(
        lines,
        "### Most People Will Want",
        everyday,
        sorted_assets,
        used,
        asset_base_url,
    )
    append_download_group(
        lines,
        "### Command Line",
        cli,
        sorted_assets,
        used,
        asset_base_url,
    )
    other = [
        f"- {asset_reference(name, asset_base_url)}"
        for name in sorted_assets
        if name not in used
    ]
    if other:
        lines.extend(["", "### Other Files", "", *other])

    lines.extend(
        [
            "",
            "## Changes",
            "",
            changes,
            "",
            "## Verification",
            "",
            f"- Built from commit `{commit}` for release `{tag}`.",
            "- GitHub artifact attestations record build provenance for every release file.",
        ]
    )
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--notes", type=Path, default=Path("RELEASE_NOTES.md"))
    parser.add_argument("--tag")
    parser.add_argument(
        "--channel", choices=("validate", "github", "apple", "zapstore"), required=True
    )
    parser.add_argument("--commit")
    parser.add_argument("--asset-dir", type=Path)
    parser.add_argument("--asset-base-url", default="")
    parser.add_argument("--out", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.tag:
        require_tag(args.tag)

    releases = parse_notes(args.notes.read_text(encoding="utf-8"))
    validate_releases(releases, args.tag)
    if args.channel == "validate":
        return 0
    if not args.tag:
        raise ValueError("--tag is required when rendering notes")

    section_name = {
        "github": "GitHub",
        "apple": "Apple",
        "zapstore": "Zapstore",
    }[args.channel]
    body = releases[args.tag][section_name]
    if args.channel == "github":
        if not args.commit or args.asset_dir is None:
            raise ValueError("--commit and --asset-dir are required for GitHub notes")
        if not args.asset_dir.is_dir():
            raise ValueError(f"asset directory does not exist: {args.asset_dir}")
        assets = [path.name for path in args.asset_dir.iterdir() if path.is_file()]
        rendered = render_github(
            args.tag, args.commit, body, assets, args.asset_base_url
        )
    else:
        rendered = body + "\n"

    if args.out:
        args.out.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
