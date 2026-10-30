#!/usr/bin/env python3
"""Synthetic qualification only: no phone, trace capture or native build."""
import copy
from datetime import datetime, timezone
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch
import xml.etree.ElementTree as ET

import ios_quality_observer as observer
import ios_quality_trace as subject
import ios_relaunch_lifecycle as lifecycle
import ios_runtime_acceptance as contract
from ios_ordinary_relaunch import binding, EvidenceError
from test_ios_ordinary_relaunch import DistributionFixture
from test_ios_relaunch_lifecycle import bind_cutoff, state, BUNDLE
from test_ios_runtime_acceptance import receipt


def table(name, columns, rows):
    root = ET.Element('trace-query-result')
    node = ET.SubElement(root, 'node')
    schema = ET.SubElement(node, 'schema', name=name)
    for column in columns:
        ET.SubElement(ET.SubElement(schema, 'col'), 'mnemonic').text = column
    for values in rows:
        row = ET.SubElement(node, 'row')
        for tag, text, attributes in values:
            ET.SubElement(row, tag, attributes).text = None if text is None else str(text)
    return ET.tostring(root)


class Fixture:
    def __init__(self, root):
        self.root = Path(root)
        self.distribution = DistributionFixture(root)
        self.candidate = self.distribution.candidate
        self.window = {'candidate': self.candidate, 'distribution_binding': self.distribution.config_binding,
                       'bundle_id': self.distribution.config['bundle_id'], 'device': 'synthetic-device',
                       'process_name': 'Example', 'executable': self.distribution.executable}
        self.config = {'schema': 'iris-quality-observer-input-v1', 'pid': 102, 'window': self.write('window', self.window),
                       'qualification': self.write('qualification', {'schema': 'iris-quality-observer-qualification-v1',
                           'passed': True, 'checker_sources': observer.sources()})}
        self.inventory = self.write('inventory', {'result': {'runningProcesses': [
            {'processIdentifier': 102, 'executable': self.window['executable']},
            {'processIdentifier': 1, 'executable': 'file:///system/SpringBoard.app/SpringBoard'}]}})
        apps = {'result': {'apps': [{'bundleIdentifier': self.window['bundle_id'],
                                     'bundleVersion': self.candidate['apple_build'],
                                     'url': self.distribution.config['installed_url']}]}}
        self.epoch = datetime(2026, 1, 1, tzinfo=timezone.utc).timestamp()
        header = '[connected:synthetic-device]\n'
        active = state(lifecycle.ACTIVE, 0).replace(BUNDLE, self.window['bundle_id'])
        self.capture = {'startup_text': header, 'lifecycle_text': header + active, 'samples': []}
        self.observations = []
        for i in range(191):
            t = 100 + i * 2
            sample = {'observed_monotonic': t, 'pid': 102, 'executable': self.window['executable'],
                      'loggers_alive': True, 'stderr': ''}
            for name in ('startup', 'lifecycle'):
                data = self.capture[name + '_text'].encode()
                sample[name + '_bytes'], sample[name + '_sha256'] = len(data), subject.hashlib.sha256(data).hexdigest()
            self.capture['samples'].append(sample)
            self.observations.append({'capture': sample, 'inventory': self.inventory, 'monotonic_before': t - .05,
                                      'monotonic_after': t, 'utc_before': self.epoch + t - 90.05,
                                      'utc_after': self.epoch + t - 90})
        self.evidence = {'schema': 'iris-quality-capture-v1', 'candidate': self.candidate,
                         'configuration': self.write('configuration', self.config), 'checker_sources': observer.sources(),
                         'apps_before': self.write('apps', apps), 'apps_after': self.write('apps-after', apps),
                         'capture': self.capture, 'springboard_pid': 1, 'observations': self.observations,
                         'boundaries': [0, 90, 120, 150, 180],
                         'recording': {'started_monotonic': 89.9, 'started_utc': self.epoch - .1,
                                       'ready_monotonic': 99.8, 'ready_utc': self.epoch + 9.8,
                                       'finished_monotonic': 480, 'finished_utc': self.epoch + 390,
                                       'stdout': self.raw('record.stdout', b'Starting recording with the Activity Monitor template. Targeting All Processes.\nReached specified time limit, ending recording...\nRecording completed. Saving output file...\n'),
                                       'stderr': self.raw('record.stderr', b'')}}
        self.evidence['toc'] = self.raw('toc.xml', self.toc())
        self.evidence['counters'] = self.raw('counters.xml', self.counters())
        self.evidence['thermal'] = self.raw('thermal.xml', self.thermal())

    def raw(self, name, value):
        p = self.root / name
        p.write_bytes(value)
        return binding(p)

    def write(self, name, value):
        return self.raw(name + '.json', (json.dumps(value) + '\n').encode())

    def toc(self, **changes):
        values = {'start-date': '2026-01-01T00:00:00+00:00', 'end-date': '2026-01-01T00:06:30+00:00',
                  'duration': '390', 'template-name': 'Activity Monitor', 'end-reason': 'Time limit reached'}
        values.update(changes)
        root = ET.Element('trace-toc'); run = ET.SubElement(root, 'run'); info = ET.SubElement(run, 'info')
        ET.SubElement(ET.SubElement(info, 'target'), 'device', platform='iOS', uuid='synthetic-device')
        summary = ET.SubElement(info, 'summary')
        for key, value in values.items(): ET.SubElement(summary, key).text = value
        return ET.tostring(root)

    def counters(self, *, cpu=lambda t: t * .02, writes=lambda t: t * 1024, omit=(), changed_process=None):
        rows = []
        for t in range(391):
            if t in omit: continue
            process = ('process', None, {'id': 'p', 'fmt': 'Example (102)'}) if t == 0 else ('process', None, {'ref': 'p'})
            if t == changed_process: process = ('process', None, {'id': 'replacement', 'fmt': 'Example (102)'})
            rows.append([('start-time', t * 10**9, {}), process, ('pid', 102, {}), ('boolean', 0, {}),
                         ('duration-on-core', int(cpu(t) * 10**9), {}), ('duration-on-core', 0, {}),
                         ('size-in-bytes', int(writes(t)), {})])
        return table('sysmon-process', ['time', 'process', 'pid', 'recently-died', 'cpu-total-system', 'cpu-total-user', 'disk-bytes-written'], rows)

    def thermal(self, intervals=((0, 390, 'Nominal', 0),)):
        return table('device-thermal-state-intervals', ['start', 'duration', 'end', 'thermal-state', 'track-label', 'is-induced'],
            [[('start-time', a * 10**9, {}), ('duration', (b-a) * 10**9, {}), ('start-time', b * 10**9, {}),
              ('thermal-state', state, {}), ('string', 'Current', {}), ('boolean', induced, {})]
             for a, b, state, induced in intervals])

    def bind(self):
        bind_cutoff(self.capture)
        closure = {'closed': True, 'failures': [], 'candidate': self.candidate, 'device': self.window['device'],
                   'measurement_cutoff_sha256': lifecycle.evidence_sha256(self.capture['measurement_cutoff'])}
        self.capture['closure_binding'] = self.write('closure', closure)
        return self.write('evidence', self.evidence)


class QualityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.fixture = Fixture(self.temp.name)

    def evaluate(self): return subject.evaluate(self.fixture.bind())

    def reject(self):
        with self.assertRaises((ValueError, KeyError, TypeError, OSError, ET.ParseError)): self.evaluate()

    def test_fixed_windows_and_original_counter_units(self):
        result = self.evaluate()
        self.assertEqual(result['warmup_seconds'], 180)
        self.assertEqual(result['discarded_interval_seconds'], 60)
        self.assertEqual(result['foreground_sample_max_gap_seconds'], 2)
        for window in result['windows']:
            self.assertAlmostEqual(window['cpu_percent_one_core'], 2)
            self.assertAlmostEqual(window['disk_write_mib_per_minute'], 60 / 1024)
            self.assertEqual(window['observed_seconds'], 60)
            self.assertEqual(window['metric_seconds'], 60)
        quality = receipt()['quality']; quality['cpu'] = result
        contract.validate_quality(quality, self.fixture.candidate)

    def test_failed_cpu_window_cannot_be_averaged_away(self):
        f = self.fixture
        f.evidence['counters'] = f.raw('high-cpu.xml', f.counters(cpu=lambda t: t*.02 + min(max(t-250, 0), 60)*.04))
        self.reject()

    def test_disk_write_budget_is_independent(self):
        f = self.fixture
        f.evidence['counters'] = f.raw('high-disk.xml', f.counters(writes=lambda t: t*1024 + min(max(t-250, 0), 60)*1024**2/10))
        self.reject()

    def test_counter_reset_and_same_pid_new_trace_identity_fail(self):
        f = self.fixture
        for data in (f.counters(cpu=lambda t: t*.02 if t < 300 else 0), f.counters(changed_process=300)):
            f.evidence['counters'] = f.raw('bad-counter.xml', data); self.reject()

    def test_truncated_metric_interval_fails_with_full_observer_duration(self):
        f = self.fixture
        f.evidence['counters'] = f.raw('missing-endpoints.xml', f.counters(omit=(250, 251, 310)))
        self.reject()

    def test_missing_process_counters_cannot_hide_behind_normal_host_samples(self):
        f = self.fixture
        f.evidence['counters'] = f.raw('counter-gap.xml', f.counters(omit=range(270, 279)))
        self.reject()

    def test_nominal_thermal_must_cover_every_observed_second(self):
        f = self.fixture
        for intervals in (((0, 300, 'Nominal', 0), (301, 390, 'Nominal', 0)),
                          ((0, 390, 'Fair', 0),), ((0, 390, 'Nominal', 1),), ((0, 350, 'Nominal', 0),)):
            f.evidence['thermal'] = f.raw('bad-thermal.xml', f.thermal(intervals)); self.reject()

    def test_later_quieter_boundary_is_not_selected(self):
        self.fixture.evidence['boundaries'][1] = 91
        self.reject()

    def test_clock_jump_or_foreground_gap_fail(self):
        f = self.fixture
        f.observations[50]['utc_after'] += 1; self.reject()
        f.observations[50]['utc_after'] -= 1
        f.observations[50]['monotonic_before'] -= 3
        f.observations[50]['utc_before'] -= 3; self.reject()

    def test_capture_flag_without_real_inventory_cannot_pass(self):
        f = self.fixture
        for rows in ([], [{'processIdentifier': 103, 'executable': f.window['executable']}],
                     [{'processIdentifier': 102, 'executable': f.window['executable']},
                      {'processIdentifier': 1, 'executable': 'file:///system/SpringBoard.app/SpringBoard'},
                      {'processIdentifier': 900, 'executable': 'file:///test-Runner.app/test'}]):
            f.observations[70]['inventory'] = f.write('bad-inventory', {'result': {'runningProcesses': rows}})
            self.reject()

    def test_intervening_background_and_return_cannot_hide_between_polls(self):
        f = self.fixture
        extra = (state(lifecycle.SUSPENDED, 1) + state(lifecycle.ACTIVE, 2)).replace(BUNDLE, f.window['bundle_id'])
        f.capture['lifecycle_text'] += extra
        data = f.capture['lifecycle_text'].encode()
        for sample in f.capture['samples'][1:]:
            sample.update(lifecycle_bytes=len(data), lifecycle_sha256=subject.hashlib.sha256(data).hexdigest())
        self.reject()

    def test_bad_logger_health_and_changed_prefix_fail(self):
        f = self.fixture
        f.capture['samples'][70]['loggers_alive'] = False; self.reject()
        f.capture['samples'][70]['loggers_alive'] = True
        f.capture['samples'][70]['lifecycle_sha256'] = '0' * 64; self.reject()

    def test_wrong_candidate_and_changed_source_are_rejected(self):
        f = self.fixture
        f.evidence['candidate'] = {**f.candidate, 'apple_build': '2026103100'}; self.reject()
        f.evidence['candidate'] = f.candidate
        f.evidence['checker_sources'] = []; self.reject()

    def test_wrong_trace_target_or_interrupted_recording_fails(self):
        f = self.fixture
        for data in (f.toc(**{'end-reason': 'User stopped'}), f.toc().replace(b'platform="iOS"', b'platform="iOS Simulator"'),
                     f.toc().replace(b'uuid="synthetic-device"', b'uuid="other-device"')):
            f.evidence['toc'] = f.raw('bad-toc.xml', data); self.reject()

    def test_recording_clock_and_stderr_are_validated(self):
        f = self.fixture
        f.evidence['recording']['ready_utc'] += 5; self.reject()
        f.evidence['recording']['ready_utc'] -= 5
        f.evidence['recording']['stderr'] = f.raw('bad-stderr', b'warning'); self.reject()

    def test_rehashed_wrong_installed_build_is_rejected(self):
        f = self.fixture
        apps = {'result': {'apps': [{'bundleIdentifier': f.window['bundle_id'], 'bundleVersion': '2026101800',
                                    'url': f.distribution.config['installed_url']}]}}
        f.evidence['apps_after'] = f.write('wrong-app', apps); self.reject()

    def test_duplicate_unknown_cycle_and_sentinel_trace_values_fail(self):
        f = self.fixture
        data = f.counters()
        variants = (data.replace(b'ref="p"', b'ref="missing"', 1),
                    data.replace(b'ref="p"', b'id="p" ref="p"', 1),
                    data.replace(b'<duration-on-core>0</duration-on-core>', b'<sentinel />', 1))
        for value in variants:
            f.evidence['counters'] = f.raw('invalid-table.xml', value); self.reject()

    def test_readiness_requires_real_output_and_no_diagnostics(self):
        f = self.fixture
        item = {'stdout': Path(f.raw('ready', b'Starting recording with the Activity Monitor template. Targeting All Processes.\nCtrl-C to stop the recording\n')['path']),
                'stderr': Path(f.raw('ready-stderr', b'')['path'])}
        self.assertTrue(observer.ready_text(item))
        item['stderr'].write_text('error')
        with self.assertRaises(EvidenceError): observer.ready_text(item)

    def test_runtime_requires_explicit_execution_before_native_access(self):
        with patch.object(observer, 'OwnedWindow') as window, self.assertRaises(EvidenceError):
            observer.record(self.fixture.write('config', self.fixture.config), self.fixture.root / 'output')
        window.assert_not_called()

    def driver(self, *, replacement=False):
        """Replay real command/output shapes with a deterministic virtual clock."""
        f, now, commands = self.fixture, [0.0], []
        output = f.root / 'run'
        output.mkdir()
        f.config['xctrace'] = f.raw('synthetic-xctrace', b'fixture tool')
        f.window.update(build=f.candidate['apple_build'], coredevice_id='synthetic-coredevice')
        f.config['window'] = f.write('window', f.window)
        config = f.write('configuration', f.config)

        class Window:
            def __init__(self, *_):
                self.out, self.config, self.serial = output, f.window, 0
                self.closed, self.children, self.capture = False, [], None
            def guard(self): pass
            def wait(self, item, seconds):
                if item['process'].poll() != 0: raise ValueError('synthetic incomplete child')
            def spawn(self, argv, label):
                commands.append(argv); self.serial += 1
                stdout, stderr = output / (label+'.stdout'), output / (label+'.stderr')
                stderr.write_bytes(b'')
                if argv[1] == 'record':
                    stdout.write_text('Starting recording with the Activity Monitor template. Targeting All Processes.\nCtrl-C to stop the recording\nReached specified time limit, ending recording...\nRecording completed. Saving output file...\n')
                    child = SimpleNamespace(poll=lambda: 0 if now[0] >= 390 else None)
                else:
                    stdout.write_text('Finished export\n')
                    destination = Path(argv[argv.index('--output') + 1])
                    source = f.toc() if label == 'toc' else (f.counters() if label == 'counters' else f.thermal())
                    destination.write_bytes(source)
                    child = SimpleNamespace(poll=lambda: 0)
                return {'stdout': stdout, 'stderr': stderr, 'process': child}
            def close(self):
                self.closed = True
                if self.capture and self.capture.cutoff:
                    observer.save(output / 'closure.json', {'closed': True, 'failures': [], 'candidate': f.candidate,
                        'device': f.window['device'], 'measurement_cutoff_sha256': lifecycle.evidence_sha256(self.capture.cutoff)})

        class Core:
            def __init__(self, window): self.window = window
            def run(self, argv):
                commands.append(argv); self.window.serial += 1
                if argv[:3] == ['device', 'info', 'apps']:
                    result = json.loads(Path(f.evidence['apps_before']['path']).read_text())['result']
                elif argv == ['device', 'info', 'processes']:
                    result = json.loads(Path(f.inventory['path']).read_text())['result']
                else:
                    result = {'deviceIdentifier': 'synthetic-coredevice',
                              'process': {'processIdentifier': 103 if replacement else 102, 'executable': f.window['executable']},
                              'launchOptions': {'activatedWhenStarted': True, 'terminateExistingInstances': False,
                                  'startStopped': False, 'arguments': [], 'environmentVariables': {},
                                  'platformSpecificOptions': {}, 'user': {'active': True}}}
                observer.save(output / f'coredevice-{self.window.serial:03d}.json', {'result': result})
                return result

        class Capture:
            def __init__(self, window, core):
                self.window, self.core, window.capture = window, core, self
                self.samples, self.cutoff, self.os_process = [], None, {'processIdentifier': 1}
            def start(self): pass
            def health(self): pass
            def prefixes(self): return {key: f.capture[key] for key in ('startup_text', 'lifecycle_text')}
            def sample(self, pid):
                self.core.run(['device', 'info', 'processes'])
                sample = copy.deepcopy(f.capture['samples'][0]); sample['observed_monotonic'] = now[0]
                self.samples.append(sample)
                return sample
            def mark_measurement_cutoff(self, samples):
                value = {**self.prefixes(), 'samples': samples}
                bind_cutoff(value); self.cutoff = value['measurement_cutoff']
            def final_capture(self, samples):
                return {**self.prefixes(), 'samples': samples, 'measurement_cutoff': self.cutoff,
                        'closure_binding': binding(output / 'closure.json')}

        with patch.object(observer, 'OwnedWindow', Window), patch.object(observer, 'CoreDevice', Core), \
             patch.object(observer, 'Capture', Capture), patch.object(observer.time, 'monotonic', side_effect=lambda: now[0]), \
             patch.object(observer.time, 'time', side_effect=lambda: f.epoch + now[0]), \
             patch.object(observer.time, 'sleep', side_effect=lambda n: now.__setitem__(0, now[0] + n)):
            result = observer.record(config, output, execute=True)
        return result, commands

    def test_complete_driver_replay_has_only_one_same_process_activation(self):
        result, commands = self.driver()
        self.assertEqual(result['status'], 'passed')
        launches = [c for c in commands if c[:3] == ['device', 'process', 'launch']]
        self.assertEqual(launches, [['device', 'process', 'launch', self.fixture.window['bundle_id']]])
        trace = next(c for c in commands if c[1] == 'record')
        self.assertIn('--all-processes', trace)
        self.assertEqual(trace[trace.index('--time-limit') + 1], '390s')
        self.assertFalse(any('--terminate-existing' in c or '--console' in c or '--attach' in c for c in commands))

    def test_driver_rejects_new_pid_instead_of_measuring_a_relaunch(self):
        with self.assertRaisesRegex(EvidenceError, 'same process'):
            self.driver(replacement=True)
        self.assertTrue((self.fixture.root / 'run/failure.json').is_file())
        self.assertFalse((self.fixture.root / 'run/cpu.json').exists())


if __name__ == '__main__':
    unittest.main()
