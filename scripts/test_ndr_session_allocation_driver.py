#!/usr/bin/env python3
"""Offline authentic receive admission and shared-stage checks; never Cargo."""
import copy
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_ndr_session_allocation as DRIVER


def inputs():
    data = json.loads(DRIVER.INPUTS.read_text()); data["enabled"] = True
    return data


def output(test, *, allocations=300, failure=None):
    count = test["expected_count"]
    text = f"running {count} tests\ntest {test['name'] or 'integration_case'} ... "
    if test["target"] == DRIVER.TARGET:
        text += f"direct_receive_allocations skipped_keys=1000 allocations={allocations} maximum=500 invalid_hmac=true state_unchanged=true restored_retry=true skipped_recovery=true\n"
    if failure:
        text += f"thread 'test' panicked at source.rs:1: {failure}\nFAILED\n"
    status, passed, failed = ("FAILED", count - 1, 1) if failure else ("ok", count, 0)
    return text + f"test result: {status}. {passed} passed; {failed} failed; 0 ignored; 100 filtered out;\n"


def red(test, allocations=1300):
    return output(test, allocations=allocations, failure=f"direct receive allocated {allocations} times while deriving skipped keys")


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'inputs.json'; path.write_text(json.dumps(data))
            with mock.patch.object(DRIVER, 'INPUTS', path):return DRIVER.contract()

    def test_disabled_blocks_admission(self):
        with self.assertRaisesRegex(AssertionError, 'disabled'):
            self.verify(dict(inputs(), enabled=False))

    def test_exact_forty_two_tests_share_seven_build_targets(self):
        data = self.verify(inputs()); self.assertEqual(sum(t['expected_count'] for t in data['tests']), 42)
        command = DRIVER.ndr.build_command(data['tests'])
        self.assertEqual(command.count('--lib'), 1); self.assertEqual(command.count('--test'), 6)
        self.assertIn('--offline', command); self.assertEqual(command[command.index('--jobs') + 1], '2')

    def test_source_paths_selection_and_limits_fail_closed(self):
        changes = [lambda d: d.update(after=None), lambda d: d.update(after=d['before']), lambda d: d.update(repository='other/repo'),
            lambda d: d['before_changed_paths'].append('rust/crates/nostr-double-ratchet/src/session.rs'),
            lambda d: d['after_changed_paths'].append(DRIVER.FIXTURE), lambda d: d['tests'][0].update(timeout_seconds=600),
            lambda d: d['tests'][0].update(ignored=True), lambda d: d['tests'][0].update(marker=None),
            lambda d: d['tests'][2].update(expected_count=5), lambda d: d['tests'][-1].update(target='other')]
        for change in changes:
            data = inputs(); change(data)
            with self.subTest(change=change), self.assertRaises(AssertionError):self.verify(data)

    def test_new_preflight_uses_case_schema_and_exact_input_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); gh = root / 'github-output'
            with mock.patch.object(DRIVER.ndr, 'git', side_effect=['', 'driver-sha']), \
                mock.patch.object(DRIVER.ndr.time, 'time', return_value=1000), mock.patch.dict(os.environ, GITHUB_OUTPUT=str(gh)):
                DRIVER.run('preflight', None, root, inputs())
            result = json.loads((root / 'preflight.json').read_text())
            self.assertEqual(result['schema'], 'ndr-session-allocation/v1'); self.assertEqual(result['inputs_sha256'], DRIVER.ndr.sha(DRIVER.INPUTS))
            self.assertEqual(result['deadline_unix'], 2500); self.assertEqual(result['workflow_timeout_seconds'], 1800)


