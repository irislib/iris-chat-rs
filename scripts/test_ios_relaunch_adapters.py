#!/usr/bin/env python3
"""Offline fake-process tests; no CoreDevice, USB, logging or signals executed."""
import copy
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch
from unittest.mock import Mock

import ios_relaunch_adapters as adapters
from ios_ordinary_relaunch import EvidenceError, binding
import test_ios_relaunch_lifecycle as log_fixture
from test_ios_ordinary_relaunch import DistributionFixture


def transcript():
    rows = [{'event': event, 'monotonic': i, 'unix': 100 + i} for i, event in enumerate(adapters.HOME_EVENTS)]
    rows[0].update(nonce='nonce', device='device', build='2026103000', transport='coredeviceproxy-userspace')
    for i in (1, 5): rows[i].update(nonce='nonce', build='2026103000', pid=102)
    rows[5]['device_effect_observed'] = False
    rows[6].update(completed=True, errors=[])
    return rows


class ProtocolTests(unittest.TestCase):
    def validate(self, rows):
        adapters.validate_home_transcript(rows, device='device', nonce='nonce', build='2026103000', pid=102, complete=True)

    def test_complete_local_write_and_closure(self): self.validate(transcript())

    def test_wrong_target_duplicate_action_or_closure_failure_fails(self):
        mutations = [lambda d: d[0].update(device='other'), lambda d: d[1].update(pid=103),
                     lambda d: d[5].update(device_effect_observed=True), lambda d: d[6].update(completed=False),
                     lambda d: d[6].update(errors=['close failed']), lambda d: d.insert(3, dict(d[2])),
                     lambda d: d[3].update(event='recovery_up_written'), lambda d: d[4].update(monotonic=-1),
                     lambda d: d.pop()]
        for mutation in mutations:
            value = transcript(); mutation(value)
            with self.subTest(mutation=mutation), self.assertRaises(EvidenceError): self.validate(value)

    def test_foreground_requires_exact_os_source_and_latest_state(self):
        text = log_fixture.state(adapters.lifecycle.ACTIVE, 0, pid=101)
        self.assertEqual(adapters.foreground_pid(text, log_fixture.BUNDLE, 1), 101)
        self.assertIsNone(adapters.foreground_pid('[connected:synthetic-device]\n', log_fixture.BUNDLE, 1))
        self.assertIsNone(adapters.foreground_pid(text + log_fixture.state(adapters.lifecycle.SUSPENDED, 1, pid=101), log_fixture.BUNDLE, 1))
        with self.assertRaises(EvidenceError): adapters.foreground_pid(text, log_fixture.BUNDLE, 99)
        with self.assertRaises(EvidenceError): adapters.foreground_pid(text + log_fixture.state(adapters.lifecycle.ACTIVE, -1), log_fixture.BUNDLE, 1)


class WindowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        source = self.root / 'distribution'; source.mkdir()
        self.distribution = DistributionFixture(source)
        self.clock = 10.0
        self.tools = {}
        for name in ('coredevice', 'syslog', 'python', 'home_qualification'):
            p = self.root / name; p.write_bytes(name.encode()); self.tools[name] = binding(p)
        p = self.root / 'home_disabled'; p.write_bytes(b'EXECUTION_ENABLED = False\n'); self.tools['home_disabled'] = binding(p)
        p = self.root / 'home_helper'; p.write_bytes(b'EXECUTION_ENABLED = True\n'); self.tools['home_helper'] = binding(p)
        self.config = {'schema': 'iris-relaunch-adapters-v1', 'enabled': True, 'device': 'synthetic-device',
                       'candidate': self.distribution.candidate, 'distribution_binding': self.distribution.config_binding,
                       'coredevice_id': 'synthetic-coredevice', 'bundle_id': 'example.test.app',
                       'build': self.distribution.candidate['apple_build'],
                       'process_name': 'Example', 'executable': self.distribution.executable, 'owner_pid': os.getppid(),
                       'output_root': str((self.root / 'output').resolve()),
                       'deadline_monotonic': 100, 'minimum_free_bytes': 200 * 1024**2,
                       'max_output_bytes': 1024**2, 'tools': self.tools}
        lease = self.root / 'lease'; lease.mkdir(); (lease / 'owner.json').write_text(json.dumps({'pid': os.getppid()}))
        for target, value in [('HOME_DISABLED_SHA256', self.tools['home_disabled']['sha256']),
                              ('HOME_QUALIFICATION_SHA256', self.tools['home_qualification']['sha256'])]:
            p = patch.object(adapters, target, value); p.start(); self.addCleanup(p.stop)
        for target, value in [('native_lab.lock_path', lambda _: lease), ('native_lab.process_is_alive', lambda _: True),
                              ('shutil.disk_usage', lambda _: SimpleNamespace(free=1024**3))]:
            p = patch('ios_relaunch_adapters.' + target, value); p.start(); self.addCleanup(p.stop)

    def admit(self):
        p = self.root / 'admission.json'; p.write_text(json.dumps(self.config))
        return adapters.OwnedWindow(binding(p), self.root / 'output', clock=lambda: self.clock)

    def test_requires_explicit_admission_before_any_process(self):
        self.config['enabled'] = False
        with patch.object(adapters.subprocess, 'Popen') as spawn, self.assertRaises(EvidenceError): self.admit()
        spawn.assert_not_called()
        self.assertFalse((self.root / 'output').exists())

    def test_admission_cannot_be_replayed_in_a_different_output_directory(self):
        self.config['output_root'] = str(self.root / 'other-output')
        with self.assertRaises(EvidenceError): self.admit()

    def test_invalid_parent_or_deadline_fails(self):
        self.config['owner_pid'] += 1
        with self.assertRaises(EvidenceError): self.admit()

    def test_adapter_cannot_query_another_device_than_qualified_distribution(self):
        self.config['device'] = 'another-phone'
        with patch.object(adapters.subprocess, 'Popen') as spawn, self.assertRaises(EvidenceError): self.admit()
        spawn.assert_not_called()
        self.config['owner_pid'] = os.getppid(); self.config['deadline_monotonic'] = 9
        with self.assertRaises(EvidenceError): self.admit()

    def test_enabled_helper_may_have_only_the_reviewed_one_line_delta(self):
        Path(self.tools['home_helper']['path']).write_bytes(b'EXECUTION_ENABLED = True\nextra()\n')
        self.tools['home_helper'] = binding(self.tools['home_helper']['path'])
        with self.assertRaises(EvidenceError): self.admit()

    def test_space_output_deadline_and_changed_tools_stop(self):
        window = self.admit(); self.clock = 101
        with self.assertRaises(EvidenceError): window.guard()
        self.clock = 10
        (window.out / 'oversized').write_bytes(b'x' * 1024**2)
        with self.assertRaises(EvidenceError): window.guard()
        (window.out / 'oversized').unlink()
        with patch.object(adapters.shutil, 'disk_usage', return_value=SimpleNamespace(free=1)), self.assertRaises(EvidenceError): window.guard()
        Path(self.tools['syslog']['path']).write_bytes(b'changed')
        with self.assertRaises(EvidenceError): window.close()
        self.assertFalse(json.loads((window.out / 'closure.json').read_text())['closed'])

    def test_exited_leader_is_not_signalled_using_a_reusable_group_number(self):
        window = self.admit()
        child = SimpleNamespace(stdin=None, poll=lambda: 0, wait=lambda timeout: 0)
        window.children = [{'process': child, 'group': 123}]
        with patch.object(adapters.os, 'killpg', side_effect=ProcessLookupError) as signal:
            window.close()
        self.assertEqual(signal.call_args_list[0].args, (123, 0))
        self.assertEqual(signal.call_count, 1)

    def test_owned_spawn_has_isolated_group_and_clean_environment(self):
        with patch.dict(os.environ, {'DEVICECTL_CHILD_TEST': 'forbidden', 'DYLD_FAKE': 'forbidden', 'PYTHONPATH': 'forbidden'}):
            window = self.admit()
        child = Mock(pid=123, stdin=None); child.poll.return_value = None; child.wait.return_value = 0
        with patch.object(adapters.subprocess, 'Popen', return_value=child) as popen:
            window.spawn([str(window.tools['coredevice']), 'argument'], 'synthetic')
        options = popen.call_args.kwargs
        self.assertTrue(options['start_new_session'])
        self.assertFalse(any(key.startswith(('DEVICECTL_CHILD_', 'DYLD_', 'PYTHONPATH')) for key in options['env']))
        def signal(group, signum):
            if signum == 0: raise ProcessLookupError()
        with patch.object(adapters.os, 'killpg', side_effect=signal) as kill:
            window.close()
        self.assertEqual(kill.call_args_list[0].args, (123, adapters.signal.SIGTERM))

    def test_coredevice_uses_pinned_tool_exact_device_and_bounded_command(self):
        window = self.admit(); core = adapters.CoreDevice(window)
        def spawn(command, label):
            output = Path(command[command.index('--json-output') + 1])
            output.write_text(json.dumps({'result': {'runningProcesses': []}}))
            return {'synthetic': True}
        with patch.object(window, 'spawn', side_effect=spawn) as start, patch.object(window, 'wait') as wait:
            self.assertEqual(core.run(['device', 'info', 'processes']), {'runningProcesses': []})
        command = start.call_args.args[0]
        self.assertEqual(command[0], str(window.tools['coredevice']))
        self.assertEqual(command[command.index('--device') + 1], self.config['device'])
        self.assertEqual(command[command.index('--timeout') + 1], '6')
        self.assertEqual(wait.call_args.args[1], 7)
        window.close()

    def test_ready_capture_rejects_logger_errors_and_incomplete_final_tail(self):
        window = self.admit()
        springboard = {'processIdentifier': 1, 'executable': 'file:///system/SpringBoard.app/SpringBoard'}
        core = Mock(); core.run.return_value = {'runningProcesses': [springboard]}
        capture = adapters.Capture(window, core)
        def spawn(command, label):
            log = Path(command[command.index('-o') + 1]); log.write_text('[connected:synthetic-device]\n')
            stdout, stderr = window.out / (label + '.stdout'), window.out / (label + '.stderr')
            stdout.write_bytes(b''); stderr.write_bytes(b'')
            child = Mock(); child.poll.return_value = None
            return {'process': child, 'stdout': stdout, 'stderr': stderr}
        with patch.object(window, 'spawn', side_effect=spawn): capture.start()
        self.assertEqual(set(capture.prefixes()), {'startup_text', 'lifecycle_text'})
        capture.streams['startup']['stderr'].write_text('logger error')
        with self.assertRaises(EvidenceError): capture.prefixes()
        capture.streams['startup']['stderr'].write_bytes(b'')
        capture.streams['startup']['capture'].write_text('[connected:synthetic-device]\npartial')
        window.close()
        final = capture.final_texts()
        self.assertTrue(final['startup_text'].endswith('partial'))
        with self.assertRaises(adapters.lifecycle.LifecycleError):
            adapters.lifecycle.capture_prefixes(**final, device=self.config['device'])

    def test_home_adapter_accepts_only_complete_bound_local_protocol(self):
        window = self.admit(); home = adapters.PrearmedHome(window)
        stdout, stderr = window.out / 'home.stdout', window.out / 'home.stderr'
        stderr.write_bytes(b'')
        rows = transcript()
        rows[0].update(nonce=home.nonce, device=self.config['device'], build=self.config['build'])
        for i in (1, 5): rows[i].update(nonce=home.nonce, build=self.config['build'])
        stdout.write_text(json.dumps(rows[0]) + '\n')
        child = Mock(stdin=io.BytesIO()); child.poll.return_value = None
        item = {'process': child, 'stdout': stdout, 'stderr': stderr, 'group': 123}
        with patch.object(window, 'spawn', return_value=item) as spawn:
            armed = home.arm()
        self.assertEqual(spawn.call_args.args[0][1:3], ['-I', '-B'])
        def finish(*_): stdout.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        proof = {'candidate': {'apple_build': self.config['build']}, 'replacement': True, 'pid': 102}
        with patch.object(window, 'wait', side_effect=finish), patch.object(adapters.os, 'killpg', side_effect=ProcessLookupError):
            result = home.fire(armed, proof)
        self.assertTrue(result['acknowledged'])
        sent = json.loads(child.stdin.getvalue())
        self.assertEqual(sent, {'op': 'FIRE', 'nonce': home.nonce, 'build': self.config['build'], 'pid': 102})
        stdout.write_bytes(stdout.read_bytes() + b'partial')
        with self.assertRaises(EvidenceError): home.events(final=True)
        window.close()

    def test_coredevice_rejects_unqualified_operations_before_spawn(self):
        window = self.admit(); core = adapters.CoreDevice(window)
        for cmd in (['device', 'process', 'terminate', '--pid', '1'],
                    ['device', 'process', 'launch', '--start-stopped', self.config['bundle_id']],
                    ['device', 'process', 'launch', 'other.bundle']):
            with patch.object(window, 'spawn') as spawn, self.assertRaises(EvidenceError): core.run(cmd)
            spawn.assert_not_called()
        window.close()

    def test_home_fire_is_one_shot_even_when_completion_is_uncertain(self):
        window = self.admit(); home = adapters.PrearmedHome(window)
        stdin = io.BytesIO()
        home.item = {'process': SimpleNamespace(stdin=stdin), 'group': 123}
        armed = {'armed': True, 'nonce': home.nonce, 'device': self.config['device'],
                 'bundle_id': self.config['bundle_id'], 'build': self.config['build']}
        proof = {'candidate': {'apple_build': self.config['build']}, 'replacement': True, 'pid': 102}
        with patch.object(window, 'wait', side_effect=TimeoutError('uncertain')):
            with self.assertRaises(TimeoutError): home.fire(armed, proof)
        self.assertEqual(len(stdin.getvalue().splitlines()), 1)
        with self.assertRaises(EvidenceError): home.fire(armed, proof)
        window.close()


if __name__ == '__main__': unittest.main()
