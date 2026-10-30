#!/usr/bin/env python3
"""Fast passive restoration watcher for the existing one-shot relaunch driver.

Use ``ObservedRestorationHome(controller, capture, home)`` as the controller's
prearm/trigger callbacks; always close it before closing the owned capture.
No device work occurs on import. The existing controller validates the real
ordinary launch before ``fire`` can act. This watcher only schedules Home;
unchanged lifecycle evaluation must still prove native restoration overlap,
135 seconds suspended, same-audit-token resume and 15 resumed seconds.

Arrival clocks are observations of complete local log records, not device
emission times or measured transport latency. Physical responsiveness remains
unqualified until an actual candidate-bound observation proves the overlap.
"""
from __future__ import annotations

import hashlib
import os
import re
import threading
import time

import ios_relaunch_lifecycle as lifecycle
from ios_ordinary_relaunch import EvidenceError, require, validate_launch_result

POLL_SECONDS = 0.005
MARKER_WAIT_SECONDS = 15
MAX_RECORDS = 1024


class _Tail:
    """Read only appended bytes; preserve exact complete-record prefix bindings."""
    def __init__(self, path, initial):
        self.path = path
        require(not path.is_symlink(), 'Capture path is a symlink')
        self.stream = path.open('rb')
        try:
            self.identity = os.fstat(self.stream.fileno())
            raw = self.stream.read(len(initial) + 1)
            # A concurrent append is allowed, but the ready prefix must match.
            require(raw[:len(initial)] == initial, 'Ready capture prefix changed')
            self.stream.seek(len(initial))
            self.offset = len(initial)
            self.pending = b''
            self.digest = hashlib.sha256(initial)
            stamps = [lifecycle._stamp(x) for x in initial.decode().splitlines() if lifecycle.STAMP.match(x)]
            self.previous_stamp = stamps[-1] if stamps else None
        except BaseException:
            self.stream.close()
            raise

    def read(self):
        info = self.path.stat()
        require((info.st_dev, info.st_ino) == (self.identity.st_dev, self.identity.st_ino)
                and not self.path.is_symlink(), 'Capture file was replaced')
        require(self.offset + len(self.pending) <= info.st_size <= lifecycle.MAX_CAPTURE_BYTES,
                'Capture shrank or exceeded its bound')
        data = self.pending + self.stream.read(lifecycle.MAX_CAPTURE_BYTES + 1)
        require(self.offset + len(data) <= lifecycle.MAX_CAPTURE_BYTES, 'Capture exceeded its bound')
        boundary = data.rfind(b'\n') + 1
        self.pending = data[boundary:]
        records = []
        for raw in data[:boundary].splitlines(keepends=True):
            self.offset += len(raw)
            self.digest.update(raw)
            line = raw.decode('utf-8')
            require(not lifecycle.LOSS.search(line) and '[connected:' not in line,
                    'Live capture disconnected, reconnected or lost records')
            require('\x00' not in line and '\ufffd' not in line and '\r' not in line, 'Invalid capture text')
            stamp = lifecycle._stamp(line) if lifecycle.STAMP.match(line) else None
            if stamp is not None:
                require(self.previous_stamp is None or stamp >= self.previous_stamp, 'Device clock reversed')
                self.previous_stamp = stamp
            records.append((line.rstrip('\n'), stamp, self.offset, self.digest.hexdigest()))
        return records

    def close(self):
        self.stream.close()


