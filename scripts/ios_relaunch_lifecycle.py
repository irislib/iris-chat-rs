#!/usr/bin/env python3
"""Evaluate private, append-only captures of one ordinary iOS relaunch.

No device operations occur here. Device log timestamps order device events only;
host monotonic sample times measure observed suspension and resume intervals.
This is new qualification code, not a reconstruction of historical helpers.
The caller separately proves the fresh launch and the same-audit-token resume.
"""
from __future__ import annotations

from datetime import datetime
import hashlib
import math
import re


MAX_CAPTURE_BYTES = 8 * 1024 * 1024
MAX_SAMPLE_GAP_SECONDS = 8
MAX_OBSERVATION_SECONDS = 420
ACTIVE = "running-active-Visible"
SUSPENDED = "running-suspended-NotVisible"
SAMPLE_KEYS = {"observed_monotonic", "pid", "executable", "startup_bytes", "startup_sha256",
               "lifecycle_bytes", "lifecycle_sha256", "loggers_alive", "stderr"}
STAMP = re.compile(r"^(?P<stamp>[A-Z][a-z]{2}\s+\d{1,2} \d{2}:\d{2}:\d{2}\.\d{6}) ")
LOSS = re.compile(r"\[(?:disconnected|error)(?::|\])|\b(?:dropped|lost) \d+ (?:log )?messages\b", re.I)


class LifecycleError(ValueError):
    """Incomplete or contradictory lifecycle evidence; never a runtime pass."""


def require(condition, message):
    if not condition:
        raise LifecycleError(message)


def _number(value):
    require(type(value) in (int, float) and math.isfinite(value), "Invalid monotonic timestamp")
    return value


def _stamp(line):
    match = STAMP.match(line)
    require(match is not None, "Missing precise device timestamp")
    try:
        # A fixed leap year retains calendar/day ordering. It is not a UTC epoch
        # or a host timestamp. A year rollover is rejected as a backwards clock.
        value = datetime.strptime("2000 " + match["stamp"], "%Y %b %d %H:%M:%S.%f")
    except ValueError as error:
        raise LifecycleError("Invalid device timestamp") from error
    return (value - datetime(2000, 1, 1)).total_seconds()


def _capture(text, device):
    require(isinstance(text, str), "Capture must be text")
    try:
        data = text.encode("utf-8", errors="strict")
    except UnicodeEncodeError as error:
        raise LifecycleError("Invalid capture encoding") from error
    require(0 < len(data) <= MAX_CAPTURE_BYTES and data.endswith(b"\n"), "Incomplete or oversized capture")
    require("\x00" not in text and "\ufffd" not in text and "\r" not in text, "Invalid capture encoding")
    lines = text.splitlines()
    require(lines[0] == "[connected:" + device + "]", "Capture is not connected to the expected device")
    require(sum(line.startswith("[connected:") for line in lines) == 1, "Capture reconnected")
    require(not LOSS.search(text), "Capture reports lost messages or a disconnected stream")
    return data


def capture_prefixes(*, startup_text, lifecycle_text, device):
    """Bind ready, complete snapshots immediately before the one launch call."""
    require(isinstance(device, str) and device and "\n" not in device, "Missing capture device")
    result = {}
    for name, text in (("startup", startup_text), ("lifecycle", lifecycle_text)):
        data = _capture(text, device)
        result[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    return result


def _prefix(data, size, digest, *, complete=False):
    require(type(size) is int and 0 < size <= len(data), "Invalid capture byte boundary")
    require(isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest), "Invalid capture digest")
    require(hashlib.sha256(data[:size]).hexdigest() == digest, "Capture prefix was changed or truncated")
    if complete:
        require(data[:size].endswith(b"\n"), "Initial capture boundary splits a record")
    try:
        prefix = data[:size].decode("utf-8")
    except UnicodeDecodeError as error:
        raise LifecycleError("Capture boundary splits a UTF-8 character") from error
    return prefix[:prefix.rfind("\n") + 1]


