#!/usr/bin/env python3
"""Synthetic contracts only: no native commands, device, signing or deployment."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

import ios_ordinary_relaunch as relaunch
import ios_runtime_acceptance as contract
from test_ios_runtime_acceptance import CANDIDATE, receipt
import test_ios_relaunch_lifecycle as lifecycle_fixture


class DistributionFixture:
    def __init__(self, root):
        self.root = Path(root)
        self.candidate = dict(CANDIDATE)
        self.inputs = {}
        for name in relaunch.PROVENANCE_INPUTS - {'provenance_qualification'}:
            self.inputs[name] = self.write(name, {'synthetic_evidence': name})
        self.candidate.update(ipa_sha256=self.inputs['official_ipa']['sha256'],
                              manifest_sha256=self.inputs['manifest']['sha256'])
        original = {'uuid': 'synthetic-code-id', 'loadCommandsSHA256': 'a' * 64, 'unsignedContentSHA256': 'b' * 64}
        self.code = {role: {'original': dict(original), 'resigned': dict(original)}
                     for role in ('app', 'notification-service', 'share-extension')}
        self.proof = {**contract.ADHOC_POLICY,
                      **{key: self.inputs[key.removesuffix('_sha256')]['sha256'] for key in relaunch.PROVENANCE_DIGESTS},
                      'ipa_sha256': self.candidate['ipa_sha256'], 'testflight_upgrade_claimed': False,
                      'installed_binary_extraction_claimed': False,
                      'bundle_instruction_sha256': {role: relaunch.sha256(json.dumps(original, sort_keys=True, separators=(',', ':')).encode())
                                                    for role in self.code}}
        self.qualification = {'schema': 'iris-qualified-adhoc-provenance-v1', 'candidate': self.candidate,
                              'inputs': dict(self.inputs), 'production_adhoc': self.proof, 'code_bindings': self.code,
                              'checker_sources': [self.write('synthetic-checker', {'fixture': True})]}
        self.config = {'schema': 'iris-ordinary-relaunch-input-v1', 'candidate': self.candidate,
                       'device': 'synthetic-device', 'coredevice_id': 'synthetic-coredevice', 'bundle_id': 'example.test.app',
                       'installed_url': 'file:///synthetic/Example.app/', 'previous_build': '2026101800',
                       'installation_finished_utc': 1, 'production_adhoc': self.proof,
                       'inputs': self.inputs, 'code_bindings': self.code}
        self.qualification['target'] = {k: self.config[k] for k in relaunch.TARGET_KEYS}
        review = {key: self.qualification[key] for key in ('candidate', 'target', 'inputs', 'checker_sources', 'code_bindings')}
        review.update(schema='iris-adhoc-provenance-review-v1', decision='accepted')
        self.qualification['review'] = self.write('synthetic-review', review)
        self.inputs['provenance_qualification'] = self.write('provenance-qualification', self.qualification)
        self.config_binding = self.write('distribution', self.config)
        self.executable = 'file:///synthetic/Example.app/Example'
        self.installed = {'bundleIdentifier': self.config['bundle_id'], 'bundleVersion': self.candidate['apple_build'],
                          'url': self.config['installed_url']}

    def write(self, name, value):
        path = self.root / (name + '.json')
        path.write_text(json.dumps(value, sort_keys=True))
        return relaunch.binding(path)

    def load(self):
        return relaunch.validate_distribution(self.config_binding, self.candidate)

    def snapshot(self, pid, at=10):
        return {'observed_utc': at, 'processes': [{'pid': pid, 'executable': self.executable}], 'foreground_pid': pid}

    def result(self, pid=102, replacement=True):
        return {'deviceIdentifier': self.config['coredevice_id'],
                'launchOptions': {'activatedWhenStarted': True, 'arguments': [], 'environmentVariables': {'TERM': 'dumb'},
                                  'platformSpecificOptions': {}, 'startStopped': False,
                                  'terminateExistingInstances': replacement, 'user': {'active': True}},
                'process': {'processIdentifier': pid, 'executable': self.executable,
                            'auditToken': [0, 501, 501, 501, 501, pid, 0, 22]}}

    def launch_proof(self, **changes):
        args = {'distribution': self.load(), 'installed': self.installed, 'result': self.result(),
                'snapshot': self.snapshot(102, 10.3), 'command': relaunch.launch_command(self.config['bundle_id'], replacement=True),
                'started': 10, 'returned': 10.2, 'validated': 10.4, 'executable': self.executable,
                'before': self.snapshot(101, 9.9)}
        args.update(changes)
        return relaunch.validate_launch(**args)


class RelaunchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.fixture = DistributionFixture(self.temp.name)

    def reject_config(self, mutate):
        value = copy.deepcopy(self.fixture.config)
        mutate(value)
        b = self.fixture.write('invalid', value)
        with self.assertRaises((relaunch.EvidenceError, contract.ValidationError, OSError)):
            relaunch.validate_distribution(b, self.fixture.candidate)

    def test_independently_bound_provenance_loads(self):
        self.assertEqual(self.fixture.load(), self.fixture.config)

    def test_missing_or_tampered_raw_input_fails(self):
        p = Path(self.fixture.inputs['installation']['path'])
        p.write_text('{}')
        with self.assertRaises(relaunch.EvidenceError): self.fixture.load()
        p.unlink()
        with self.assertRaises(relaunch.EvidenceError): self.fixture.load()

    def test_candidate_and_all_code_roles_must_match(self):
        mutations = [lambda d: d['candidate'].update(source_commit='f' * 40),
                     lambda d: d['production_adhoc'].update(signing='development'),
                     lambda d: d['production_adhoc'].update(testflight_upgrade_claimed=True),
                     lambda d: d['code_bindings']['app']['resigned'].update(uuid='changed'),
                     lambda d: d['code_bindings']['notification-service']['resigned'].update(loadCommandsSHA256='f' * 64),
                     lambda d: d['code_bindings'].pop('share-extension'),
                     lambda d: d.update(previous_build=d['candidate']['apple_build']),
                     lambda d: d['inputs'].pop('profiles'),
                     lambda d: d.update(device='other-physical-phone'),
                     lambda d: d.update(coredevice_id='other-coredevice-id'),
                     lambda d: d['production_adhoc'].update(installation_sha256='f' * 64),
                     lambda d: d.update(installed_url='https://example.invalid/app')]
        for mutation in mutations:
            with self.subTest(mutation=mutation): self.reject_config(mutation)

    def test_rehashed_but_unqualified_configuration_rejected(self):
        self.reject_config(lambda d: d['production_adhoc'].update(authorization_sha256='f' * 64))
        def change_qualified_code(d):
            for side in ('original', 'resigned'):
                d['code_bindings']['app'][side]['uuid'] = 'different-but-matching'
            encoded = json.dumps(d['code_bindings']['app']['original'], sort_keys=True, separators=(',', ':')).encode()
            d['production_adhoc']['bundle_instruction_sha256']['app'] = relaunch.sha256(encoded)
        self.reject_config(change_qualified_code)

    def test_qualification_checker_and_review_bytes_remain_bound(self):
        Path(self.fixture.qualification['checker_sources'][0]['path']).write_text('{}')
        with self.assertRaises(relaunch.EvidenceError): self.fixture.load()

    def test_ordinary_replacement_has_fresh_process(self):
        proof = self.fixture.launch_proof()
        self.assertEqual(proof['previous_pid'], 101)
        self.assertEqual(proof['pid'], 102)
        self.assertTrue(proof['debugger_free'])

    def test_cold_absence_and_same_process_are_not_replacement_route(self):
        for before in ({'observed_utc': 9.9, 'processes': [], 'foreground_pid': None},
                       self.fixture.snapshot(102, 9.9),
                       {**self.fixture.snapshot(101, 9.9), 'foreground_pid': None}):
            with self.subTest(before=before), self.assertRaises(relaunch.EvidenceError):
                self.fixture.launch_proof(before=before)

    def test_launch_options_cannot_create_debugger_or_identity_loopholes(self):
        mutations = {'startStopped': True, 'arguments': ['--fake'], 'environmentVariables': {'DEVICECTL_CHILD_FAKE': '1'},
                     'platformSpecificOptions': {'suspend': True}, 'terminateExistingInstances': False,
                     'activatedWhenStarted': False, 'user': {'active': False}}
        for key, value in mutations.items():
            result = self.fixture.result(); result['launchOptions'][key] = value
            with self.subTest(key=key), self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(result=result)
        result = self.fixture.result(); result['launchOptions']['consoleAttached'] = True
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(result=result)
        command = relaunch.launch_command(self.fixture.config['bundle_id'], replacement=True) + ['--console']
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(command=command)

    def test_wrong_target_install_and_process_inventory_fail(self):
        for key, value in [('deviceIdentifier', 'another-device')]:
            result = self.fixture.result(); result[key] = value
            with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(result=result)
        installed = dict(self.fixture.installed, bundleVersion='2026102000')
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(installed=installed)
        result = self.fixture.result(); result['process']['auditToken'][5] = 999
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(result=result)
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(snapshot=self.fixture.snapshot(103, 10.3))
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(executable='file:///different/Example')

    def test_launch_and_inventory_freshness_not_extended_for_slow_home(self):
        for delta in ({'validated': 18.2001}, {'returned': 9.9}, {'started': float('nan')},
                      {'before': self.fixture.snapshot(101, 1)}, {'snapshot': self.fixture.snapshot(102, 10.1)}):
            with self.subTest(delta=delta), self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(**delta)

    def test_resume_requires_original_pid_and_real_audit_token(self):
        first = self.fixture.launch_proof()
        args = {'before': None, 'resume_of': first, 'result': self.fixture.result(replacement=False),
                'command': relaunch.launch_command(self.fixture.config['bundle_id'], replacement=False)}
        resumed = self.fixture.launch_proof(**args)
        self.assertEqual(resumed['audit_token'], first['audit_token'])
        changed = self.fixture.result(replacement=False); changed['process']['auditToken'][-1] += 1
        with self.assertRaises(relaunch.EvidenceError): self.fixture.launch_proof(**{**args, 'result': changed})

    def controller(self):
        return relaunch.OrdinaryForegroundRelaunch(self.fixture.config_binding, self.fixture.candidate,
                                                  self.fixture.installed, self.fixture.executable, clock=lambda: 10)

    def callbacks(self):
        f = self.fixture
        snapshots = iter([f.snapshot(101), f.snapshot(102)])
        calls = []
        return {'snapshot': lambda: next(snapshots),
                'prearm': lambda: {'armed': True, 'nonce': 'synthetic-nonce-0001', 'device': f.config['device'],
                                  'bundle_id': f.config['bundle_id'], 'build': f.candidate['apple_build']},
                'capture_prefixes': lambda: {'startup_text': '[connected:synthetic-device]\n',
                                             'lifecycle_text': '[connected:synthetic-device]\n'},
                'launch': lambda command: (calls.append(('launch', command)) or f.result()),
                'trigger': lambda armed, proof: (calls.append(('trigger', proof['pid'])) or
                    {'nonce': armed['nonce'], 'build': armed['build'], 'pid': proof['pid'], 'sent_count': 1, 'acknowledged': True})}, calls

    def test_one_prearmed_launch_and_trigger_are_never_lifecycle_acceptance(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        record = controller.run_once(**callbacks)
        self.assertTrue(record['completed']); self.assertFalse(record['physical_acceptance'])
        self.assertEqual([call[0] for call in calls], ['launch', 'trigger'])
        with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)

    def test_valid_raw_launch_triggers_before_postlaunch_inventory(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        original = callbacks['snapshot']
        callbacks['snapshot'] = lambda: (calls.append(('snapshot',)) or original())
        record = controller.run_once(**callbacks)
        self.assertEqual([call[0] for call in calls], ['snapshot', 'launch', 'trigger', 'snapshot'])
        self.assertFalse(record['launch_result_proof']['inventory_verified'])
        self.assertTrue(record['launch_proof']['inventory_verified'])

    def test_bad_actual_launch_result_prevents_trigger(self):
        for mutate in (lambda d: d.update(deviceIdentifier='wrong'),
                       lambda d: d['launchOptions'].update(startStopped=True),
                       lambda d: d['process'].update(processIdentifier=101),
                       lambda d: d['process'].update(executable='file:///wrong/Example')):
            controller = self.controller(); callbacks, calls = self.callbacks()
            result = self.fixture.result(); mutate(result)
            callbacks['launch'] = lambda _: result
            with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)
            self.assertEqual(controller.record['trigger_count'], 0)
            self.assertFalse(any(call[0] == 'trigger' for call in calls))

    def test_posttrigger_inventory_failure_is_preserved_without_retry(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        snapshots = iter([self.fixture.snapshot(101), self.fixture.snapshot(103)])
        callbacks['snapshot'] = lambda: next(snapshots)
        with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)
        self.assertEqual(controller.record['trigger_count'], 1)
        self.assertIn('failure', controller.record)
        self.assertFalse(controller.record['completed'])
        with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)

    def test_uncertain_launch_or_trigger_failure_cannot_retry(self):
        for failure in ('launch', 'trigger'):
            controller = self.controller(); callbacks, calls = self.callbacks()
            def fail(*_): raise TimeoutError('synthetic uncertain outcome')
            callbacks[failure] = fail
            with self.subTest(failure=failure), self.assertRaises(TimeoutError): controller.run_once(**callbacks)
            self.assertFalse(controller.record['completed'])
            with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)

    def test_arming_mismatch_prevents_app_action(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        callbacks['prearm'] = lambda: {'armed': True, 'nonce': 'synthetic-nonce-0001', 'device': 'wrong',
                                      'bundle_id': self.fixture.config['bundle_id'], 'build': self.fixture.candidate['apple_build']}
        with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)
        self.assertEqual(calls, [])

    def runtime_fixture(self):
        f = self.fixture
        attempt = relaunch.OrdinaryForegroundRelaunch(f.config_binding, f.candidate, f.installed, f.executable,
                                                     clock=lambda: 10, monotonic=lambda: -1)
        callbacks, _ = self.callbacks()
        record = attempt.run_once(**callbacks)
        capture = lifecycle_fixture.fixture()
        lifecycle_fixture.replace_capture(capture, 'startup', ' Chat[', ' Example[')
        lifecycle_fixture.replace_capture(capture, 'lifecycle', lifecycle_fixture.BUNDLE, f.config['bundle_id'])
        for sample in capture['samples']: sample['executable'] = f.executable
        capture = {key: capture[key] for key in ('startup_text', 'lifecycle_text', 'samples')}
        capture['process_name'] = 'Example'
        capture['closure_binding'] = f.write('capture-closure', {'schema': 'iris-relaunch-adapter-closure-v1',
            'candidate': f.candidate, 'device': f.config['device'], 'closed': True, 'failures': [], 'groups': [1001, 1002, 1003]})
        resume = {'result': f.result(replacement=False), 'snapshot': f.snapshot(102, 147.85),
                  'command': relaunch.launch_command(f.config['bundle_id'], replacement=False),
                  'started_utc': 147.5, 'returned_utc': 147.8, 'validated_utc': 147.9,
                  'started_monotonic': 136.5, 'returned_monotonic': 136.8}
        return record, resume, capture

    def test_full_raw_evidence_replay_requires_same_resume_and_real_overlap(self):
        record, resume, capture = self.runtime_fixture()
        evaluated = self.evaluate(record, resume, capture)
        self.assertEqual(evaluated['lifecycle']['suspended_seconds'], 135)
        self.assertFalse(evaluated['physical_release_acceptance'])
        wrong = copy.deepcopy(resume); wrong['result']['process']['auditToken'][-1] += 1
        with self.assertRaises(relaunch.EvidenceError):
            self.evaluate(record, wrong, capture)
        wrong = copy.deepcopy(resume); wrong['started_monotonic'] = 20
        with self.assertRaises(relaunch.EvidenceError):
            self.evaluate(record, wrong, capture)
        wrong = copy.deepcopy(record); wrong['launch_proof']['pid'] = 103
        with self.assertRaises(relaunch.EvidenceError):
            self.evaluate(wrong, resume, capture)
        wrong = copy.deepcopy(capture)
        lifecycle_fixture.replace_capture(wrong, 'startup', 'native_restoring_observed=true', 'native_restoring_observed=false')
        with self.assertRaises(relaunch.lifecycle.LifecycleError):
            self.evaluate(record, resume, wrong)

    def evaluate(self, attempt, resume, capture):
        return relaunch.evaluate_runtime(self.fixture.config_binding, self.fixture.candidate, attempt, resume, capture)

    def test_delayed_capture_stops_before_native_launch(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        clock = iter([10, 19])
        controller.clock = lambda: next(clock)
        with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)
        self.assertEqual(calls, [])
        self.assertEqual(controller.record['launch_count'], 0)

    def test_returned_trigger_failure_never_becomes_completion(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        callbacks['trigger'] = lambda *_: {'passed': False, 'failure': 'synthetic timeout'}
        with self.assertRaises(relaunch.EvidenceError): controller.run_once(**callbacks)
        self.assertFalse(controller.record['completed'])
        record, resume, capture = self.runtime_fixture()
        record['failure'] = 'preserved first failure'
        with self.assertRaises(relaunch.EvidenceError): self.evaluate(record, resume, capture)

    def test_replay_revalidates_armed_target_even_when_acknowledgement_matches(self):
        record, resume, capture = self.runtime_fixture()
        for change in ('build', 'device', 'bundle_id', 'armed'):
            wrong = copy.deepcopy(record)
            wrong['armed'][change] = False if change == 'armed' else 'another-target'
            if change == 'build': wrong['trigger_result']['build'] = 'another-target'
            with self.subTest(change=change), self.assertRaises(relaunch.EvidenceError): self.evaluate(wrong, resume, capture)

    def test_provenance_mutation_during_observation_is_rejected(self):
        record, resume, capture = self.runtime_fixture()
        Path(self.fixture.inputs['profiles']['path']).write_text('{}')
        with self.assertRaises(relaunch.EvidenceError): self.evaluate(record, resume, capture)

    def test_runtime_cannot_skip_failed_process_group_closure(self):
        record, resume, capture = self.runtime_fixture()
        value = json.loads(Path(capture['closure_binding']['path']).read_text())
        value.update(closed=False, failures=['owned logger remains'])
        capture['closure_binding'] = self.fixture.write('failed-closure', value)
        with self.assertRaises(relaunch.EvidenceError): self.evaluate(record, resume, capture)

    def test_stale_resume_with_relabelled_monotonic_times_is_rejected(self):
        record, resume, capture = self.runtime_fixture()
        resume.update(started_utc=11, returned_utc=11.2, validated_utc=11.3)
        resume['snapshot']['observed_utc'] = 11.25
        with self.assertRaises(relaunch.EvidenceError): self.evaluate(record, resume, capture)

    def test_composition_replays_raw_evidence_and_requires_fresh_helper_qualification(self):
        from qualify_ios_ordinary_relaunch import SOURCES, TESTS
        record, resume, capture = self.runtime_fixture()
        raw = self.fixture.write('runtime-capture', {'schema': 'iris-ordinary-relaunch-capture-v1',
                                                   'attempt': record, 'resume': resume, 'capture': capture})
        scripts = Path(__file__).resolve().parent
        qualification = {'schema': 'iris-ordinary-relaunch-qualification-v1', 'synthetic_only': True,
                         'sources_unchanged': True, 'device_actions': False, 'native_builds': False,
                         'release_acceptance': False, 'exit_code': 0, 'sources': [relaunch.binding(scripts / name) for name in SOURCES],
                         'tests': list(TESTS), 'tagged_validator': relaunch.binding(scripts / 'ios_runtime_acceptance.py'),
                         'tests_log': self.fixture.write('synthetic-test-log', {'fixture': True}),
                         'provenance_qualification': self.fixture.inputs['provenance_qualification']}
        qualified = self.fixture.write('helper-qualification', qualification)
        proof = relaunch.compose_adhoc_proof(self.fixture.config_binding, self.fixture.candidate,
                                            normal_launch_binding=raw, qualification_binding=qualified)
        contract.validate_adhoc_proof(proof, self.fixture.candidate)
        self.assertEqual(proof['normal_launch_sha256'], raw['sha256'])
        for mutate in (lambda d: d.update(provenance_qualification=None), lambda d: d.update(exit_code=1),
                       lambda d: d.update(sources=[]), lambda d: d.update(release_acceptance=True),
                       lambda d: d.update(tests=['some-other-test'])):
            invalid = copy.deepcopy(qualification); mutate(invalid)
            rejected = self.fixture.write('bad-helper-qualification', invalid)
            with self.assertRaises(relaunch.EvidenceError):
                relaunch.compose_adhoc_proof(self.fixture.config_binding, self.fixture.candidate,
                                            normal_launch_binding=raw, qualification_binding=rejected)

    def test_log_readiness_is_required_before_launch(self):
        controller = self.controller(); callbacks, calls = self.callbacks()
        callbacks['capture_prefixes'] = lambda: {'startup_text': '', 'lifecycle_text': ''}
        with self.assertRaises(relaunch.lifecycle.LifecycleError): controller.run_once(**callbacks)
        self.assertEqual(calls, [])

    def test_safe_receipt_still_uses_tagged_v3_requirements(self):
        safe = receipt(contract.ADHOC_CHANNEL)
        safe['release'] = dict(self.fixture.candidate)
        safe['installation']['production_adhoc'] = {**self.fixture.proof,
                                                   'normal_launch_sha256': 'c' * 64, 'qualification_sha256': 'd' * 64}
        contract.validate_receipt(safe, **self.fixture.candidate)
        for mutate in (lambda d: d['checks']['early_background'].update(restore_in_flight=False),
                       lambda d: d['checks']['early_background'].update(observed_seconds=134.999),
                       lambda d: d['checks']['early_background'].update(process_replacements=1),
                       lambda d: d['checks']['cold_background_launch'].update(foreground_activations=1),
                       lambda d: d['installation']['production_adhoc'].pop('profiles_sha256')):
            invalid = copy.deepcopy(safe); mutate(invalid)
            with self.assertRaises(contract.ValidationError): contract.validate_receipt(invalid, **self.fixture.candidate)


if __name__ == '__main__': unittest.main()
