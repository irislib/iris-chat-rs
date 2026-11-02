#!/usr/bin/env python3
"""Validate source-bound shared-discovery CPU/wire evidence from real libtest."""
import hashlib
import json
import math
from pathlib import Path
import re
import sys

SOURCES = {
    "fixture_source_sha256": "core/src/core/tests/synthetic_history_scale.rs",
    "support_source_sha256": "core/src/core/tests/populated_discovery_idle_support.rs",
    "test_source_sha256": "core/src/core/tests/populated_discovery_idle.rs",
    "discovery_source_sha256": "core/src/core/protocol/public_discovery.rs",
}


def equivalent(a, b):
    if isinstance(a, float) and isinstance(b, float):
        return math.isfinite(a) and math.isfinite(b) and abs(a-b) <= max(math.ulp(a), math.ulp(b))
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(equivalent(a[key], b[key]) for key in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(equivalent(x, y) for x, y in zip(a, b))
    return a == b


def integer(value, expected=None):
    assert type(value) is int and value >= 0
    if expected is not None:
        assert value == expected


def finite(value, minimum, maximum=math.inf):
    assert type(value) in (float, int) and math.isfinite(value) and minimum <= value <= maximum


def validate(receipt_path, source, relay_count, run_log=None):
    receipt = json.loads(receipt_path.read_text())
    raw_log = receipt_path.with_suffix(".log").read_bytes()
    assert receipt["schema"] == "iris-populated-discovery-idle/v2"
    assert receipt["passed"] is True and receipt["timed_out"] is False
    integer(receipt["exit_code"], 0)
    integer(receipt["log_bytes"], len(raw_log))
    assert receipt["log_sha256"] == hashlib.sha256(raw_log).hexdigest()
    assert len(receipt["measurements"]) == 1
    result = receipt["measurements"][0]
    integer(result["database_bytes"])
    integer(result["message_rows"])
    assert result["database_bytes"] >= 100 * 1024 * 1024 and result["message_rows"] >= 20800
    for key, expected in (("contacts", 96), ("peer_devices", 288), ("own_devices", 3), ("groups", 8),
                          ("discovery_subjects", 384), ("mesh_discovery_subjects", 96),
                          ("total_discovery_subjects", 480), ("relay_count", relay_count),
                          ("anonymous_connections", relay_count)):
        integer(result[key], expected)
    integer(result["revoked_stale_records"])
    assert result["revoked_stale_records"] >= 96
    assert result["targeted_fresh_proof_delivered"] is True and result["privacy_wire_passed"] is True
    if relay_count == 5:
        assert result["debug_assertions"] is False
    for key, path in SOURCES.items():
        assert result[key] == hashlib.sha256((source / path).read_bytes()).hexdigest(), "Stale discovery source"
    readiness = result["relay_readiness"]
    assert len(readiness) == relay_count
    assert sorted(row["relay_index"] for row in readiness) == list(range(relay_count))
    for row in readiness:
        integer(row["relay_index"])
        integer(row["subjects"], 480)
        integer(row["connections"], 1)
        integer(row["anonymous_authentications"], 1)
    samples = result["quiet_samples"]
    assert len(samples) == 2
    for sample in samples:
        finite(sample["seconds"], 125)
        finite(sample["cpu_percent_one_core"], 0, 5)
        for key in ("application_wire_bytes", "connections", "app_row_writes", "protocol_writes"):
            integer(sample[key], 0)
        integer(sample["evicted_discovery_replays"])
        integer(sample["core_messages_processed"])
        assert sample["core_messages_processed"] >= sample["evicted_discovery_replays"] >= 192
        relays = sample["relay_samples"]
        assert len(relays) == relay_count
        assert sorted(row["relay_index"] for row in relays) == list(range(relay_count))
        for row in relays:
            for key, expected in (("subjects", 480), ("active_connections", 1), ("application_wire_bytes", 0), ("connections", 0)):
                integer(row[key], expected)
            integer(row["relay_index"])
    finite(result["settle_seconds"], 0)
    finite(receipt["seconds"], result["settle_seconds"] + sum(row["seconds"] for row in samples))
    text = raw_log.decode()
    name = "populated_account_five_relay_discovery_stays_quiet_and_targeted_demand_progresses" if relay_count == 5 else "populated_account_discovery_stays_quiet_and_targeted_demand_progresses"
    assert "test core::tests::populated_discovery_idle::" + name + " ... " in text
    assert re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", text) == [("ok", "1", "0", "0")]
    for marker, expected in (("POPULATED_DISCOVERY_IDLE_RESULT=", [result]), ("POPULATED_DISCOVERY_IDLE_SAMPLE=", samples)):
        actual = [json.loads(line.split(marker, 1)[1]) for line in text.splitlines() if marker in line]
        assert equivalent(actual, expected), "Companion log disagrees with measurements"
    if run_log is not None:
        outer = [json.loads(line.split("POPULATED_DISCOVERY_IDLE_RECEIPT=", 1)[1]) for line in run_log.read_text().splitlines() if "POPULATED_DISCOVERY_IDLE_RECEIPT=" in line]
        assert len(outer) == 1 and equivalent(outer[0], receipt), "Invocation did not emit this receipt exactly once"


if __name__ == "__main__":
    validate(Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3]), Path(sys.argv[4]) if len(sys.argv) > 4 else None)
    print("Five-relay populated discovery CPU/wire companion evidence verified" if sys.argv[3] == "5" else "Populated discovery CPU/wire companion evidence verified")