class AllocationTests(unittest.TestCase):
    def setUp(self):self.test = inputs()['tests'][0]

    def test_exact_authentication_and_positive_recovery_metric_red_green(self):
        metric, counts = DRIVER.allocation_result(red(self.test), self.test, 101, True)
        self.assertEqual(metric['skipped_keys'], 1000); self.assertTrue(counts['expected_regression_failure'])
        for value in (1, 500):self.assertEqual(DRIVER.allocation_result(output(self.test, allocations=value), self.test, 0, False)[0]['allocations'], value)

    def test_wrong_metric_flags_workload_limit_or_duplicate_fail(self):
        text = red(self.test)
        cases = [text.replace('skipped_keys=1000', 'skipped_keys=999'), text.replace('maximum=500', 'maximum=600'),
            text.replace('allocations=1300', 'allocations=0'), text + text]
        cases += [text.replace(key + '=true', key + '=false') for key in ('invalid_hmac', 'state_unchanged', 'restored_retry', 'skipped_recovery')]
        for candidate in cases:
            with self.subTest(candidate=candidate[:80]), self.assertRaises(AssertionError):DRIVER.allocation_result(candidate, self.test, 101, True)

    def test_compile_zero_tests_unexpected_pass_wrong_assertion_or_exit_are_not_red(self):
        text = red(self.test)
        cases = [('error: could not compile', 101), (text.replace('1 failed', '0 failed'), 101), (output(self.test), 0),
            (text.replace('allocated 1300 times', 'allocated 9999 times'), 101), (text.replace('while deriving skipped keys', 'different panic'), 101), (text, -9)]
        for candidate, code in cases:
            with self.subTest(code=code), self.assertRaises(AssertionError):DRIVER.allocation_result(candidate, self.test, code, True)

    def test_green_over_limit_zero_or_failed_recovery_is_rejected(self):
        for text, code in [(red(self.test), 101), (output(self.test, allocations=501), 0), (output(self.test, allocations=0), 0),
            (output(self.test).replace('restored_retry=true', 'restored_retry=false'), 0), (output(self.test), 101)]:
            with self.subTest(code=code), self.assertRaises(AssertionError):DRIVER.allocation_result(text, self.test, code, False)


class SourceTests(unittest.TestCase):
    def test_case_specific_fixture_path_is_hashed_without_changing_default(self):
        data = inputs(); shared = DRIVER.ndr
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for path in [DRIVER.FIXTURE, *DRIVER.PRODUCTION, 'rust/Cargo.lock', 'rust/Cargo.toml']:
                destination = root / path; destination.parent.mkdir(parents=True, exist_ok=True); destination.write_text(path)
            facts = {('rev-parse', 'HEAD'):data['after'], ('status', '--porcelain'):'', ('rev-parse', 'HEAD^'):data['before'],
                ('rev-parse', 'HEAD^{tree}'):'tree', ('diff', '--name-only', 'HEAD^', 'HEAD'):'\n'.join(DRIVER.PRODUCTION),
                ('ls-files', 'rust/Cargo.lock', 'rust/**/Cargo.toml', 'rust/Cargo.toml'):'rust/Cargo.lock\nrust/Cargo.toml'}
            with mock.patch.object(shared, 'git', side_effect=lambda _, *args:facts[args]):
                proof = shared.source_proof('after', root, data, case=DRIVER.session_case())
            self.assertEqual(proof['allocation_fixture_sha256'], shared.sha(root / DRIVER.FIXTURE))
            self.assertEqual(shared.blind_case()['allocation_path'], shared.ALLOCATION_PATH)
            self.assertEqual(shared.blind_case()['schema'], 'ndr-blind-allocation/v1')


