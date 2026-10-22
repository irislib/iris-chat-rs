#!/usr/bin/env python3
"""Optional process-scoped attribution after an authoritative CPU failure."""
from pathlib import Path
import re
import shutil
import sys


def record_command(perf, output, workload, maximum="31M"):
    # Reserve 1 MiB below the 32 MiB artifact ceiling for headers/metadata.
    # Do not populate perf's build-id cache with a second executable copy.
    return [perf, "record", "-e", "cpu-clock:u", "-F", "49", "--call-graph",
            "dwarf,4096", "--clockid", "mono", "--max-size", maximum,
            "--no-buildid-cache", "-o", str(output), "--", *workload]


def sample_failure(command, root, out, deadline, target, bounded, sha):
    receipt = {"status": "skipped", "diagnostic_only": True,
               "cpu_acceptance": False, "binary_sha256": sha(Path(command[0])),
               "frequency_hz": 49, "dwarf_stack_bytes": 4096,
               "maximum_data_bytes": 32 * 1024**2,
               "coverage": "Entire sampled test process, including setup and restart; not isolated to its measured minute.",
               "no_system_settings_changed": True, "no_tools_installed": True}
    perf = shutil.which("perf")
    if perf is None:
        receipt["reason"] = "perf is not installed"
        return receipt
    receipt["perf_path"] = perf
    setting = Path("/proc/sys/kernel/perf_event_paranoid")
    receipt["perf_event_paranoid"] = setting.read_text().strip() if setting.exists() else "unavailable"
    try:
        version, text = bounded([perf, "version", "--build-options"], root, out, "perf-build-options", 15, deadline, target)
        if version["exit_code"] != 0:
            receipt["reason"] = "Installed perf cannot report build options; see perf-build-options.log"
            return receipt
        if not re.search(r"(?:libunwind|libdw-dwarf-unwind|dwarf-post-unwind)\s*:\s*\[\s*on\s*\]", text):
            receipt["reason"] = "Installed perf does not advertise DWARF post-unwind support"
            return receipt
        _, help_text = bounded([perf, "record", "-h"], root, out, "perf-record-help", 15, deadline, target)
        required = ("--call-graph", "--clockid", "--max-size", "--no-buildid-cache")
        if not all(flag in help_text for flag in required):
            receipt["reason"] = "Installed perf record lacks a required bounded-recording option"
            return receipt
        probe_path = out / "perf-permission-probe.data"
        probe = [sys.executable, "-c", "import time\nend=time.monotonic()+0.25\nwhile time.monotonic()<end: pass"]
        probe_run, _ = bounded(record_command(perf, probe_path, probe, "1M"), root, out, "perf-permission-probe", 15, deadline, target)
        receipt["permission_probe"] = probe_run
        if probe_run["exit_code"] != 0 or not probe_path.is_file() or probe_path.stat().st_size == 0:
            receipt["reason"] = "Own-process software-event/DWARF permission probe failed; see perf-permission-probe.log"
            return receipt
        receipt["permission_probe_sha256"] = sha(probe_path)
        assert sha(Path(command[0])) == receipt["binary_sha256"], "Executable changed before sampling"
        data = out / "whole-retry.perf.data"
        run, _ = bounded(record_command(perf, data, command), root, out, "perf-sampled-test", 300, deadline, target)
        receipt.update(status="sampled", run=run, test_log="perf-sampled-test.log")
        assert sha(Path(command[0])) == receipt["binary_sha256"], "Executable changed during sampling"
        assert data.is_file() and 0 < data.stat().st_size <= receipt["maximum_data_bytes"], "Perf data missing or exceeds the 32 MiB ceiling"
        receipt.update(data_bytes=data.stat().st_size, data_sha256=sha(data))
        # Decode now against the same executable; retain caller stacks, sample
        # counts/loss statistics and raw data without copying the target cache.
        report, _ = bounded([perf, "report", "--stdio", "--header", "--stats", "--no-children",
                             "--percent-limit", "0.5", "-g", "graph,0.5,caller", "-i", str(data)],
                            root, out, "perf-report", 60, deadline, target)
        receipt["report"] = report
        receipt["sample_and_loss_statistics_log"] = "perf-report.log"
        if report["exit_code"] != 0:
            receipt["report_error"] = "Perf report failed; raw capture and log retained"
    except Exception as error:
        receipt.update(status="incomplete", reason=f"{type(error).__name__}: {error}")
    return receipt