class ObservedRestorationHome:
    """One Home only, after a real launch proof and a still-pending foreground enqueue.

    The worker only reads the existing owned logger files. It makes no device
    query and never invokes Home. Inventory latency therefore does not delay
    observing markers while the controller's normal launch is in progress.
    """
    def __init__(self, controller, capture, home, *, clock=time.monotonic, utc=time.time):
        self.controller, self.capture, self.home = controller, capture, home
        self.clock, self.utc = clock, utc
        self.condition = threading.Condition()
        self.stop = threading.Event()
        self.thread = None
        self.tails = {}
        self.used = False
        self.error = None
        self.record = {'records': [], 'fire_decision': None, 'launch_proof_observed_monotonic': None,
                       'poll_max_gap_seconds': 0, 'physical_overlap_claimed': False}
        c = capture.window.config
        self.startup = re.compile(re.escape(c['process_name']) + r'(?:\([^)]*\))?\[(\d+)\] <[^>]+>: '
            r'iris\.startup phase=(\w+) restore_pending_result=(true|false) '
            r'native_restoring_observed=(true|false) app_backgrounded=(true|false)$')
        self.state = re.compile(r'SpringBoard(?:\(RunningBoardServices\))?\['
            + str(capture.os_process['processIdentifier']) + r'\] <[^>]+>: '
            r'Received state update for (\d+) \(app<' + re.escape(c['bundle_id'])
            + r'(?:\([^>]*\))?>, ([A-Za-z-]+)(?:[ ,].*)?$')

    def arm(self):
        require(self.thread is None and not self.tails, 'Marker watcher cannot restart')
        armed = self.home.arm()
        texts = self.capture.prefixes()
        self.started = self.clock()
        self.deadline = min(self.started + MARKER_WAIT_SECONDS,
                            self.capture.window.config['deadline_monotonic'])
        try:
            for name in ('startup', 'lifecycle'):
                self.tails[name] = _Tail(self.capture.streams[name]['capture'], texts[name + '_text'].encode())
            self.thread = threading.Thread(target=self._watch, name='iris-restore-marker', daemon=False)
            self.thread.start()
        except BaseException:
            self.close()
            raise
        return armed

    def _read_batch(self):
        self.capture.health()
        rows = []
        # Consume both complete snapshots before deciding, including a result
        # delivered together with enqueue. Never fire from an earlier row alone.
        for name, tail in self.tails.items():
            batch = tail.read()
            observed, utc = self.clock(), self.utc()
            for line, stamp, size, digest in batch:
                pattern = self.startup if name == 'startup' else self.state
                relevant = ('iris.startup ' in line if name == 'startup'
                            else 'Received state update for ' in line
                            and self.capture.window.config['bundle_id'] in line)
                if not relevant:
                    continue
                match = pattern.search(line)
                require(match is not None and stamp is not None, 'Malformed watched record')
                row = {'stream': name, 'pid': int(match[1]), 'device_stamp': stamp,
                       'observed_monotonic': observed, 'observed_utc': utc,
                       'prefix_bytes': size, 'prefix_sha256': digest}
                if name == 'startup':
                    row.update(phase=match[2], pending=match[3] == 'true',
                               native=match[4] == 'true', backgrounded=match[5] == 'true')
                else:
                    row['state'] = match[2]
                rows.append(row)
        return rows

    def _watch(self):
        previous = self.started
        try:
            while not self.stop.is_set():
                now = self.clock()
                require(now < self.deadline, 'No eligible restoration marker within bounded wait')
                rows = self._read_batch()
                with self.condition:
                    self.record['poll_max_gap_seconds'] = max(self.record['poll_max_gap_seconds'], now - previous)
                    previous = now
                    require(len(self.record['records']) + len(rows) <= MAX_RECORDS, 'Too many watched records')
                    self.record['records'].extend(rows)
                    self.condition.notify_all()
                self.stop.wait(POLL_SECONDS)
        except BaseException as error:
            with self.condition:
                self.error = error
                self.condition.notify_all()

    def _eligible(self, pid):
        rows = [x for x in self.record['records'] if x['pid'] == pid]
        startup = [x for x in rows if x['stream'] == 'startup']
        if not startup:
            return None
        require(len(startup) == 1 and startup[0]['phase'] == 'restore_enqueued',
                'Restoration completed, changed or was coalesced before Home')
        enqueue = startup[0]
        require(enqueue['pending'] and not enqueue['backgrounded'], 'Restore did not enqueue in foreground')
        states = [x for x in rows if x['stream'] == 'lifecycle']
        if not states:
            return None
        require(states[-1]['state'] == lifecycle.ACTIVE, 'New process already left foreground')
        if not any(x['state'] == lifecycle.ACTIVE and x['device_stamp'] <= enqueue['device_stamp'] for x in states):
            return None
        return enqueue

    def fire(self, armed, proof):
        require(not self.used and self.thread is not None, 'Marker-triggered Home cannot retry')
        self.used = True  # Reserve before waiting: failures and uncertain writes never retry.
        r = self.controller.record
        require(r.get('launch_result_proof') is proof and r.get('launch_count') == 1
                and r.get('trigger_count') == 1, 'Missing actual controller launch proof')
        checked = validate_launch_result(self.controller.distribution, r['installed'], r['result'], r['command'],
            r['started_utc'], r['returned_utc'], proof['validated_utc'],
            executable=r['executable'], before=r['before'])
        require(checked == proof, 'Controller launch proof changed')
        with self.condition:
            self.record['launch_proof_observed_monotonic'] = self.clock()
            while True:
                if self.error is not None:
                    raise self.error
                require(not self.stop.is_set() and self.clock() < self.deadline, 'Marker watcher stopped or expired')
                enqueue = self._eligible(proof['pid'])
                if enqueue is not None:
                    self.record['fire_decision'] = {'pid': proof['pid'], 'observed_monotonic': self.clock(),
                                                    'observed_utc': self.utc(), 'enqueue': dict(enqueue)}
                    break
                self.condition.wait(POLL_SECONDS)
        # Existing pinned transport and its one-write/closure checks stay intact.
        # New markers can race this write; only final native-overlap evidence
        # establishes whether backgrounding actually happened soon enough.
        try:
            return self.home.fire(armed, proof)
        finally:
            self.stop.set()

    def close(self):
        self.stop.set()
        if self.thread is not None:
            self.thread.join(timeout=1)
            require(not self.thread.is_alive(), 'Marker watcher did not stop')
        for tail in self.tails.values():
            tail.close()
        if self.error is not None:
            raise self.error
