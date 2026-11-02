"""Offline resource refusal/depletion tests; no device, native processes or large files."""
import json
from pathlib import Path
import tempfile
import signal
import threading
import time
import unittest
from unittest.mock import Mock, patch

from ios_ordinary_relaunch import EvidenceError
from ios_capture_capacity import CaptureCapacity, CapacityWindowMixin, POLICY, GIB, RECEIPT_RESERVE, MAX_JSON_BYTES
import ios_relaunch_adapters as adapters
import test_ios_relaunch_adapters as window_tests


class CapacityTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.out = Path(self.directory.name) / 'capture'
        self.free = POLICY['admission_free_bytes']
        self.output = 0
        self.now = 10.0
        self.abort = Mock()

    def budget(self):
        return CaptureCapacity(self.out, POLICY, deadline=610, clock=lambda: self.now,
            free_probe=lambda: self.free, output_probe=lambda: self.output, abort=self.abort)

    def test_admission_refuses_before_creating_outputs(self):
        self.free -= 1
        with self.assertRaises(EvidenceError): self.budget()
        self.assertFalse(self.out.exists())
        self.abort.assert_not_called()

    def test_growth_trigger_retains_peak_and_aborts_without_grace(self):
        budget = self.budget()
        self.free -= GIB
        budget.check()
        self.free -= 1
        with self.assertRaises(EvidenceError): budget.check()
        self.abort.assert_called_once()
        self.assertEqual(budget.snapshot()['peak_filesystem_growth_bytes'], GIB + 1)
        self.assertFalse(budget.snapshot()['passed'])
        self.free += 2 * GIB
        with self.assertRaises(EvidenceError): budget.check()

    def test_shutdown_depletion_is_not_hidden_by_later_recovery(self):
        budget = self.budget()
        budget.set_phase('shutdown')
        self.free = 12 * GIB - 1
        with self.assertRaises(EvidenceError): budget.check()
        self.free = 20 * GIB
        self.assertLess(budget.snapshot()['phases']['shutdown']['minimum_free_bytes'], 12 * GIB)
        self.assertTrue(budget.snapshot()['failures'])

    def test_final_write_refuses_projected_output_overflow(self):
        budget = self.budget()
        self.out.mkdir()
        self.output = POLICY['max_output_bytes'] - RECEIPT_RESERVE - 32
        with self.assertRaises(EvidenceError): budget.write_json(self.out / 'cpu.json', {'data': 'x' * 64})
        self.assertFalse((self.out / 'cpu.json').exists())
        self.abort.assert_called()

    def test_post_write_depletion_is_rejected(self):
        budget = self.budget()
        self.out.mkdir()
        calls = 0
        def free_probe():
            nonlocal calls
            calls += 1
            return self.free if calls < 3 else 11 * GIB
        budget.free_probe = free_probe
        with self.assertRaises(EvidenceError): budget.write_json(self.out / 'capture.json', {'data': 'x' * 100})
        self.assertFalse(budget.snapshot()['passed'])

    def test_final_receipt_is_bounded_and_binds_evidence(self):
        budget = self.budget()
        self.out.mkdir()
        path = self.out / 'cpu.json'
        budget.write_json(path, {'passed': True})
        receipt = budget.finish(success=True, evidence=[path])
        data = json.loads(Path(receipt['path']).read_text())
        self.assertTrue(data['passed'])
        self.assertEqual(data['evidence'][0]['path'], str(path.resolve()))
        self.assertIn('final_evidence', data['phases'])
        self.assertLessEqual(Path(receipt['path']).stat().st_size, RECEIPT_RESERVE)

    def test_sampling_gap_invalidates_capture(self):
        budget = self.budget()
        self.now += 0.501
        with self.assertRaises(EvidenceError): budget.check()
        self.assertGreater(budget.snapshot()['maximum_sample_gap_seconds'], 0.5)

    def test_deadline_and_owner_exit_invalidate_capture(self):
        for reason in ('deadline', 'owner'):
            with self.subTest(reason=reason):
                budget = self.budget()
                if reason == 'deadline': budget.deadline = self.now
                else: budget.owner_alive = lambda: False
                with self.assertRaises(EvidenceError): budget.check()
                self.assertIn('deadline' if reason == 'deadline' else 'owner', ';'.join(budget.failures))

    def test_probe_failure_fails_closed(self):
        budget = self.budget()
        budget.free_probe = Mock(side_effect=OSError('volume disappeared'))
        with self.assertRaises(EvidenceError): budget.check()
        self.assertFalse(budget.snapshot()['passed'])
        self.abort.assert_called()

    def test_evidence_cannot_escape_or_grow_without_bound(self):
        budget = self.budget()
        self.out.mkdir()
        outside = self.out.parent / 'outside.json'
        link = self.out / 'link.json'
        link.symlink_to(outside)
        for path, value in ((outside, {}), (link, {}), (self.out / 'large.json', {'x': 'x' * MAX_JSON_BYTES})):
            with self.subTest(path=path), self.assertRaises(EvidenceError): budget.write_json(path, value)
        self.assertFalse(outside.exists())
        self.assertFalse((self.out / 'large.json').exists())

    def test_missing_bound_evidence_invalidates_final_receipt(self):
        budget = self.budget()
        self.out.mkdir()
        with self.assertRaises(EvidenceError): budget.finish(success=True, evidence=[self.out / 'missing'])
        self.assertFalse(json.loads((self.out / 'capacity.json').read_text())['passed'])

    def test_fsync_failure_cannot_leave_successful_receipt(self):
        budget = self.budget()
        self.out.mkdir()
        with patch('ios_capture_capacity.os.fsync', side_effect=OSError('no capacity')):
            with self.assertRaises(EvidenceError): budget.finish(success=True)
        self.assertEqual((self.out / 'capacity.json').read_bytes(), b'')
        self.assertFalse(budget.final_observations['passed'])

    def test_last_receipt_write_breach_is_recorded(self):
        budget = self.budget()
        self.out.mkdir()
        writes = 0
        def fsync(_):
            nonlocal writes
            writes += 1
            if writes == 2: self.free -= GIB + 1
        with patch('ios_capture_capacity.os.fsync', side_effect=fsync):
            with self.assertRaises(EvidenceError): budget.finish(success=True)
        self.assertFalse(json.loads((self.out / 'capacity.json').read_text())['passed'])
        self.assertEqual(budget.final_observations['peak_filesystem_growth_bytes'], GIB + 1)

    def test_background_monitor_stays_live_through_final_receipt_flushes(self):
        budget = self.budget()
        self.out.mkdir()
        budget.start()
        self.addCleanup(budget.stop.set)
        writes = []
        def fsync(_):
            self.assertTrue(budget.thread.is_alive())
            self.assertFalse(budget.stop.is_set())
            writes.append(True)
        with patch('ios_capture_capacity.os.fsync', side_effect=fsync): budget.finish(success=True)
        self.assertEqual(len(writes), 2)
        self.assertFalse(budget.thread.is_alive())
        self.assertGreater(budget.final_observations['samples'], json.loads((self.out / 'capacity.json').read_text())['samples'])

    def test_last_post_monitor_probe_cannot_hide_deadline_or_sampling_overrun(self):
        root = self.out
        for reason, duration, remaining in (('sampling', 0.501, 600), ('deadline', 0.45, 0.4)):
            with self.subTest(reason=reason):
                self.out = root / reason
                self.out.mkdir(parents=True)
                self.now = 10.0
                budget = self.budget()
                budget.deadline = self.now + remaining
                delayed = False
                def probe():
                    nonlocal delayed
                    if budget.stop.is_set() and not delayed:
                        self.assertFalse(budget.thread.is_alive())
                        delayed = True
                        self.now += duration
                    return self.free
                budget.free_probe = probe
                budget.start()
                self.addCleanup(budget.stop.set)
                with self.assertRaises(EvidenceError): budget.finish(success=True)
                self.assertTrue(delayed)
                self.assertFalse(json.loads((self.out / 'capacity.json').read_text())['passed'])
                self.assertFalse(budget.final_observations['passed'])
                self.assertAlmostEqual(budget.final_observations['maximum_probe_duration_seconds'], duration)
                self.assertGreaterEqual(budget.final_observations['phases']['final_evidence']['last_monotonic'], 10 + duration)

    def test_failed_operation_gets_unaccepted_capacity_receipt(self):
        budget = self.budget()
        self.out.mkdir()
        with self.assertRaises(EvidenceError): budget.finish(success=False)
        data = json.loads((self.out / 'capacity.json').read_text())
        self.assertFalse(data['passed'])
        self.assertTrue(data['failures'])

    def test_policy_numbers_cannot_be_relaxed(self):
        for key in ('minimum_free_bytes', 'admission_free_bytes', 'max_filesystem_growth_bytes', 'shutdown_headroom_bytes', 'max_output_bytes'):
            changed = dict(POLICY); changed[key] += 1
            with self.subTest(key=key), self.assertRaises(EvidenceError):
                CaptureCapacity(self.out, changed, deadline=610, clock=lambda: self.now, free_probe=lambda: self.free)


