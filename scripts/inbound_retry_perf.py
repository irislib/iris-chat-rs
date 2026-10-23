#!/usr/bin/env python3
"""Optional process-scoped attribution after an authoritative CPU failure."""
from pathlib import Path
import re
import shutil
import sys


def record_command(perf, output, workload, maximum="31M", frequency_hz=49):
    # Reserve 1 MiB below the 32 MiB artifact ceiling for headers/metadata.
    # Do not populate perf's build-id cache with a second executable copy.
    return [perf, "record", "-e", "cpu-clock:u", "-F", str(frequency_hz), "--call-graph",
            "dwarf,4096", "--clockid", "mono", "--max-size", maximum,
            "--no-buildid-cache", "-o", str(output), "--", *workload]


def sample_failure(command, root, out, deadline, target, bounded, sha, *,
                   frequency_hz=49, timeout_seconds=300, uninline_report=False,
                   report_max_bytes=None):
    assert frequency_hz in (49, 199) and timeout_seconds in (300, 420)
    assert report_max_bytes in (None, 8 * 1024**2)
    receipt = {"status": "skipped", "diagnostic_only": True,
               "cpu_acceptance": False, "binary_sha256": sha(Path(command[0])),
               "frequency_hz": frequency_hz, "dwarf_stack_bytes": 4096,
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
        probe_run, _ = bounded(record_command(perf, probe_path, probe, "1M", frequency_hz), root, out, "perf-permission-probe", 15, deadline, target)
        receipt["permission_probe"] = probe_run
        if probe_run["exit_code"] != 0 or not probe_path.is_file() or probe_path.stat().st_size == 0:
            receipt["reason"] = "Own-process software-event/DWARF permission probe failed; see perf-permission-probe.log"
            return receipt
        receipt["permission_probe_sha256"] = sha(probe_path)
        assert probe_path.stat().st_size <= 1024**2, "Permission probe exceeds 1 MiB"
        report_flags = ["--stdio", "--header", "--stats", "--no-children",
                        "--percent-limit", "0.5", "-g", "graph,0.5,caller"]
        if uninline_report:
            report_flags.append("--no-inline")

        def report_command(path):
            args = [perf, "report", *report_flags, "-i", str(path)]
            if report_max_bytes is not None:
                # Bound only this report process's output file, not other logs.
                args = [sys.executable, "-c",
                        "import os,resource,sys; n=int(sys.argv[1]); "
                        "resource.setrlimit(resource.RLIMIT_FSIZE,(n,n)); "
                        "os.execv(sys.argv[2],sys.argv[2:])", str(report_max_bytes), *args]
            return args

        if uninline_report:
            report_probe, _ = bounded(report_command(probe_path), root, out,
                                      "perf-report-probe", 15, deadline, target)
            receipt["report_probe"] = report_probe
            if report_probe["exit_code"] != 0:
                receipt["reason"] = "Uninlined DWARF report probe failed; workload not executed"
                return receipt
        assert sha(Path(command[0])) == receipt["binary_sha256"], "Executable changed before sampling"
        data = out / "whole-retry.perf.data"
        run, _ = bounded(record_command(perf, data, command, frequency_hz=frequency_hz), root, out, "perf-sampled-test", timeout_seconds, deadline, target)
        receipt.update(status="sampled", run=run, test_log="perf-sampled-test.log")
        assert sha(Path(command[0])) == receipt["binary_sha256"], "Executable changed during sampling"
        assert data.is_file() and 0 < data.stat().st_size <= receipt["maximum_data_bytes"], "Perf data missing or exceeds the 32 MiB ceiling"
        receipt.update(data_bytes=data.stat().st_size, data_sha256=sha(data))
        # Decode now against the same executable; retain caller stacks, sample
        # counts/loss statistics and raw data without copying the target cache.
        report, _ = bounded(report_command(data), root, out, "perf-report", 60, deadline, target)
        receipt["report"] = report
        receipt["report_max_bytes"] = report_max_bytes
        receipt["uninline_report"] = uninline_report
        receipt["sample_and_loss_statistics_log"] = "perf-report.log"
        if report["exit_code"] != 0:
            receipt["report_error"] = "Perf report failed; raw capture and log retained"
        if report_max_bytes is not None and (out / "perf-report.log").stat().st_size >= report_max_bytes:
            receipt.update(status="incomplete", reason="Perf report reached its output ceiling; raw capture retained")
    except Exception as error:
        receipt.update(status="incomplete", reason=f"{type(error).__name__}: {error}")
    return receipt
