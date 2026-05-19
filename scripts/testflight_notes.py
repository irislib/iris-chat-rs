#!/usr/bin/env python3

from __future__ import annotations

from pathlib import Path


DEFAULT_WHATS_NEW = (
    "Please test account creation, restoring with a secret key, direct and group chats, "
    "linked devices, notifications, attachments, and Nearby messaging."
)
MAX_WHATS_NEW_LENGTH = 4000


def what_to_test_from_release_notes(
    version_name: str,
    repo_root: str | Path,
    release_notes_file: str | Path = "ZAPSTORE_RELEASE_NOTES.md",
) -> str:
    root = Path(repo_root).resolve()
    notes_path = Path(release_notes_file)
    if not notes_path.is_absolute():
        notes_path = root / notes_path

    try:
        lines = notes_path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return ""

    headings = {f"Iris Chat {version_name}", version_name, f"v{version_name}"}
    collecting = False
    collected: list[str] = []
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("# "):
            if collecting:
                break
            collecting = stripped[2:].strip() in headings
            continue
        if collecting and stripped:
            collected.append(stripped)

    whats_new = "\n".join(collected).strip()
    if len(whats_new) > MAX_WHATS_NEW_LENGTH:
        whats_new = whats_new[: MAX_WHATS_NEW_LENGTH - 3].rstrip() + "..."
    return whats_new


def resolved_what_to_test(
    version_name: str,
    repo_root: str | Path,
    *,
    explicit: str = "",
    release_notes_file: str | Path = "ZAPSTORE_RELEASE_NOTES.md",
) -> str:
    explicit = explicit.strip()
    if explicit:
        return explicit
    return (
        what_to_test_from_release_notes(version_name, repo_root, release_notes_file)
        or DEFAULT_WHATS_NEW
    )