class CapacityWindow(CapacityWindowMixin, adapters.OwnedWindow):
    _guard_without_capacity = adapters.OwnedWindow.guard
    _close_without_capacity = adapters.OwnedWindow.close


class WindowCapacityTests(unittest.TestCase):
    def setUp(self):
        self.fixture = window_tests.WindowTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.free = POLICY['admission_free_bytes']
        self.fixture.config.update(minimum_free_bytes=POLICY['minimum_free_bytes'], max_output_bytes=POLICY['max_output_bytes'])
        self.fixture.observation_input()
        disk = patch.object(adapters.shutil, 'disk_usage', side_effect=lambda _: type('Usage', (), {'free': self.free})())
        disk.start(); self.addCleanup(disk.stop)
        self.killed = threading.Event()
        def killpg(group, sig):
            if sig == 0: raise ProcessLookupError
            if sig == signal.SIGKILL: self.killed.set()
        signals = patch.object(adapters.os, 'killpg', side_effect=killpg)
        self.signals = signals.start(); self.addCleanup(signals.stop)
        with patch.object(adapters, 'OwnedWindow', CapacityWindow):
            self.window = self.fixture.admit(mode='observation-only', output_ceiling=POLICY['max_output_bytes'])
        self.budget = CaptureCapacity(self.window.out, POLICY, deadline=100,
            clock=lambda: self.fixture.clock, free_probe=lambda: self.free)
        self.window.attach_capacity(self.budget)
        def stop():
            self.budget.stop.set()
            self.budget.thread.join(timeout=1)
        self.addCleanup(stop)

    def test_depletion_during_shutdown_kills_immediately_and_invalidates_closure(self):
        child = Mock(stdin=None)
        child.poll.side_effect = lambda: 0 if self.killed.is_set() else None
        def wait(timeout):
            self.free -= GIB + 1
            self.assertTrue(self.killed.wait(1), 'monitor waited for shutdown grace instead of killing')
            child.returncode = 0
            return 0
        child.wait.side_effect = wait
        self.window.children = [{'process': child, 'group': 123}]
        started = time.monotonic()
        with self.assertRaises(EvidenceError): self.window.close()
        self.assertLess(time.monotonic() - started, 2)
        self.assertIn((123, signal.SIGKILL), [call.args for call in self.signals.call_args_list])
        closure = json.loads((self.window.out / 'closure.json').read_text())
        self.assertFalse(closure['closed'])
        self.assertTrue(closure['failures'])
        self.assertGreater(self.budget.snapshot()['phases']['shutdown']['peak_filesystem_growth_bytes'], GIB)

    def test_background_monitor_continues_after_window_close(self):
        self.window.close()
        self.free -= GIB + 1
        deadline = time.monotonic() + 1
        while not self.budget.failures and time.monotonic() < deadline: time.sleep(0.01)
        self.assertTrue(self.budget.failures)
        with self.assertRaises(EvidenceError): self.window.invalidate_capacity_closure()
        self.assertFalse(json.loads((self.window.out / 'closure.json').read_text())['closed'])

    def test_successful_shutdown_and_final_files_remain_observed(self):
        self.window.close()
        path = self.window.out / 'cpu.json'
        self.budget.write_json(path, {'passed': True})
        receipt = self.budget.finish(success=True, evidence=[path, self.window.out / 'closure.json'])
        data = json.loads(Path(receipt['path']).read_text())
        self.assertTrue(data['passed'])
        self.assertTrue(self.budget.finished)
        self.assertFalse(self.budget.thread.is_alive())
        self.assertGreater(data['phases']['final_evidence']['peak_output_bytes'], 0)
        self.assertIn('shutdown', data['phases'])

    def test_failure_after_spawn_check_still_kills_registered_child(self):
        self.free -= GIB + 1
        with self.assertRaises(EvidenceError): self.budget.check()
        child = Mock(stdin=None)
        child.poll.return_value = None
        self.window.children.append({'process': child, 'group': 456})
        with self.assertRaises(EvidenceError): self.budget.check()
        self.assertIn((456, signal.SIGKILL), [call.args for call in self.signals.call_args_list])

    def test_no_space_for_failure_receipt_invalidates_prior_closure(self):
        self.window.close()
        self.free = 0
        with self.assertRaises(EvidenceError): self.budget.check()
        with self.assertRaises(EvidenceError): self.window.invalidate_capacity_closure()
        self.assertEqual((self.window.out / 'closure.json').read_bytes(), b'')


if __name__ == '__main__': unittest.main()