class StageTests(unittest.TestCase):
    def exercise(self, which, *, compile_failure=False, wrong_red=False, failed_green=None):
        data, shared = inputs(), DRIVER.ndr
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); out = root / 'evidence'; target = root / 'target'; out.mkdir(); target.mkdir()
            source = {'commit':data[which], 'parent':data['base'] if which=='before' else data['before'],
                'allocation_fixture_sha256':'same', 'dependency_inputs':{'lock':'same'}}
            versions = {tool:f'{tool} 1.99.1 (test)\n' for tool in ('cargo', 'rustc')}
            shared.save(out / 'preflight.json', {'schema':'ndr-session-allocation/v1', 'inputs':data, 'inputs_sha256':shared.sha(DRIVER.INPUTS), 'toolchain':'1.99.1', 'deadline_unix':time.time()+1500})
            if which=='after':
                (out/'before').mkdir(); shared.save(out/'before/receipt.json', {'accepted':True, 'stage':'before', 'source_path':str(root), 'target_path':str(target), 'source':dict(source, commit=data['before']), 'toolchain':versions})
            selected = data['tests'] if which=='after' else data['tests'][:1]
            artifacts=[]
            for kind,name in dict.fromkeys((t['kind'],t['target']) for t in selected):
                binary=target/name; binary.write_text('immutable executable')
                artifacts.append({'reason':'compiler-artifact','executable':str(binary),'target':{'kind':[kind],'name':name},'profile':{'test':True,'opt_level':'3','debuginfo':1,'debug_assertions':False}})
            calls=[]
            def bounded(command,_root,_out,name,timeout,deadline,_target):
                calls.append((name,command)); self.assertEqual(_target,target); self.assertGreater(deadline,time.time())
                if name=='fetch':
                    self.assertEqual(which,'before');self.assertEqual(timeout,300);return {'exit_code':0},'fetched'
                if name=='build':
                    self.assertEqual(timeout,600);self.assertEqual(command,shared.build_command(selected));return {'exit_code':101 if compile_failure else 0},'\n'.join(map(json.dumps,artifacts))
                index=int(name.split('-')[1]);test=selected[index];self.assertEqual(timeout,300);self.assertEqual(command[0],str(target/test['target']))
                if which=='before':return {'exit_code':101},red(test).replace('while deriving skipped keys','wrong failure') if wrong_red else red(test)
                if index==failed_green:return {'exit_code':101},output(test,failure='unrelated correctness failure')
                return {'exit_code':0},output(test)
            with mock.patch.object(shared,'source_proof',return_value=source), mock.patch.object(shared,'git',return_value=''), \
                mock.patch.object(shared.subprocess,'check_output',side_effect=lambda cmd,**kw:versions[cmd[0]].encode()), \
                mock.patch.object(shared,'bounded',bounded), mock.patch.dict(os.environ,CARGO_TARGET_DIR=str(target)):
                if compile_failure or wrong_red or failed_green is not None:
                    with self.assertRaises(AssertionError):DRIVER.run(which,root,out,data)
                    self.assertFalse((out/which/'receipt.json').exists())
                else:
                    DRIVER.run(which,root,out,data);receipt=json.loads((out/which/'receipt.json').read_text())
                    self.assertEqual(receipt['schema'],'ndr-session-allocation/v1');self.assertTrue(receipt['accepted'])
                    self.assertEqual(sum(t['counts']['passed'] for t in receipt['tests']),42 if which=='after' else 0)
                    self.assertFalse(receipt['physical_cpu_acceptance']);self.assertEqual(len(receipt['test_binaries']),7 if which=='after' else 1)
            executions=[c for name,c in calls if name.startswith('test-')]
            self.assertEqual(len(executions),0 if compile_failure else failed_green+1 if failed_green is not None else len(selected))
            self.assertEqual(len([name for name,_ in calls if name=='build']),1)
            if which=='after' and len(executions)>2:self.assertEqual(executions[1][0],executions[2][0])

    def test_one_red_and_all_forty_two_green_checks_use_reviewed_targets(self):
        self.exercise('before');self.exercise('after')

    def test_compile_failure_and_wrong_red_stop_without_acceptance(self):
        self.exercise('before',compile_failure=True);self.exercise('after',compile_failure=True);self.exercise('before',wrong_red=True)

    def test_each_focused_green_failure_stops_further_execution(self):
        for index in range(len(DRIVER.selected_tests())):
            with self.subTest(index=index):self.exercise('after',failed_green=index)


if __name__=='__main__':unittest.main()
