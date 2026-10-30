#!/usr/bin/env python3
"""Bounded adapters for an explicitly reserved physical iPhone evidence window.

No device work occurs on import. The release owner supplies a private, hash-bound
admission and pinned tools. This adapter never enables the retained HID helper,
installs an app, pairs a device, builds a UI runner, or retries an action.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time
import uuid

import native_lab
import ios_relaunch_lifecycle as lifecycle
from ios_ordinary_relaunch import EvidenceError, binding, exact, finite, read_bound, require, validate_distribution, verify_binding


HOME_DISABLED_SHA256 = '74a9a77fef0b4a5ad3699eb17a390a59e1d16247f1a920fe3e1f881a7c79f6c4'
HOME_QUALIFICATION_SHA256 = '708b4c79ccbcea27a1152a64177cec0ce86c6b010a27c2534e57c23c1e0b1809'
MAX_OUTPUT = 100 * 1024 * 1024
HOME_EVENTS = ['ARMED', 'FIRE', 'home_down_requested', 'home_down_written', 'home_up_written',
               'home_write_complete', 'host_transport_close']


def validate_home_transcript(events, *, device, nonce, build, pid=None, complete=False):
    require(isinstance(events, list) and 0 < len(events) <= len(HOME_EVENTS), 'Missing or excessive HID events')
    require([row.get('event') for row in events] == HOME_EVENTS[:len(events)], 'Unexpected HID event order or failure')
    previous = None
    for row in events:
        stamp = finite(row.get('monotonic'))
        require(previous is None or stamp >= previous, 'HID clock reversed')
        previous = stamp
        finite(row.get('unix'))
    armed = events[0]
    require(armed.get('nonce') == nonce and armed.get('build') == build and armed.get('device') == device
            and armed.get('transport') == 'coredeviceproxy-userspace', 'HID armed another target')
    for row in events:
        if row['event'] in ('FIRE', 'home_write_complete'):
            require(pid is not None and row.get('pid') == pid and row.get('nonce') == nonce
                    and row.get('build') == build, 'HID completion target changed')
        if row['event'] == 'home_write_complete':
            require(row.get('device_effect_observed') is False, 'Transport cannot claim device effect')
        if row['event'] == 'host_transport_close':
            require(row.get('completed') is True and row.get('errors') == [], 'HID transport did not close')
    if complete:
        require(len(events) == len(HOME_EVENTS), 'Incomplete HID operation')


class OwnedWindow:
    """Exact tools, direct-parent leases, bounded outputs and owned process groups."""
    def __init__(self, admission_binding, out, *, clock=time.monotonic):
        self.binding, self.clock = admission_binding, clock
        self.config = read_bound(admission_binding)
        c = self.config
        exact(c, {'schema', 'enabled', 'candidate', 'distribution_binding', 'device', 'coredevice_id', 'bundle_id', 'build', 'process_name',
                  'executable', 'owner_pid', 'output_root', 'deadline_monotonic', 'minimum_free_bytes', 'max_output_bytes', 'tools'})
        require(c['schema'] == 'iris-relaunch-adapters-v1' and c['enabled'] is True, 'Runtime is not admitted')
        distribution = validate_distribution(c['distribution_binding'], c['candidate'])
        require(all(c[key] == distribution[key] for key in ('device', 'coredevice_id', 'bundle_id'))
                and c['build'] == distribution['candidate']['apple_build']
                and c['executable'] == distribution['installed_url'] + c['process_name'],
                'Adapter target differs from qualified distribution')
        require(type(c['owner_pid']) is int and c['owner_pid'] == os.getppid()
                and native_lab.process_is_alive(c['owner_pid']), 'Missing live direct-parent owner')
        for resource in ('ios-testflight-acceptance', 'ios-device:' + c['device']):
            owner = json.loads((native_lab.lock_path(resource) / 'owner.json').read_text())
            require(owner['pid'] == c['owner_pid'], 'Wrong native lease owner')
        require(0 < finite(c['deadline_monotonic']) - clock() <= 600, 'Invalid runtime deadline')
        require(type(c['max_output_bytes']) is int and 0 < c['max_output_bytes'] <= MAX_OUTPUT, 'Invalid output budget')
        require(type(c['minimum_free_bytes']) is int and c['minimum_free_bytes'] > MAX_OUTPUT, 'Missing host reserve')
        exact(c['tools'], {'coredevice', 'syslog', 'python', 'home_helper', 'home_disabled', 'home_qualification'})
        self.tools = {name: verify_binding(value) for name, value in c['tools'].items()}
        disabled = self.tools['home_disabled'].read_bytes()
        require(c['tools']['home_disabled']['sha256'] == HOME_DISABLED_SHA256
                and c['tools']['home_qualification']['sha256'] == HOME_QUALIFICATION_SHA256,
                'HID helper differs from retained qualified source')
        require(disabled.count(b'EXECUTION_ENABLED = False') == 1
                and self.tools['home_helper'].read_bytes() == disabled.replace(b'EXECUTION_ENABLED = False', b'EXECUTION_ENABLED = True'),
                'HID helper activation contains another change')
        self.out = Path(out).resolve()
        require(c['output_root'] == str(self.out), 'Output root differs from admitted one-shot window')
        require(shutil.disk_usage(self.out.parent).free >= c['minimum_free_bytes'] + c['max_output_bytes'], 'Insufficient reserved capacity')
        self.out.mkdir(mode=0o700, exist_ok=False)
        self.children, self.serial = [], 0
        self.environment = {k: v for k, v in os.environ.items()
                            if not k.startswith(('DEVICECTL_CHILD_', 'DYLD_', 'PYTHON'))}
        self.environment.update(LANG='C', LC_ALL='C', TERM='dumb', PYTHONDONTWRITEBYTECODE='1')
        self.closed = False

    def guard(self):
        require(not self.closed and self.clock() < self.config['deadline_monotonic'], 'Runtime deadline or closed window')
        require(native_lab.process_is_alive(self.config['owner_pid']), 'Reservation owner exited')
        require(shutil.disk_usage(self.out).free >= self.config['minimum_free_bytes'], 'Host reserve reached')
        total = 0
        for path in self.out.rglob('*'):
            require(not path.is_symlink(), 'Unexpected output symlink')
            if path.is_file():
                info = path.stat(); total += max(info.st_size, info.st_blocks * 512)
        require(total + 4096 <= self.config['max_output_bytes'], 'Runtime output budget exceeded')

    def spawn(self, command, label, *, stdin=False):
        self.guard()
        self.serial += 1
        base = self.out / f'{self.serial:03d}-{label}'
        stdout, stderr = base.with_suffix('.stdout'), base.with_suffix('.stderr')
        with stdout.open('xb') as output, stderr.open('xb') as errors:
            child = subprocess.Popen(command, stdin=subprocess.PIPE if stdin else subprocess.DEVNULL,
                stdout=output, stderr=errors, start_new_session=True, env=self.environment)
            item = {'process': child, 'stdout': stdout, 'stderr': stderr, 'command': list(command), 'group': child.pid}
            self.children.append(item)
        return item

    def wait(self, item, seconds):
        deadline = min(self.clock() + seconds, self.config['deadline_monotonic'])
        while item['process'].poll() is None:
            self.guard()
            require(self.clock() < deadline, 'Owned command timed out')
            time.sleep(0.01)
        require(item['process'].returncode == 0, 'Owned command failed')
        self.guard()

    def close(self):
        if self.closed:
            return
        failures = []
        for item in reversed(self.children):
            child, group = item['process'], item['group']
            if child.stdin is not None:
                try: child.stdin.close()
                except (OSError, ValueError): pass
            # A reaped leader's numeric PID may have been reused. Never signal
            # a group solely because its number matches an old child receipt.
            if child.poll() is None:
                try: os.killpg(group, signal.SIGTERM)
                except ProcessLookupError: pass
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try: os.killpg(group, signal.SIGKILL)
                except ProcessLookupError: pass
                try: child.wait(timeout=2)
                except subprocess.TimeoutExpired: failures.append('owned child remains')
            try:
                os.killpg(group, 0)
                failures.append('owned process group remains')
            except ProcessLookupError:
                pass
        try:
            require(read_bound(self.binding) == self.config, 'Runtime admission changed')
            for value in self.config['tools'].values(): verify_binding(value)
        except (OSError, ValueError) as error:
            failures.append(str(error))
        self.closed = True
        record = {'schema': 'iris-relaunch-adapter-closure-v1', 'candidate': self.config['candidate'],
                  'device': self.config['device'], 'closed': not failures, 'failures': failures,
                  'groups': [x['group'] for x in self.children]}
        (self.out / 'closure.json').write_text(json.dumps(record, sort_keys=True) + '\n')
        require(not failures, 'Owned process-group closure failed')

    def __enter__(self): return self
    def __exit__(self, *_): self.close()


class CoreDevice:
    def __init__(self, window):
        self.window = window

    def run(self, command):
        bundle = self.window.config['bundle_id']
        require(command in (['device', 'info', 'processes'],
                            ['device', 'info', 'apps', '--filter', "bundleIdentifier == '" + bundle + "'"],
                            ['device', 'process', 'launch', bundle],
                            ['device', 'process', 'launch', '--terminate-existing', bundle]),
                'Unqualified CoreDevice operation')
        self.window.guard()
        output = self.window.out / f'coredevice-{self.window.serial + 1:03d}.json'
        args = [str(self.window.tools['coredevice']), *command[:3], '--device', self.window.config['device'],
                '--timeout', '6', '--json-output', str(output), *command[3:]]
        child = self.window.spawn(args, 'coredevice')
        self.window.wait(child, 7)
        require(output.stat().st_size <= 4 * 1024 * 1024, 'Oversized CoreDevice response')
        return json.loads(output.read_text())['result']


class Capture:
    def __init__(self, window, coredevice):
        self.window, self.core = window, coredevice
        self.streams, self.os_process = {}, None

    def start(self):
        c = self.window.config
        rows = self.core.run(['device', 'info', 'processes'])['runningProcesses']
        found = [r for r in rows if r.get('executable', '').endswith('/SpringBoard.app/SpringBoard')]
        require(len(found) == 1, 'Missing unique SpringBoard process')
        self.os_process = found[0]
        for name, process_filter, message in (('startup', c['process_name'], 'iris.startup '),
                                              ('lifecycle', str(self.os_process['processIdentifier']), c['bundle_id'])):
            path = self.window.out / (name + '.log')
            args = [str(self.window.tools['syslog']), '-u', c['device'], '--no-colors', '-p', process_filter,
                    '-m', message, '-o', str(path)]
            item = self.window.spawn(args, name)
            item['capture'] = path
            self.streams[name] = item
        deadline = min(self.window.clock() + 5, self.window.config['deadline_monotonic'])
        while True:
            self.window.guard()
            self.health()
            if all(x['capture'].exists() and x['capture'].read_bytes().startswith(('[connected:' + c['device'] + ']\n').encode())
                   for x in self.streams.values()):
                self.prefixes()
                return
            require(self.window.clock() < deadline, 'Loggers did not become ready')
            time.sleep(0.02)

    def health(self):
        require(len(self.streams) == 2, 'Missing loggers')
        for item in self.streams.values():
            require(item['process'].poll() is None and not item['stderr'].read_bytes()
                    and not item['stdout'].read_bytes(), 'Logger exited or reported an error')

    def texts(self):
        self.window.guard(); self.health()
        result = {}
        for name, item in self.streams.items():
            require(item['capture'].stat().st_size <= lifecycle.MAX_CAPTURE_BYTES, 'Capture size exceeded')
            # A concurrently appended tail is retained on disk, but is not yet a
            # complete observation. Final closure must evaluate the whole file.
            data = item['capture'].read_bytes()
            data = data[:data.rfind(b'\n') + 1]
            result[name + '_text'] = data.decode('utf-8')
        return result

    def prefixes(self):
        value = self.texts()
        lifecycle.capture_prefixes(**value, device=self.window.config['device'])
        return value

    def snapshot(self):
        rows = self.core.run(['device', 'info', 'processes'])['runningProcesses']
        require(self.os_process in rows, 'SpringBoard process changed')
        require(not any('-Runner.app/' in r.get('executable', '') for r in rows), 'XCTest runner present')
        selected = [{'pid': r['processIdentifier'], 'executable': r['executable']} for r in rows
                    if r.get('executable') == self.window.config['executable']]
        text = self.prefixes()['lifecycle_text']
        foreground = foreground_pid(text, self.window.config['bundle_id'], self.os_process['processIdentifier'])
        return {'observed_utc': time.time(), 'processes': selected, 'foreground_pid': foreground}

    def sample(self, expected_pid):
        snapshot = self.snapshot()
        require(len(snapshot['processes']) == 1 and snapshot['processes'][0]['pid'] == expected_pid, 'Observed process changed')
        texts = self.prefixes()
        result = {'observed_monotonic': self.window.clock(), 'pid': expected_pid,
                  'executable': self.window.config['executable'], 'loggers_alive': True, 'stderr': ''}
        for name in ('startup', 'lifecycle'):
            data = texts[name + '_text'].encode()
            result[name + '_bytes'], result[name + '_sha256'] = len(data), hashlib.sha256(data).hexdigest()
        return result

    def final_texts(self):
        require(self.window.closed, 'Close every owned process before final capture')
        closure = json.loads((self.window.out / 'closure.json').read_text())
        require(closure['closed'] is True, 'Process-group closure failed')
        for item in self.streams.values():
            require(not item['stdout'].read_bytes() and not item['stderr'].read_bytes(), 'Final logger error')
        # Do not trim the final tail: incomplete final records must fail parsing.
        return {name + '_text': item['capture'].read_bytes().decode('utf-8') for name, item in self.streams.items()}

    def final_capture(self, samples):
        return {**self.final_texts(), 'process_name': self.window.config['process_name'], 'samples': samples,
                'closure_binding': binding(self.window.out / 'closure.json')}


def foreground_pid(text, bundle_id, springboard_pid):
    """Read actual ordered SpringBoard target states; presence is not visibility."""
    source = r'SpringBoard(?:\(RunningBoardServices\))?\[' + str(springboard_pid) + r'\] <[^>]+>: '
    pattern = re.compile(source + r'Received state update for (\d+) \(app<' + re.escape(bundle_id)
                         + r'(?:\([^>]*\))?>, ([A-Za-z-]+)(?:[ ,].*)?$')
    last, previous_stamp = None, None
    for line in text.splitlines():
        if 'Received state update for ' not in line or bundle_id not in line:
            continue
        match = pattern.search(line)
        require(match is not None, 'Unrecognized foreground source record')
        stamp = lifecycle._stamp(line)
        require(previous_stamp is None or stamp >= previous_stamp, 'Foreground source clock reversed')
        previous_stamp, last = stamp, match
    return int(last[1]) if last and last[2] == lifecycle.ACTIVE else None


class PrearmedHome:
    def __init__(self, window):
        self.window, self.item = window, None
        self.nonce, self.fired = str(uuid.uuid4()), False

    def events(self, *, final=False):
        require(self.item is not None, 'HID helper not started')
        require(not self.item['stderr'].read_bytes(), 'HID helper reported an error')
        data = self.item['stdout'].read_bytes()
        require(len(data) <= 32768, 'Oversized HID transcript')
        require(not final or data.endswith(b'\n'), 'Incomplete final HID transcript')
        return [json.loads(line) for line in data.splitlines(keepends=True) if line.endswith(b'\n')]

    def arm(self):
        require(self.item is None, 'HID helper cannot be restarted')
        c = self.window.config
        command = [str(self.window.tools['python']), '-I', '-B', str(self.window.tools['home_helper']),
                   '--device', c['device'], '--nonce', self.nonce, '--build', c['build'], '--execute']
        self.item = self.window.spawn(command, 'home', stdin=True)
        deadline = min(self.window.clock() + 22, c['deadline_monotonic'])
        while True:
            self.window.guard()
            events = self.events()
            if events:
                validate_home_transcript(events, device=c['device'], nonce=self.nonce, build=c['build'])
                require(len(events) == 1 and self.item['process'].poll() is None, 'HID helper acted before launch')
                return {'armed': True, 'nonce': self.nonce, 'device': c['device'], 'bundle_id': c['bundle_id'], 'build': c['build']}
            require(self.item['process'].poll() is None and self.window.clock() < deadline, 'HID did not arm')
            time.sleep(0.002)

    def fire(self, armed, proof):
        require(not self.fired and self.item is not None, 'HID action cannot be retried')
        c = self.window.config
        require(armed == {'armed': True, 'nonce': self.nonce, 'device': c['device'], 'bundle_id': c['bundle_id'], 'build': c['build']}, 'Wrong armed target')
        require(proof['candidate']['apple_build'] == c['build'] and proof['replacement'] is True, 'Wrong launch proof')
        self.window.guard()
        self.fired = True  # Reserve before writing: uncertain outcome never retries.
        line = json.dumps({'op': 'FIRE', 'nonce': self.nonce, 'build': c['build'], 'pid': proof['pid']}) + '\n'
        self.item['process'].stdin.write(line.encode()); self.item['process'].stdin.flush()
        self.window.wait(self.item, 7)
        events = self.events(final=True)
        validate_home_transcript(events, device=c['device'], nonce=self.nonce, build=c['build'], pid=proof['pid'], complete=True)
        try:
            os.killpg(self.item['group'], 0)
        except ProcessLookupError:
            pass
        else:
            raise EvidenceError('HID group remains after local completion')
        return {'nonce': self.nonce, 'build': c['build'], 'pid': proof['pid'], 'sent_count': 1, 'acknowledged': True}
