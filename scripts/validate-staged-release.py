#!/usr/bin/env python3

"""Validate a staged release before resuming publication."""

import json
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise ValueError(message)


def main() -> int:
    if len(sys.argv) != 4:
        fail("usage: validate-staged-release.py <stage-dir> <tag> <commit>")

    stage = Path(sys.argv[1]).resolve()
    expected_tag, expected_commit = sys.argv[2:]
    manifest_path = stage / "release.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    if manifest.get("tag") != expected_tag:
        fail(f"staged tag does not match {expected_tag}")
    if manifest.get("commit") != expected_commit:
        fail(f"staged commit does not match {expected_commit}")

    assets_dir = (stage / "assets").resolve()
    entries = manifest.get("assets")
    if not isinstance(entries, list) or not entries:
        fail("staged release has no assets")

    paths = []
    for entry in entries:
        name = entry.get("name") if isinstance(entry, dict) else None
        relative = entry.get("path") if isinstance(entry, dict) else None
        size = entry.get("size") if isinstance(entry, dict) else None
        if not isinstance(name, str) or relative != f"assets/{name}":
            fail("invalid staged asset path")
        is_cli = name.startswith(("iris-aarch64-", "iris-x86_64-"))
        if "executable" in entry and not is_cli:
            fail(f"non-CLI asset has executable metadata: {name}")
        path = (stage / relative).resolve()
        if path.parent != assets_dir or not path.is_file():
            fail(f"staged asset is missing or has an unsafe path: {name}")
        if path.stat().st_size != size:
            fail(f"staged asset size does not match manifest: {name}")
        paths.append(path)

    if len(paths) != len(set(paths)):
        fail("staged release contains duplicate asset paths")
    actual = {path.resolve() for path in assets_dir.iterdir() if path.is_file()}
    if set(paths) != actual:
        fail("staged assets do not exactly match release.json")

    print(*(str(path) for path in paths), sep="\n")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
