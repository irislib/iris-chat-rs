#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import json
import sqlite3
import sys
from pathlib import Path
from typing import Any


MESSAGE_EVENT_KIND = 1060


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Inspect AppCore pending relay publishes and optionally write selected "
            "event IDs to a local relay fault-injection drop file."
        )
    )
    parser.add_argument(
        "command",
        choices=("list", "write-drop-file"),
        help="List matching pending rows or write their event IDs to --drop-file.",
    )
    parser.add_argument(
        "--data-dir",
        required=True,
        type=Path,
        help="App data directory containing core.sqlite3, or a direct path to core.sqlite3.",
    )
    parser.add_argument("--target-owner-hex", help="Filter by target_owner_pubkey_hex.")
    parser.add_argument("--target-device-hex", help="Filter by target_device_id.")
    parser.add_argument("--event-id", action="append", help="Filter by exact event ID. Repeatable.")
    parser.add_argument("--label-contains", help="Filter by substring in pending label.")
    parser.add_argument(
        "--pairwise-only",
        action="store_true",
        help="Only include encrypted pairwise NDR message events: kind 1060 with header tag.",
    )
    parser.add_argument(
        "--group-sender-outer-only",
        action="store_true",
        help="Only include visible group sender-key outer events: kind 1060 without header tag.",
    )
    parser.add_argument("--limit", type=int, default=0, help="Maximum matching rows to output.")
    parser.add_argument(
        "--format",
        choices=("table", "json", "ids"),
        default="table",
        help="Output format for list. write-drop-file always prints written IDs.",
    )
    parser.add_argument(
        "--drop-file",
        type=Path,
        help="Destination file for write-drop-file. Use with IRIS_LOCAL_RELAY_DROP_EVENT_IDS_FILE.",
    )
    parser.add_argument(
        "--append",
        action="store_true",
        help="Append to --drop-file instead of replacing it.",
    )
    return parser.parse_args()


def db_path(data_dir: Path) -> Path:
    if data_dir.is_file():
        return data_dir
    return data_dir / "core.sqlite3"


def load_rows(path: Path) -> list[dict[str, Any]]:
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    try:
        rows = conn.execute(
            """
            SELECT event_id, owner_pubkey_hex, label, event_json, inner_event_id,
                   target_owner_pubkey_hex, target_device_id, message_id, chat_id,
                   created_at_secs, attempt_count, last_error
            FROM pending_relay_publishes
            ORDER BY created_at_secs ASC, event_id ASC
            """
        ).fetchall()
    finally:
        conn.close()
    return [dict(row) for row in rows]


def event_tags(event: dict[str, Any]) -> list[list[Any]]:
    tags = event.get("tags")
    if not isinstance(tags, list):
        return []
    return [tag for tag in tags if isinstance(tag, list)]


def has_tag(event: dict[str, Any], tag_name: str) -> bool:
    return any(tag and tag[0] == tag_name for tag in event_tags(event))


def looks_like_group_sender_outer(event: dict[str, Any]) -> bool:
    if int(event.get("kind") or 0) != MESSAGE_EVENT_KIND or has_tag(event, "header"):
        return False
    content = event.get("content")
    if not isinstance(content, str):
        return False
    try:
        decoded = base64.b64decode(content, validate=True)
    except Exception:
        return False
    return len(decoded) >= 8


def classify_event(event: dict[str, Any]) -> str:
    kind = int(event.get("kind") or 0)
    if kind == MESSAGE_EVENT_KIND and has_tag(event, "header"):
        return "pairwise-encrypted"
    if looks_like_group_sender_outer(event):
        return "group-sender-outer"
    return f"kind-{kind}"


def hydrate(row: dict[str, Any]) -> dict[str, Any]:
    try:
        event = json.loads(row["event_json"])
    except Exception as error:
        event = {"parse_error": str(error)}
    row = dict(row)
    row["event"] = event
    row["kind"] = event.get("kind")
    row["pubkey"] = event.get("pubkey")
    row["classification"] = classify_event(event) if isinstance(event, dict) else "invalid"
    row["has_header_tag"] = has_tag(event, "header") if isinstance(event, dict) else False
    return row


def normalized(value: str | None) -> str | None:
    return value.lower() if value else None


def matches(row: dict[str, Any], args: argparse.Namespace) -> bool:
    if args.target_owner_hex and normalized(row.get("target_owner_pubkey_hex")) != normalized(
        args.target_owner_hex
    ):
        return False
    if args.target_device_hex and normalized(row.get("target_device_id")) != normalized(
        args.target_device_hex
    ):
        return False
    if args.event_id and row["event_id"] not in set(args.event_id):
        return False
    if args.label_contains and args.label_contains not in (row.get("label") or ""):
        return False
    if args.pairwise_only and row["classification"] != "pairwise-encrypted":
        return False
    if args.group_sender_outer_only and row["classification"] != "group-sender-outer":
        return False
    return True


def matching_rows(args: argparse.Namespace) -> list[dict[str, Any]]:
    path = db_path(args.data_dir)
    if not path.exists():
        raise SystemExit(f"database not found: {path}")
    rows = [hydrate(row) for row in load_rows(path)]
    rows = [row for row in rows if matches(row, args)]
    if args.limit > 0:
        rows = rows[: args.limit]
    return rows


def short(value: Any, width: int = 12) -> str:
    if value is None:
        return "-"
    text = str(value)
    if len(text) <= width:
        return text
    return text[:width]


def print_table(rows: list[dict[str, Any]]) -> None:
    headers = [
        "created",
        "class",
        "event_id",
        "author",
        "target_owner",
        "target_device",
        "label",
    ]
    print("  ".join(f"{header:>18}" for header in headers))
    for row in rows:
        values = [
            row.get("created_at_secs"),
            row.get("classification"),
            short(row.get("event_id"), 18),
            short(row.get("pubkey"), 18),
            short(row.get("target_owner_pubkey_hex"), 18),
            short(row.get("target_device_id"), 18),
            row.get("label") or "-",
        ]
        print("  ".join(f"{str(value):>18}" for value in values))


def write_drop_file(rows: list[dict[str, Any]], path: Path, append: bool) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = "a" if append else "w"
    with path.open(mode, encoding="utf-8") as handle:
        for row in rows:
            handle.write(f"{row['event_id']}\n")


def main() -> int:
    args = parse_args()
    rows = matching_rows(args)
    if args.command == "write-drop-file":
        if args.drop_file is None:
            raise SystemExit("write-drop-file requires --drop-file")
        write_drop_file(rows, args.drop_file, args.append)
        for row in rows:
            print(row["event_id"])
        print(f"wrote {len(rows)} event id(s) to {args.drop_file}", file=sys.stderr)
        return 0 if rows else 1

    if args.format == "json":
        print(json.dumps(rows, indent=2, sort_keys=True))
    elif args.format == "ids":
        for row in rows:
            print(row["event_id"])
    else:
        print_table(rows)
    return 0 if rows else 1


if __name__ == "__main__":
    raise SystemExit(main())
