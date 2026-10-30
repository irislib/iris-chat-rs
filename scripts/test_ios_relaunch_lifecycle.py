#!/usr/bin/env python3
"""Synthetic lifecycle qualification only; no phones, identities, or builds."""
import copy
from datetime import datetime, timedelta
import hashlib
import unittest

import ios_relaunch_lifecycle as lifecycle


DEVICE = "synthetic-device"
BUNDLE = "example.synthetic.chat"
PID = 102
EXECUTABLE = "file:///synthetic/Chat.app/Chat"
HEADER = "[connected:" + DEVICE + "]\n"


def stamp(seconds):
    value = datetime(2000, 3, 10, 12) + timedelta(seconds=seconds)
    return value.strftime("%b %d %H:%M:%S.%f")


def startup(phase, seconds, *, pending=False, native=False, background=False, pid=PID):
    return (f"{stamp(seconds)} Chat[{pid}] <Notice>: iris.startup phase={phase} "
            f"restore_pending_result={str(pending).lower()} native_restoring_observed={str(native).lower()} "
            f"app_backgrounded={str(background).lower()}\n")


def state(value, seconds, *, pid=PID):
    return (f"{stamp(seconds)} SpringBoard(RunningBoardServices)[1] <Notice>: Received state update for "
            f"{pid} (app<{BUNDLE}(00000000-0000-0000-0000-000000000000)>, {value}\n")


def binding(text):
    data = text.encode()
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def bind_cutoff(value):
    """Bind synthetic shutdown metadata without altering any live observation."""
    last = value["samples"][-1]
    now = last["observed_monotonic"]
    value["measurement_cutoff"] = {
        "schema": "iris-live-capture-cutoff-v1", "intent": "finish-observation",
        "sample_index": len(value["samples"]) - 1, "samples_sha256": lifecycle.evidence_sha256(value["samples"]),
        "observed_monotonic": now, "requested_monotonic": now + 0.1,
        "closing_started_monotonic": now + 0.2, "closing_finished_monotonic": now + 0.5,
        "prefixes": {name: {"bytes": last[name + "_bytes"], "sha256": last[name + "_sha256"]}
                     for name in ("startup", "lifecycle")},
        "streams": {name: {"signal_requested_monotonic": now + 0.25, "signal_sent_monotonic": now + 0.3,
                           "stopped_monotonic": now + 0.4, "signal": "SIGTERM", "signal_delivered": True,
                           "returncode": 0, "raw_bytes": binding(value[name + "_text"])["bytes"],
                           "raw_sha256": binding(value[name + "_text"])["sha256"]}
                    for name in ("startup", "lifecycle")}}


def fixture():
    """A fresh foreground process, true native overlap, 135s + 15s observed."""
    enqueue = startup("restore_enqueued", 0.1, pending=True)
    entry = startup("backgrounded", 0.5, pending=True, native=True, background=True)
    result = startup("restore_result_observed", 0.7, background=True)
    active = state(lifecycle.ACTIVE, 0)
    suspended = state(lifecycle.SUSPENDED, 1)
    resumed = state(lifecycle.ACTIVE, 137)
    value = dict(startup_text=HEADER + enqueue + entry + result,
                 lifecycle_text=HEADER + active + suspended + resumed,
                 bundle_id=BUNDLE, process_name="Chat", pid=PID, executable=EXECUTABLE,
                 device=DEVICE, initial_prefixes={name: binding(HEADER) for name in ("startup", "lifecycle")},
                 samples=[], old_pid=101)
    times = [0, 1, *range(6, 137, 5), 137, 142, 147, 152]
    for t in times:
        first = HEADER + enqueue if t < 1 else value["startup_text"]
        second = HEADER + active + (suspended if t >= 1 else "") + (resumed if t >= 137 else "")
        sample = {"observed_monotonic": t, "pid": PID, "executable": EXECUTABLE,
                  "loggers_alive": True, "stderr": ""}
        for name, text in (("startup", first), ("lifecycle", second)):
            for key, item in binding(text).items():
                sample[name + "_" + key] = item
        value["samples"].append(sample)
    bind_cutoff(value)
    return value


