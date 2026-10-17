#!/usr/bin/env python3
"""Temporary hosted-only diagnostic: one build, unit check, three fixed UI runs.

No retries or test policy changes. Each run uses the unchanged test's fresh
isolated account. Retain every xcresult, exit status, log and anonymous trace.
"""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
TEST = "IrisChatUITests/ChatPaginationUITests/testMixedHeightHistoryPrependPreservesVisibleMessage"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2); stream.write("\n")


def trace_inventory(simulator):
    result = subprocess.run(["xcrun", "simctl", "get_app_container", simulator,
                             "fi.siriusbusiness.irischat", "data"], capture_output=True, text=True, timeout=20)
    if result.returncode:
        return {}
    directory = Path(result.stdout.strip()) / "tmp"
    paths = list(directory.glob("iris-pagination-timeline-pages-*.log"))
    if len(paths) > 16:
        raise ValueError("Unexpected diagnostic trace count; never delete fixture evidence")
    return {path.name: path for path in paths}


def retain_new_traces(before, after, destination):
    records = []
    for name, path in after.items():
        if name in before:
            continue
        if not re.fullmatch(r"iris-pagination-timeline-pages-[0-9A-Fa-f-]{36}\.log", name):
            raise ValueError("Unexpected trace name")
        if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 2 * 1024 * 1024:
            raise ValueError("Unexpected trace type or size")
        data = path.read_bytes()
        target = destination / name
        with target.open("xb") as output: output.write(data)
        records.append({"file": name, "bytes": len(data), "sha256": digest(target),
                        "summary": data.decode().splitlines()[0]})
    return records


def execute(command, directory, timeout):
    directory.mkdir()
    start = time.time()
    with (directory / "xcodebuild.log").open("x") as output:
        process = subprocess.Popen(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        timed_out = False
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            import signal
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try: code = process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL); code = process.wait(timeout=10)
    result = {"command": command, "exitCode": code, "timedOut": timed_out,
              "startedAtUnix": start, "finishedAtUnix": time.time(),
              "logSHA256": digest(directory / "xcodebuild.log")}
    write(directory / "command.json", result)
    return result


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Diagnostic execution is hosted-only; no local simulator or build action")
    evidence = ROOT / "work/clipboard/pagination-diagnostic"
    evidence.mkdir(parents=True, exist_ok=False)
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    destination = subprocess.check_output([str(ROOT / "scripts/ios-simulator-destination")],
                                          cwd=ROOT, text=True, timeout=180).strip()
    match = re.fullmatch(r"platform=iOS Simulator,id=([0-9A-Fa-f-]{36})", destination)
    if not match:
        raise ValueError("Exact simulator destination required")
    simulator = match.group(1)
    command = ["xcodebuild", "-project", "ios/IrisChat.xcodeproj", "-scheme", "IrisChat",
               "-destination", destination, "-parallel-testing-enabled", "NO"]
    build = execute(command + ["build-for-testing"], evidence / "build", 1800)
    if build["exitCode"] != 0:
        write(evidence / "summary.json", {"source": source, "build": build, "testsExecuted": False})
        return 1
    unit_dir = evidence / "recorder-unit"
    unit = execute(command + ["-only-testing:IrisChatTests/ChatTimelineHistoryDiagnosticTests",
                   "-resultBundlePath", str(unit_dir / "unit.xcresult"), "test-without-building"], unit_dir, 600)
    runs = []
    # All three are scheduled independently, regardless of previous outcome.
    # No retry-on-failure setting and no threshold/gesture/deadline override.
    for iteration in range(1, 4):
        directory = evidence / ("iteration-" + str(iteration))
        collection_error = None
        try:
            before = trace_inventory(simulator)
        except (ValueError, OSError, subprocess.SubprocessError) as error:
            before = {}
            collection_error = str(error)
        result = execute(command + ["-only-testing:" + TEST, "-resultBundlePath",
                         str(directory / "test.xcresult"), "test-without-building"], directory, 600)
        try:
            result["traces"] = retain_new_traces(before, trace_inventory(simulator), directory)
            result["completeTrace"] = collection_error is None and len(result["traces"]) == 1 and all(
                "truncated=false" in trace["summary"] and "incomplete=false" in trace["summary"]
                for trace in result["traces"])
        except (ValueError, OSError, subprocess.SubprocessError) as error:
            collection_error = str(error)
            result["completeTrace"] = False
        if collection_error is not None:
            result["collectionError"] = collection_error
        result["iteration"] = iteration
        write(directory / "result.json", result)
        runs.append(result)
    write(evidence / "summary.json", {"source": source, "test": TEST, "fixedExecutions": 3,
          "retryOnFailure": False, "unit": unit, "runs": runs, "diagnosticOnly": True})
    return int(unit["exitCode"] != 0 or any(run["exitCode"] != 0 or not run["completeTrace"] for run in runs))


if __name__ == "__main__":
    raise SystemExit(main())
