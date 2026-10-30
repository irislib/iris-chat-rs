"""Synthetic warm lifecycle contracts; no device, push, build or CPU work."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import ios_warm_lifecycle as warm
import ios_relaunch_lifecycle as life
from ios_ordinary_relaunch import EvidenceError
import test_ios_relaunch_lifecycle as logs
from test_ios_ordinary_relaunch import DistributionFixture


def fixture():
    changes = [(0, life.ACTIVE), (61, warm.native.BACKGROUND), (62, life.SUSPENDED), (199, life.ACTIVE)]
    value = {'startup_text': logs.HEADER, 'lifecycle_text': logs.HEADER + ''.join(logs.state(state, t) for t, state in changes),
             'device': logs.DEVICE, 'process_name': 'Chat', 'samples': []}
    initial = {name: logs.binding(logs.HEADER) for name in ('startup', 'lifecycle')}
    times = [*range(0, 61, 5), 62, *range(67, 198, 5), 199, 204, 209, 214]
    for stamp in times:
        row = {'observed_monotonic': stamp, 'pid': logs.PID, 'executable': logs.EXECUTABLE, 'loggers_alive': True, 'stderr': ''}
        for name, text in [('startup', logs.HEADER), ('lifecycle', logs.HEADER + ''.join(logs.state(state, t) for t, state in changes if t <= stamp))]:
            row.update({name + '_' + k: v for k, v in logs.binding(text).items()})
        value['samples'].append(row)
    logs.bind_cutoff(value)
    return value, initial, times.index(60), times.index(197)


def evaluate(value, initial, switch_index, resume_index):
    return warm.evaluate_lifecycle(value, initial, switch_index, resume_index, bundle=logs.BUNDLE, board_pid=1,
                                   proof={'pid': logs.PID, 'executable': logs.EXECUTABLE})


class WarmLifecycleTests(unittest.TestCase):
    def reject(self, change):
        value, initial, switch_index, resume_index = fixture()
        change(value, initial)
        logs.bind_cutoff(value)
        with self.assertRaises((EvidenceError, life.LifecycleError)):
            evaluate(value, initial, switch_index, resume_index)

    def test_ordinary_three_phase_observation_requires_separate_state_followup(self):
        args = fixture(); before = copy.deepcopy(args)
        result = evaluate(*args)
        self.assertEqual(args, before)
        self.assertEqual([result[k + '_seconds'] for k in ('foreground', 'suspended', 'resumed')], [60, 135, 15])
        self.assertFalse(result['physical_release_acceptance'])
        self.assertTrue(result['preserved_state_followup_required'])

    def test_missing_or_reversed_phase_boundaries(self):
        value, initial, switch, resume = fixture()
        for a, b in [(0, resume), (switch, switch), (resume, switch), (switch, len(value['samples']) - 1), (True, resume)]:
            with self.assertRaises(EvidenceError): evaluate(value, initial, a, b)

    def test_foreground_flash_between_background_samples_fails(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 62),
            logs.state(life.SUSPENDED, 62) + logs.state(life.ACTIVE, 63) + logs.state(life.SUSPENDED, 64)))

    def test_pre_suspension_foreground_flash_fails(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(warm.native.BACKGROUND, 61),
            logs.state(warm.native.BACKGROUND, 61) + logs.state(life.ACTIVE, 61.1) + logs.state(warm.native.BACKGROUND, 61.2)))

    def test_early_foreground_interruption_fails(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.ACTIVE, 0),
            logs.state(life.ACTIVE, 0) + logs.state(warm.native.BACKGROUND, 1) + logs.state(life.ACTIVE, 2)))

    def test_resume_interruption_fails(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.ACTIVE, 199),
            logs.state(life.ACTIVE, 199) + logs.state(warm.native.BACKGROUND, 200) + logs.state(life.ACTIVE, 201)))

    def test_inventory_replacement_or_logger_error_fails(self):
        for change in ({'pid': 103}, {'executable': '/other'}, {'loggers_alive': False}, {'stderr': 'error'}):
            self.reject(lambda v, _, c=change: v['samples'][5].update(c))

    def test_state_replacement_and_springboard_change_fail(self):
        for old, new in [('for 102 ', 'for 103 '), ('Services)[1]', 'Services)[2]')]:
            self.reject(lambda v, _, a=old, b=new: logs.replace_capture(v, 'lifecycle', a, b))

    def test_all_three_device_duration_boundaries_are_required(self):
        for old, new in [(logs.state(warm.native.BACKGROUND, 61), logs.state(warm.native.BACKGROUND, 59)),
                         (logs.state(life.ACTIVE, 199), logs.state(life.ACTIVE, 196))]:
            self.reject(lambda v, _, a=old, b=new: logs.replace_capture(v, 'lifecycle', a, b))

    def test_live_host_intervals_are_not_inferred_from_device_timestamps(self):
        for index in (12, 40, -1):
            self.reject(lambda v, _, i=index: v['samples'][i].update(observed_monotonic=v['samples'][i]['observed_monotonic'] - 1))

    def test_sample_gap_and_reversed_clock_fail(self):
        for offset in (-6, 4):
            self.reject(lambda v, _, n=offset: v['samples'][6].update(observed_monotonic=v['samples'][6]['observed_monotonic'] + n))

    def test_warm_restoration_or_wrong_process_marker_fails(self):
        for text in [logs.startup('restore_enqueued', 1, pending=True), logs.startup('backgrounded', 1, background=True, pid=103)]:
            self.reject(lambda v, _, text=text: logs.replace_capture(v, 'startup', logs.HEADER, logs.HEADER + text))

    def test_raw_capture_mutation_and_disconnect_fail(self):
        self.reject(lambda v, _: v.update(lifecycle_text=v['lifecycle_text'].replace('for 102', 'for 103', 1)))
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 62),
            logs.state(life.SUSPENDED, 62) + '[disconnected:' + logs.DEVICE + ']\n'))

    def test_terminal_transition_cannot_be_hidden_after_cutoff(self):
        value, initial, switch, resume = fixture()
        value['lifecycle_text'] += logs.state(warm.native.BACKGROUND, 215)
        with self.assertRaises(life.LifecycleError): evaluate(value, initial, switch, resume)

    def test_adapter_rejects_replacement_and_other_commands_before_spawning(self):
        core = warm.WarmCoreDevice(SimpleNamespace(config={'bundle_id': logs.BUNDLE}))
        for command in [['device', 'process', 'launch', '--terminate-existing', logs.BUNDLE],
                        ['device', 'process', 'terminate', '--pid', '102'], ['device', 'process', 'launch', 'other']]:
            with self.assertRaises(EvidenceError): core.run(command)


class WarmReplayTests(unittest.TestCase):
    def setUp(self):
        # Raw-frame decoding/replay is independently qualified in the trace
        # adapter suite. These tests isolate the warm controller's binding and
        # mandatory invocation, without fabricating physical transport evidence.
        self.trace_replay = patch.object(warm.trace, 'validate_evidence').start()
        self.addCleanup(patch.stopall)
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.f = f = DistributionFixture(temporary.name)
        capture, initial, switch, resume = fixture()
        logs.replace_capture(capture, 'lifecycle', logs.BUNDLE, f.config['bundle_id'])
        for name in ('startup', 'lifecycle'):
            logs.replace_capture(capture, name, logs.DEVICE, f.config['device'])
            initial[name] = logs.binding('[connected:' + f.config['device'] + ']\n')
        for sample in capture['samples']: sample['executable'] = f.executable
        capture['device'] = f.config['device']; capture['process_name'] = 'Example'
        logs.bind_cutoff(capture)
        capture['closure_binding'] = f.write('closure', {'closed': True,
            'measurement_cutoff_sha256': life.evidence_sha256(capture['measurement_cutoff'])})
        window = {**{k: f.config[k] for k in ('device', 'coredevice_id', 'bundle_id')}, 'candidate': f.candidate,
                  'build': f.candidate['apple_build'], 'process_name': 'Example', 'executable': f.executable,
                  'distribution_binding': f.config_binding}
        config = {'window': f.write('window', window), 'installed': f.write('apps', {'result': {'apps': [f.installed]}}),
                  'seed_launch': f.write('seed', {'result': f.result(replacement=False)}),
                  'transport': f.write('synthetic-transport', {'fixture': True}),
                  'qualification': f.write('qualified', {'passed': True, 'sources': warm.sources()})}
        operations = []
        for index, (bundle, begin, end) in enumerate([(warm.native.SETTINGS, -3, -2.9), (f.config['bundle_id'], -1, -.9),
                (warm.native.SETTINGS, 60.1, 60.2), (f.config['bundle_id'], 197.1, 197.2)]):
            result = f.result(replacement=False)
            if bundle == warm.native.SETTINGS:
                result['process'].update(processIdentifier=50, executable='file:///Settings', auditToken=[0, 501, 501, 501, 501, 50, 0, 22])
            operations.append({'command': ['device', 'process', 'launch', bundle], 'result': result,
                'raw': f.write('operation-' + str(index), {'result': result}), 'started_monotonic': begin, 'returned_monotonic': end,
                'started_utc': begin + 1000, 'returned_utc': end + 1000})
        inventory = f.write('processes', {'result': {'runningProcesses': [
            {'processIdentifier': logs.PID, 'executable': f.executable},
            {'processIdentifier': 1, 'executable': 'file:///SpringBoard.app/SpringBoard'}]}})
        self.value = {'configuration': f.write('configuration', config), 'checker_sources': warm.sources(), 'automatic_retry': False,
            'trace_evidence': {'transport': config['transport']},
            'process': warm.native.identity(f.result(replacement=False), f.executable), 'operations': operations,
            'capture': capture, 'inventories': [inventory] * len(capture['samples']), 'switch_index': switch,
            'resume_index': resume, 'board_pid': 1, 'initial_prefixes': initial}

    def test_raw_native_provenance_replay_passes(self):
        self.assertEqual(warm.evaluate(self.value)['suspended_seconds'], 135)
        self.trace_replay.assert_called_once_with(self.value['trace_evidence'], self.value['capture'],
            device=self.f.config['device'], bundle=self.f.config['bundle_id'], board_pid=1)

    def test_raw_transport_cannot_be_substituted(self):
        self.value['trace_evidence']['transport'] = self.f.write('different-transport', {})
        with self.assertRaises(EvidenceError): warm.evaluate(self.value)
        self.trace_replay.assert_not_called()

    def test_failed_raw_replay_cannot_be_ignored(self):
        self.trace_replay.side_effect = ValueError('Raw frame mismatch')
        with self.assertRaisesRegex(ValueError, 'Raw frame mismatch'): warm.evaluate(self.value)

    def test_missing_raw_trace_evidence_never_passes(self):
        del self.value['trace_evidence']
        with self.assertRaises(KeyError): warm.evaluate(self.value)

    def change_operation(self, index, change):
        operation = self.value['operations'][index]; change(operation)
        operation['raw'] = self.f.write('changed-operation', {'result': operation['result']})

    def test_actual_audit_change_is_not_same_process_resume(self):
        self.change_operation(3, lambda op: op['result']['process']['auditToken'].__setitem__(7, 23))
        with self.assertRaises(EvidenceError): warm.evaluate(self.value)

    def test_raw_result_must_match_claimed_result(self):
        self.value['operations'][3]['result']['process']['auditToken'][7] = 23
        with self.assertRaises(EvidenceError): warm.evaluate(self.value)

    def test_ordinary_options_and_exact_operation_sequence(self):
        for mutation in [lambda op: op['result']['launchOptions'].update(terminateExistingInstances=True),
                         lambda op: op['result']['launchOptions'].update(arguments=['debug']),
                         lambda op: op.update(command=['device', 'process', 'terminate', '--pid', '102'])]:
            before = copy.deepcopy(self.value)
            self.change_operation(3, mutation)
            with self.assertRaises(EvidenceError): warm.evaluate(self.value)
            self.value = before

    def test_native_actions_must_bound_actual_samples(self):
        self.change_operation(2, lambda op: op.update(started_monotonic=59, returned_monotonic=59.1,
                                                      started_utc=1059, returned_utc=1059.1))
        with self.assertRaises(EvidenceError): warm.evaluate(self.value)

    def test_real_inventory_cannot_be_replaced_with_healthy_sample_flag(self):
        self.value['inventories'][5] = self.f.write('wrong-inventory', {'result': {'runningProcesses': []}})
        with self.assertRaises(EvidenceError): warm.evaluate(self.value)

    def test_failed_or_unbound_closure_fails(self):
        for closure in [{'closed': False}, {'closed': True, 'measurement_cutoff_sha256': 'a' * 64}]:
            self.value['capture']['closure_binding'] = self.f.write('wrong-closure', closure)
            with self.assertRaises(EvidenceError): warm.evaluate(self.value)



class WarmSetupAndRetentionTests(unittest.TestCase):
    def test_setup_notvisible_states_before_first_active_sample_are_not_measured(self):
        value, initial, switch, resume = fixture()
        logs.replace_capture(value, 'lifecycle', logs.state(life.ACTIVE, 0),
            logs.state(warm.native.BACKGROUND, -.2) + logs.state(life.SUSPENDED, -.1) + logs.state(life.ACTIVE, 0))
        result = evaluate(value, initial, switch, resume)
        self.assertEqual(result['foreground_seconds'], 60)

    def test_setup_time_does_not_count_as_foreground(self):
        value, initial, switch, resume = fixture()
        logs.replace_capture(value, 'lifecycle', logs.state(life.ACTIVE, 0),
            logs.state(warm.native.BACKGROUND, -5) + logs.state(life.SUSPENDED, -1) + logs.state(life.ACTIVE, 2))
        with self.assertRaises(EvidenceError): evaluate(value, initial, switch, resume)

    def test_early_notvisible_departure_is_not_counted_as_suspension(self):
        value, initial, switch, resume = fixture()
        logs.replace_capture(value, 'lifecycle', logs.state(life.ACTIVE, 199),
            logs.state(warm.native.BACKGROUND, 196.9) + logs.state(life.ACTIVE, 199))
        with self.assertRaises(EvidenceError): evaluate(value, initial, switch, resume)

    def test_resume_transition_may_precede_active_but_cannot_resuspend(self):
        value, initial, switch, resume = fixture()
        logs.replace_capture(value, 'lifecycle', logs.state(life.ACTIVE, 199),
            logs.state(warm.native.BACKGROUND, 198.9) + logs.state(life.ACTIVE, 199))
        self.assertEqual(evaluate(value, initial, switch, resume)['resumed_seconds'], 15)
        logs.replace_capture(value, 'lifecycle', logs.state(warm.native.BACKGROUND, 198.9),
            logs.state(warm.native.BACKGROUND, 198.9) + logs.state(life.SUSPENDED, 198.95))
        with self.assertRaises(EvidenceError): evaluate(value, initial, switch, resume)

    def retention(self, outcome):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        window = SimpleNamespace(out=Path(temporary.name), config={'device': logs.DEVICE})
        captured, initial, switch, resume = fixture()
        record = {'samples': captured['samples'], 'passed': False}
        capture = SimpleNamespace(final_capture=lambda samples: copy.deepcopy(captured), trace_evidence={'actual_bound_trace': 'fixture'})
        with patch.object(warm, 'evaluate', side_effect=outcome):
            if isinstance(outcome, Exception):
                with self.assertRaisesRegex(EvidenceError, 'offline failure'): warm.finish_observation(record, capture, window)
            else: warm.finish_observation(record, capture, window)
        retained = json.loads((window.out / 'observed-capture.json').read_text())
        self.assertEqual(retained['capture']['measurement_cutoff'], captured['measurement_cutoff'])
        self.assertEqual(retained['trace_evidence'], capture.trace_evidence)
        self.assertFalse(retained['passed'])
        return window

    def test_checker_failure_retains_actual_capture_and_trace_before_evaluation(self):
        window = self.retention(EvidenceError('offline failure'))
        self.assertFalse((window.out / 'capture.json').exists())

    def test_success_preserves_original_observation_and_separate_pass_receipt(self):
        window = self.retention(lambda record: {'checked': True})
        passed = json.loads((window.out / 'capture.json').read_text())
        self.assertTrue(passed['passed']); self.assertEqual(passed['evaluation'], {'checked': True})

if __name__ == '__main__': unittest.main()