def replace_capture(value, name, old, new):
    """Rebind synthetic snapshots after an intentional fixture mutation."""
    text = value[name + "_text"]
    encoded = text.encode()
    for sample in value["samples"]:
        prefix = encoded[:sample[name + "_bytes"]].decode().replace(old, new)
        for key, item in binding(prefix).items():
            sample[name + "_" + key] = item
    value[name + "_text"] = text.replace(old, new)
    bind_cutoff(value)


class LifecycleTests(unittest.TestCase):
    def reject(self, value, *, rebind=True):
        with self.assertRaises(lifecycle.LifecycleError):
            if rebind:
                bind_cutoff(value)
            lifecycle.evaluate_early_background(**value)

    def test_true_native_overlap_and_observed_suspension(self):
        value = fixture()
        before = copy.deepcopy(value)
        evidence = lifecycle.evaluate_early_background(**value)
        self.assertEqual(value, before)
        self.assertTrue(evidence["restore_in_flight"])
        self.assertEqual(evidence["suspended_seconds"], 135)
        self.assertEqual(evidence["resumed_seconds"], 15)
        self.assertEqual([evidence[key] for key in ("suspended_started_monotonic", "suspended_last_monotonic",
                                                   "resumed_started_monotonic", "resumed_last_monotonic")],
                         [1, 136, 137, 152])
        self.assertFalse(evidence["runtime_acceptance"])

    def test_ready_prefix_helper_rejects_partial_disconnected_or_wrong_streams(self):
        self.assertEqual(lifecycle.capture_prefixes(startup_text=HEADER, lifecycle_text=HEADER, device=DEVICE),
                         fixture()["initial_prefixes"])
        for bad in (HEADER.rstrip("\n"), HEADER + "[disconnected:synthetic-device]\n",
                    "[connected:other-device]\n", HEADER + "\ud800\n"):
            with self.assertRaises(lifecycle.LifecycleError):
                lifecycle.capture_prefixes(startup_text=bad, lifecycle_text=HEADER, device=DEVICE)

    def test_native_pending_and_background_are_all_required(self):
        for field in ("restore_pending_result", "native_restoring_observed", "app_backgrounded"):
            value = fixture()
            old = startup("backgrounded", 0.5, pending=True, native=True, background=True)
            replace_capture(value, "startup", old, old.replace(field + "=true", field + "=false"))
            self.reject(value)

    def test_cold_background_start_does_not_qualify(self):
        value = fixture()
        replace_capture(value, "startup", startup("restore_enqueued", 0.1, pending=True),
                        startup("restore_enqueued", 0.1, pending=True, background=True))
        self.reject(value)
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.ACTIVE, 0), state("running-active-NotVisible", 0))
        self.reject(value)

    def test_resign_active_is_not_background_entry(self):
        value = fixture()
        replace_capture(value, "startup", "phase=backgrounded", "phase=resign_active")
        self.reject(value)

    def test_terminal_before_entry_cannot_be_repaired(self):
        value = fixture()
        entry = startup("backgrounded", 0.5, pending=True, native=True, background=True)
        result = startup("restore_result_observed", 0.7, background=True)
        replace_capture(value, "startup", entry + result, result + entry)
        self.reject(value)

    def test_no_result_or_cancellation_is_not_success(self):
        for phase in ("restore_cancelled", "restore_dispatch_failed", "backgrounded"):
            value = fixture()
            replace_capture(value, "startup", "phase=restore_result_observed", "phase=" + phase)
            self.reject(value)
        value = fixture()
        replace_capture(value, "startup", startup("restore_result_observed", 0.7, background=True), "")
        self.reject(value)

    def test_wrong_pid_or_wrong_process_markers_fail(self):
        for old, new in ((f"Chat[{PID}]", f"Chat[{PID + 1}]"), ("Chat[", "NotChat[")):
            value = fixture()
            replace_capture(value, "startup", old, new)
            self.reject(value)

    def test_missing_or_late_os_foreground_fails(self):
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.ACTIVE, 0), "")
        self.reject(value)
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.ACTIVE, 0), state(lifecycle.ACTIVE, 0.6))
        self.reject(value)

    def test_device_timestamps_preserve_day_and_reject_backwards_order(self):
        value = fixture()
        replace_capture(value, "startup", stamp(0.7), "Mar 09 12:00:00.700000")
        self.reject(value)
        value = fixture()
        replace_capture(value, "lifecycle", stamp(137), "Mar 09 12:02:17.000000")
        self.reject(value)

    def test_partial_record_and_duplicate_marker_fail(self):
        value = fixture()
        value["startup_text"] = value["startup_text"].rstrip("\n")
        self.reject(value)
        value = fixture()
        record = startup("restore_result_observed", 0.7, background=True)
        replace_capture(value, "startup", record, record + record)
        self.reject(value)

    def test_stale_prelaunch_markers_cannot_supply_overlap(self):
        value = fixture()
        value["initial_prefixes"]["startup"] = binding(value["startup_text"])
        self.reject(value)

    def test_append_cannot_move_device_clock_before_initial_capture(self):
        value = fixture()
        before = state(lifecycle.ACTIVE, 200, pid=101)
        value["lifecycle_text"] = HEADER + before + value["lifecycle_text"][len(HEADER):]
        value["initial_prefixes"]["lifecycle"] = binding(HEADER + before)
        for sample in value["samples"]:
            size = sample["lifecycle_bytes"] + len(before.encode())
            sample["lifecycle_bytes"] = size
            sample["lifecycle_sha256"] = hashlib.sha256(value["lifecycle_text"].encode()[:size]).hexdigest()
        self.reject(value)

    def test_initial_boundaries_require_ready_complete_records(self):
        for delta in (-1, 1):
            value = fixture()
            data = value["startup_text"].encode()[:len(HEADER.encode()) + delta]
            value["initial_prefixes"]["startup"] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
            self.reject(value)

    def test_stream_reconnect_loss_error_or_wrong_device_fail(self):
        for extra in ("[disconnected:synthetic-device]\n", HEADER, "lost 1 messages\n"):
            value = fixture()
            replace_capture(value, "startup", startup("restore_result_observed", 0.7, background=True),
                            startup("restore_result_observed", 0.7, background=True) + extra)
            self.reject(value)
        for field, item in (("loggers_alive", False), ("stderr", "observer error")):
            value = fixture(); value["samples"][3][field] = item; self.reject(value)
        value = fixture(); value["device"] = "another-device"; self.reject(value)

    def test_prefix_hash_tampering_truncation_and_unobserved_tail_fail(self):
        value = fixture(); value["samples"][3]["startup_sha256"] = "0" * 64; self.reject(value)
        value = fixture(); value["samples"][3]["lifecycle_bytes"] = len(HEADER); self.reject(value)
        value = fixture()
        value["samples"][-1]["lifecycle_bytes"] -= 1
        value["samples"][-1]["lifecycle_sha256"] = hashlib.sha256(value["lifecycle_text"].encode()[:-1]).hexdigest()
        self.reject(value)

    def test_process_replacement_missing_identity_and_gap_fail(self):
        for field, item in (("pid", PID + 1), ("pid", True), ("executable", "file:///other"),
                            ("observed_monotonic", float("nan")), ("observed_monotonic", 100)):
            value = fixture(); value["samples"][3][field] = item; self.reject(value)
        value = fixture(); del value["samples"][3]["pid"]; self.reject(value)

    def test_same_bundle_other_process_after_launch_fails(self):
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.SUSPENDED, 1), state(lifecycle.SUSPENDED, 1, pid=103))
        self.reject(value)

    def test_explicit_old_death_only_before_new_activation(self):
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.ACTIVE, 0),
                        state("not-running", -0.1, pid=101) + state(lifecycle.ACTIVE, 0))
        lifecycle.evaluate_early_background(**value)
        for oldstate, when in ((lifecycle.ACTIVE, -0.1), ("not-running", 0.2)):
            value = fixture()
            replace_capture(value, "lifecycle", state(lifecycle.SUSPENDED, 1),
                            state(oldstate, when, pid=101) + state(lifecycle.SUSPENDED, 1))
            self.reject(value)

    def test_actual_suspension_not_background_running(self):
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.SUSPENDED, 1), state("running-active-NotVisible", 1))
        self.reject(value)

    def test_short_host_suspension_and_resume_fail(self):
        value = fixture()
        for row in value["samples"]:
            if row["observed_monotonic"] >= 136:
                row["observed_monotonic"] -= 0.1
        self.reject(value)
        value = fixture(); value["samples"][-1]["observed_monotonic"] = 151.9; self.reject(value)

    def test_host_delivery_delay_cannot_extend_short_device_suspension(self):
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.ACTIVE, 137), state(lifecycle.ACTIVE, 135.9))
        self.reject(value)

    def test_transient_os_exit_cannot_hide_between_polls(self):
        value = fixture()
        replace_capture(value, "lifecycle", state(lifecycle.ACTIVE, 137),
                        state(lifecycle.ACTIVE, 130) + state(lifecycle.SUSPENDED, 131) + state(lifecycle.ACTIVE, 137))
        self.reject(value)

    def test_intentional_terminal_disconnect_keeps_raw_and_live_cutoff_distinct(self):
        # Same terminal syntax as the retained physical capture; identity synthetic.
        value = fixture()
        live = copy.deepcopy(value["samples"])
        for name in ("startup", "lifecycle"):
            value[name + "_text"] += "[disconnected:" + DEVICE + "]\n"
        bind_cutoff(value)
        evidence = lifecycle.evaluate_early_background(**value)
        self.assertEqual(value["samples"], live)
        self.assertEqual(evidence["process_samples"], len(live))
        self.assertEqual(evidence["suspended_seconds"], 135)
        self.assertEqual(evidence["resumed_seconds"], 15)
        self.assertEqual(evidence["startup_sha256"], binding(value["startup_text"])["sha256"])
        self.assertEqual(evidence["startup_measured_sha256"], live[-1]["startup_sha256"])
        self.assertNotEqual(evidence["startup_sha256"], evidence["startup_measured_sha256"])

    def test_disconnect_before_cutoff_and_postcutoff_records_fail(self):
        value = fixture()
        replace_capture(value, "startup", startup("restore_result_observed", 0.7, background=True),
                        startup("restore_result_observed", 0.7, background=True) + "[disconnected:" + DEVICE + "]\n")
        self.reject(value)
        for name, extra in (("startup", "lost 1 messages\n"),
                            ("startup", startup("backgrounded", 153, background=True)),
                            ("lifecycle", state(lifecycle.SUSPENDED, 153)),
                            ("lifecycle", state(lifecycle.ACTIVE, 153)),
                            ("startup", "[disconnected:other-device]\n"),
                            ("startup", "[disconnected:" + DEVICE + "]\n[disconnected:" + DEVICE + "]\n"),
                            ("startup", "[disconnected:" + DEVICE + "]")):
            value = fixture(); value[name + "_text"] += extra; self.reject(value)

    def test_cutoff_intent_timing_raw_hash_and_live_sample_cannot_be_forged(self):
        changes = (lambda c: c.update(intent="cleanup-after-failure"),
                   lambda c: c.update(requested_monotonic=151),
                   lambda c: c.update(closing_started_monotonic=151),
                   lambda c: c.update(closing_finished_monotonic=200),
                   lambda c: c.update(sample_index=0),
                   lambda c: c["streams"]["startup"].update(signal_delivered=False),
                   lambda c: c["streams"]["startup"].update(returncode=-9),
                   lambda c: c["streams"]["startup"].update(raw_sha256="0" * 64),
                   lambda c: c["streams"]["startup"].update(signal_requested_monotonic=151),
                   lambda c: c.update(samples_sha256="0" * 64))
        for change in changes:
            value = fixture(); change(value["measurement_cutoff"]); self.reject(value, rebind=False)
        value = fixture(); value["samples"][-1]["observed_monotonic"] = 153
        self.reject(value, rebind=False)


if __name__ == "__main__":
    unittest.main()
