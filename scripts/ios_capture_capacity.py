"""Opt-in capture capacity monitoring; importing this module performs no work.

These are sampled abort thresholds, not filesystem quotas. The extra 1.5 GiB
is shutdown headroom, not additional accepted growth. Every observed breach or
sampling gap invalidates the capture. Shared recorder defaults stay unchanged.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import signal
import threading
import time

from ios_ordinary_relaunch import EvidenceError, binding, exact, require

GIB = 1024**3
RECEIPT_RESERVE = 64 * 1024
MAX_JSON_BYTES = 4 * 1024**2
POLL_SECONDS = 0.1
MAX_SAMPLE_GAP = 0.5
POLICY = {
    'schema': 'iris-capture-capacity-policy-v1',
    'minimum_free_bytes': 12 * GIB,
    'max_filesystem_growth_bytes': GIB,
    'shutdown_headroom_bytes': 3 * GIB // 2,
    'admission_free_bytes': 29 * GIB // 2,
    'max_output_bytes': 512 * 1024**2,
}


def output_bytes(out):
    total = 0
    for path in out.rglob('*'):
        require(not path.is_symlink(), 'Unexpected output symlink')
        if path.is_file():
            info = path.stat()
            total += max(info.st_size, info.st_blocks * 512)
    return total


class CaptureCapacity:
    def __init__(self, out, policy, *, deadline, clock=time.monotonic,
                 free_probe=None, output_probe=None, abort=lambda: None,
                 owner_alive=lambda: True):
        exact(policy, set(POLICY))
        require(all(type(policy[k]) is type(v) and policy[k] == v for k, v in POLICY.items()),
                'Unqualified capture capacity policy')
        require(0 < deadline - clock() <= 600, 'Invalid capture capacity deadline')
        self.out, self.policy, self.deadline, self.clock = Path(out), dict(policy), deadline, clock
        self.free_probe = free_probe or (lambda: shutil.disk_usage(self.out.parent).free)
        self.output_probe = output_probe or (lambda: output_bytes(self.out))
        self.abort, self.owner_alive = abort, owner_alive
        self.initial_free = self.free_probe()
        require(self.initial_free >= policy['admission_free_bytes'], 'Insufficient capture admission capacity')
        self.lock = threading.RLock()
        self.stop = threading.Event()
        self.thread = None
        self.phase = 'admission'
        self.failures, self.phases = [], {}
        self.minimum_free = self.initial_free
        self.peak_output = self.peak_growth = 0
        self.samples = 0
        self.last_sample = None
        self.maximum_gap = 0.0
        self.maximum_probe_duration = 0.0
        self.aborted = self.finished = False
        self.receipt = None
        self.final_observations = None
        self.check()

    def fail(self, reason):
        with self.lock:
            reason = str(reason)[:1024]
            if reason not in self.failures and len(self.failures) < 16:
                self.failures.append(reason)
            if not self.aborted:
                self.aborted = True
                try:
                    self.abort()
                except Exception as error:
                    self.failures.append('Owned-group abort failed: ' + str(error)[:512])

    def check(self):
        with self.lock:
            already_failed = bool(self.failures)
            try:
                started = self.clock()
                free, size = self.free_probe(), self.output_probe()
                owner_alive = self.owner_alive()
                now = self.clock()
                require(type(free) is int and type(size) is int and free >= 0 and size >= 0,
                        'Invalid capacity observation')
                probe_duration = now - started
                gap = 0.0 if self.last_sample is None else now - self.last_sample
                self.maximum_probe_duration = max(self.maximum_probe_duration, probe_duration)
                self.maximum_gap = max(self.maximum_gap, gap)
                self.last_sample = now
                growth = max(0, self.initial_free - free)
                self.minimum_free = min(self.minimum_free, free)
                self.peak_growth = max(self.peak_growth, growth)
                self.peak_output = max(self.peak_output, size)
                self.samples += 1
                phase = self.phases.setdefault(self.phase, {
                    'first_monotonic': now, 'last_monotonic': now, 'minimum_free_bytes': free,
                    'peak_filesystem_growth_bytes': growth, 'peak_output_bytes': size, 'samples': 0,
                })
                phase.update(last_monotonic=now,
                             minimum_free_bytes=min(phase['minimum_free_bytes'], free),
                             peak_filesystem_growth_bytes=max(phase['peak_filesystem_growth_bytes'], growth),
                             peak_output_bytes=max(phase['peak_output_bytes'], size), samples=phase['samples'] + 1)
                if free < self.policy['minimum_free_bytes']: self.fail('Capture host reserve reached')
                if growth > self.policy['max_filesystem_growth_bytes']: self.fail('Capture filesystem growth exceeded')
                if size + RECEIPT_RESERVE > self.policy['max_output_bytes']: self.fail('Capture output budget exceeded')
                if probe_duration < 0 or probe_duration > MAX_SAMPLE_GAP: self.fail('Capture capacity probe duration exceeded')
                if gap < 0 or gap > MAX_SAMPLE_GAP: self.fail('Capture capacity sampling gap exceeded')
                if now >= self.deadline: self.fail('Capture capacity deadline exceeded')
                if not owner_alive: self.fail('Capture reservation owner exited')
            except Exception as error:
                self.fail('Capture capacity observation failed: ' + str(error))
            if already_failed:
                try: self.abort()
                except Exception as error: self.fail('Owned-group abort failed: ' + str(error))
            require(not self.failures, '; '.join(self.failures))

    def set_phase(self, phase):
        require(phase in ('active', 'shutdown', 'final_evidence'), 'Unknown capacity phase')
        with self.lock:
            self.phase = phase
        self.check()

    def start(self):
        require(self.thread is None and not self.finished, 'Capacity monitor already started')
        self.set_phase('active')
        def monitor():
            while not self.stop.wait(POLL_SECONDS):
                try: self.check()
                except EvidenceError: pass
        self.thread = threading.Thread(target=monitor, name='iris-capture-capacity', daemon=True)
        self.thread.start()

    def snapshot(self):
        with self.lock:
            return {
                'schema': 'iris-capture-capacity-observations-v1', 'policy': dict(self.policy),
                'passed': not self.failures, 'failures': list(self.failures),
                'initial_free_bytes': self.initial_free, 'minimum_free_bytes': self.minimum_free,
                'peak_filesystem_growth_bytes': self.peak_growth, 'peak_output_bytes': self.peak_output,
                'receipt_reserve_bytes': RECEIPT_RESERVE, 'samples': self.samples,
                'maximum_sample_gap_seconds': self.maximum_gap,
                'maximum_probe_duration_seconds': self.maximum_probe_duration,
                'phases': {key: dict(value) for key, value in self.phases.items()},
            }

    def write_json(self, path, value, *, emergency=False, replace=False):
        path = Path(path)
        require(path.parent.resolve() == self.out.resolve() and not path.is_symlink(),
                'Capacity evidence escaped output root')
        payload = (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()
        require(len(payload) <= (RECEIPT_RESERVE if emergency else MAX_JSON_BYTES), 'Oversized capacity evidence')
        if not emergency:
            self.check()
            if self.output_probe() + len(payload) + RECEIPT_RESERVE > self.policy['max_output_bytes']:
                self.fail('Final evidence exceeds capture output budget')
                self.check()
        else:
            try: self.check()
            except EvidenceError: pass
            require(self.free_probe() >= len(payload) + RECEIPT_RESERVE, 'No capacity for failure evidence')
        with path.open('wb' if replace else 'xb') as output:
            for offset in range(0, len(payload), RECEIPT_RESERVE):
                output.write(payload[offset:offset + RECEIPT_RESERVE])
                output.flush()
                if not emergency: self.check()
            os.fsync(output.fileno())
        if emergency:
            try: self.check()
            except EvidenceError: pass
        else:
            self.check()

    def finish(self, *, success, evidence=()):
        require(not self.finished, 'Capacity receipt already finalized')
        path = self.out / 'capacity.json'
        evidence_bindings = []
        def report():
            return {**self.snapshot(), 'evidence': evidence_bindings}
        try:
            try: self.set_phase('final_evidence')
            except EvidenceError: success = False
            if not success: self.fail('Capture operation did not complete successfully')
            evidence_bindings = [binding(item) for item in evidence]
            self.write_json(path, report(), emergency=bool(self.failures))
            self.check()
        except Exception as error:
            self.fail('Capacity final evidence failed: ' + str(error))
        try:
            try: self.check()
            except EvidenceError: pass
            # This bounded rewrite includes observations of the first receipt
            # flush. The caller also retains final_observations, sampled after
            # the last write, rather than claiming a file can observe itself.
            self.write_json(path, report(), emergency=True, replace=path.exists())
            self.receipt = binding(path)
            self.check()
        except Exception as error:
            self.fail('Capacity final receipt failed: ' + str(error))
            try:
                self.write_json(path, report(), emergency=True, replace=path.exists())
                self.receipt = binding(path)
            except Exception:
                # Never leave a previously successful receipt after an error.
                # Missing or invalid task-owned evidence cannot be accepted.
                if path.is_file() and not path.is_symlink():
                    with path.open('wb'): pass
        finally:
            try: self.check()
            except EvidenceError: pass
            self.stop.set()
            if self.thread is not None:
                self.thread.join(timeout=1)
                if self.thread.is_alive(): self.fail('Capacity monitor did not stop')
            self.finished = True
            try: self.check()
            except EvidenceError: pass
        if self.failures and self.receipt is not None:
            # A breach in the very last observation also invalidates the file.
            try:
                self.write_json(path, report(), emergency=True, replace=True)
                self.receipt = binding(path)
            except Exception:
                if path.is_file() and not path.is_symlink():
                    with path.open('wb'): pass
        self.final_observations = self.snapshot()
        require(success and not self.failures, 'Capture capacity acceptance failed')
        return self.receipt


class CapacityWindowMixin:
    """Opt-in wrappers around the existing qualified guard and close methods."""
    capacity = None

    def attach_capacity(self, capacity):
        self.capacity = capacity
        killed = set()
        def abort():
            failures = []
            for item in list(self.children):
                child, group = item['process'], item['group']
                if group in killed or child.poll() is not None:
                    continue
                try:
                    os.killpg(group, signal.SIGKILL)
                    killed.add(group)
                except ProcessLookupError:
                    pass
                except OSError as error:
                    failures.append(str(error))
            require(not failures, '; '.join(failures))
        capacity.abort = abort
        capacity.start()

    def guard(self):
        if self.capacity is not None: self.capacity.check()
        self._guard_without_capacity()

    def write_capacity_closure(self, value):
        path = self.out / 'closure.json'
        if self.capacity is None:
            path.write_text(json.dumps(value, sort_keys=True) + '\n')
            return
        try: self.capacity.check()
        except EvidenceError: pass
        if self.capacity.failures:
            value['closed'] = False
            value['failures'] = list(dict.fromkeys(value['failures'] + self.capacity.failures))
        try:
            self.capacity.write_json(path, value, emergency=not value['closed'], replace=path.exists())
        except Exception:
            # A late breach must not leave an accepted closure behind. If even
            # the reserved failure record cannot be written, absence/invalidity
            # of this task-owned receipt is an explicit acceptance failure.
            value['closed'] = False
            value['failures'] = list(dict.fromkeys(value['failures'] + self.capacity.failures))
            try:
                self.capacity.write_json(path, value, emergency=True, replace=path.exists())
            except Exception:
                if path.exists():
                    with path.open('wb'): pass
            raise
        require(value['closed'], 'Capture resource closure failed')

    def invalidate_capacity_closure(self):
        path = self.out / 'closure.json'
        if self.capacity is None or not self.capacity.failures or not path.is_file(): return
        self.write_capacity_closure(json.loads(path.read_text()))

    def close(self):
        if self.closed: return
        if self.capacity is not None:
            try: self.capacity.set_phase('shutdown')
            except EvidenceError: pass
        try:
            self._close_without_capacity()
        finally:
            if self.capacity is not None:
                try: self.capacity.check()
                finally: self.invalidate_capacity_closure()
        if self.capacity is not None: self.capacity.set_phase('final_evidence')
