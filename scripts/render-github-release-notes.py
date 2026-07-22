#!/usr/bin/env python3

"""Render deterministic GitHub release notes from the app changelog."""

import argparse
import re
import sys
from pathlib import Path
from typing import List


TAG_RE = re.compile(r"^v(?P<version>\d+(?:\.\d+){2,3}(?:-[0-9A-Za-z.-]+)?)$")


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


def render(tag: str, commit: str, changelog: str, assets: List[str]) -> str:
    match = TAG_RE.fullmatch(tag)
    if not match:
        raise ValueError(f"unsupported release tag: {tag}")

    version = match.group("version")
    changes = release_section(changelog, version)
    asset_lines = "\n".join(f"- `{name}`" for name in sorted(assets))
    return (
        f"# Iris Chat {tag}\n\n"
        f"{changes}\n\n"
        "## Downloads\n\n"
        f"{asset_lines}\n\n"
        "## Verification\n\n"
        "- GitHub artifact attestations record build provenance for the release files.\n"
        f"- Built from commit `{commit}`.\n"
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--asset-dir", required=True, type=Path)
    parser.add_argument("--changelog", required=True, type=Path)
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