def _records(data, initial, parser):
    """Return parsed records with end-byte offsets, preserving complete order."""
    initial_stamps = [_stamp(line) for line in data[:initial].decode("utf-8").splitlines() if STAMP.match(line)]
    records, offset, previous = [], initial, initial_stamps[-1] if initial_stamps else None
    for raw in data[initial:].splitlines(keepends=True):
        offset += len(raw)
        line = raw.decode("utf-8").rstrip("\n")
        # All timestamped lines must remain ordered, not just accepted markers.
        if STAMP.match(line):
            stamp = _stamp(line)
            require(previous is None or stamp >= previous, "Device log timestamps moved backwards")
            previous = stamp
        item = parser(line)
        if item is not None:
            records.append({**item, "end_byte": offset, "stamp": _stamp(line)})
    return records


def evaluate_early_background(*, startup_text, lifecycle_text, bundle_id, process_name,
                              pid, executable, device, initial_prefixes, samples, old_pid=None):
    """Require real native-restoration overlap, 135s suspended and 15s resumed.

    Initial prefixes are captured immediately before launch. Every later sample
    binds byte lengths/hashes of both growing logs, live-loggers/empty-stderr
    observations and the actual CoreDevice process inventory (PID/executable).
    Per-poll audit tokens are deliberately not invented: CoreDevice inventory
    does not supply them. The launcher must verify the real resume audit token.
    Returned evidence is private and is not a complete v3 release acceptance.
    """
    require(type(pid) is int and pid > 0, "Invalid new process")
    require(old_pid is None or (type(old_pid) is int and old_pid > 0 and old_pid != pid), "Invalid old process")
    for value in (bundle_id, process_name, executable, device):
        require(isinstance(value, str) and value and "\n" not in value, "Missing capture identity")
    require(isinstance(initial_prefixes, dict) and set(initial_prefixes) == {"startup", "lifecycle"}, "Missing initial prefixes")
    captures = {"startup": _capture(startup_text, device), "lifecycle": _capture(lifecycle_text, device)}
    initial = {}
    for name, data in captures.items():
        binding = initial_prefixes[name]
        require(isinstance(binding, dict) and set(binding) == {"bytes", "sha256"}, "Invalid initial prefix binding")
        prefix = _prefix(data, binding["bytes"], binding["sha256"], complete=True)
        require(prefix.startswith("[connected:" + device + "]\n"), "Logger was not ready before launch")
        initial[name] = binding["bytes"]

    marker = re.compile(r"(?:^| )" + re.escape(process_name) + r"(?:\([^)]*\))?\[(\d+)\] <[^>]+>: iris\.startup "
                        r"phase=(\w+) restore_pending_result=(true|false) "
                        r"native_restoring_observed=(true|false) app_backgrounded=(true|false)$")
    def parse_startup(line):
        if "iris.startup " not in line:
            return None
        match = marker.search(line)
        require(match is not None, "Malformed startup marker")
        require(int(match[1]) == pid, "Startup marker is not from the newly launched process")
        return {"phase": match[2], "pending": match[3] == "true", "native": match[4] == "true",
                "backgrounded": match[5] == "true"}

    state_pattern = re.compile(r"SpringBoard(?:\(RunningBoardServices\))?\[\d+\] <[^>]+>: "
                               r"Received state update for (\d+) \(app<" + re.escape(bundle_id) +
                               r"(?:\([^>]*\))?>, ([A-Za-z-]+)(?:[ ,].*)?$")
    def parse_state(line):
        if "Received state update for " not in line or bundle_id not in line:
            return None
        match = state_pattern.search(line)
        require(match is not None, "Unrecognized target lifecycle record")
        return {"pid": int(match[1]), "state": match[2]}

    startup = _records(captures["startup"], initial["startup"], parse_startup)
    states = _records(captures["lifecycle"], initial["lifecycle"], parse_state)
    require([row["phase"] for row in startup] ==
            ["restore_enqueued", "backgrounded", "restore_result_observed"],
            "Need exactly one ordered enqueue, background entry and successful restoration result")
    enqueue, entry, result = startup
    require(enqueue["pending"] and not enqueue["backgrounded"], "Restoration did not start in the foreground")
    require(entry["pending"] and entry["native"] and entry["backgrounded"], "No actual native restoration at background entry")
    require(not result["pending"], "Restoration is still pending")
    require(enqueue["stamp"] < entry["stamp"] < result["stamp"], "Restoration timestamps are not ordered")

    target_states, activated = [], False
    for row in states:
        if row["pid"] != pid:
            require(row["pid"] == old_pid and not activated and row["state"] in {"not-running", "terminated", "exited"},
                    "Another instance changed target lifecycle state")
            continue
        require(row["state"] in {ACTIVE, SUSPENDED, "running-active-NotVisible"}, "Unexpected new-process lifecycle state")
        if not activated:
            require(row["state"] == ACTIVE, "New process did not enter the foreground")
            activated = True
        target_states.append(row)
    require(target_states, "Missing OS foreground evidence")
    require(target_states[0]["stamp"] <= entry["stamp"], "Foreground evidence follows background entry")
    suspended = [row for row in target_states if row["state"] == SUSPENDED]
    require(suspended and entry["stamp"] <= suspended[0]["stamp"], "No OS suspension after restoration overlap")
    resumed = [row for row in target_states
               if row["state"] == ACTIVE and row["stamp"] > suspended[0]["stamp"]]
    require(resumed and resumed[0]["stamp"] - suspended[0]["stamp"] >= 135,
            "Device lifecycle records show less than 135 seconds suspended")

    require(isinstance(samples, list) and 2 <= len(samples) <= 512, "Missing or excessive process/stream observations")
    previous_time, previous_sizes = None, dict(initial)
    suspended_start = suspended_last = resumed_start = resumed_last = None
    suspended_seconds = None
    seen_state_count = 0
    stage = "foreground"
    for sample in samples:
        require(isinstance(sample, dict) and set(sample) == SAMPLE_KEYS, "Invalid process/stream sample")
        now = _number(sample["observed_monotonic"])
        require(previous_time is None or 0 < now - previous_time <= MAX_SAMPLE_GAP_SECONDS,
                "Process observation gap or backwards host clock")
        previous_time = now
        require(type(sample["pid"]) is int and sample["pid"] == pid and sample["executable"] == executable,
                "Process disappeared or was replaced")
        require(sample["loggers_alive"] is True and sample["stderr"] == "", "Logger stopped or reported an error")
        for name, data in captures.items():
            size = sample[name + "_bytes"]
            _prefix(data, size, sample[name + "_sha256"])
            require(size >= previous_sizes[name], "Capture prefix shrank")
            previous_sizes[name] = size
        visible_states = [row for row in target_states if row["end_byte"] <= sample["lifecycle_bytes"]]
        for row in visible_states[seen_state_count:]:
            state = row["state"]
            if stage == "foreground" and state == SUSPENDED:
                stage = "suspended"
                suspended_start = now
            elif stage == "suspended" and state != SUSPENDED:
                require(state == ACTIVE and suspended_last is not None and suspended_last - suspended_start >= 135,
                        "Suspension ended before 135 observed seconds")
                suspended_seconds = suspended_last - suspended_start
                stage, resumed_start = "resumed", now
            elif stage == "resumed":
                require(state == ACTIVE, "Resumed process left the foreground")
        seen_state_count = len(visible_states)
        if stage == "suspended":
            suspended_last = now
        elif stage == "resumed":
            resumed_last = now
    require(samples[-1]["observed_monotonic"] - samples[0]["observed_monotonic"] <= MAX_OBSERVATION_SECONDS,
            "Observation exceeded its fixed bound")
    for name, data in captures.items():
        require(previous_sizes[name] == len(data), "Final sample does not bind the complete capture")
    require(stage == "resumed" and resumed_last is not None and resumed_last - resumed_start >= 15,
            "Missing 15 observed seconds in the same resumed process")
    return {"restore_in_flight": True, "suspended_seconds": suspended_seconds,
            "resumed_seconds": resumed_last - resumed_start, "process_samples": len(samples),
            "suspended_started_monotonic": suspended_start, "suspended_last_monotonic": suspended_last,
            "resumed_started_monotonic": resumed_start, "resumed_last_monotonic": resumed_last,
            "pid": pid, "executable": executable, "process_replacements": 0, "unexpected_exits": 0,
            "startup_sha256": hashlib.sha256(captures["startup"]).hexdigest(),
            "lifecycle_sha256": hashlib.sha256(captures["lifecycle"]).hexdigest(),
            "stream_continuity": "No detected loss; silent OS log loss cannot be excluded",
            "runtime_acceptance": False}
