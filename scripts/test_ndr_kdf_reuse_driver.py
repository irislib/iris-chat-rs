#!/usr/bin/env python3
"""Offline admission, measurement parsing and fail-closed execution checks."""
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock

import verify_ndr_kdf_reuse as D


def inputs():
    value = json.loads(D.INPUTS.read_text()); value['enabled'] = True
    return value


def output(test, *, red=False, failure=None):
    count = test['expected_count']
    text = f"running {count} tests\ntest {test['name'] or 'integration_case'} ... \n"
    if test['name'] == D.TEST:
        text += f"blind_kdf_initializations trials=256 initializations={256 if red else 1} maximum=1 invalid_hmac=true state_unchanged=true reference_state=true\n"
        if red and failure is None:failure = D.FAILURE
    if failure:text += f"thread 'test' panicked at source.rs:1: {failure}\n"
    return text + f"test result: {'FAILED' if failure else 'ok'}. {count-1 if failure else count} passed; {1 if failure else 0} failed; 0 ignored; 100 filtered out;\n"


def samples(test, seconds=0.5):
    return ''.join(f'blind_kdf_benchmark sample={i} searches=16 trials=160016 budget=256 ciphertext_bytes=1091 elapsed_seconds={seconds+i/100} invalid_hmac=true state_unchanged=true\n' for i in range(7)) + output(test)


class ContractTests(unittest.TestCase):
    def verify(self, data):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'inputs.json';path.write_text(json.dumps(data))
            with mock.patch.object(D,'INPUTS',path):return D.contract()

    def test_disabled_and_exact_thirty_tests_four_targets(self):
        with self.assertRaisesRegex(AssertionError,'disabled'):self.verify(dict(inputs(),enabled=False))
        data=self.verify(inputs());self.assertEqual(sum(t['expected_count'] for t in data['tests']),30)
        command=D.ndr.build_command(data['tests'])
        self.assertEqual(command.count('--lib'),1);self.assertEqual(command.count('--test'),3)
        self.assertIn('--offline',command);self.assertEqual(command[command.index('--jobs')+1],'2')

    def test_source_scope_work_and_limits_fail_closed(self):
        changes=[lambda d:d.update(after='f'*40),lambda d:d.update(repository='other/repo'),
            lambda d:d['before_changed_paths'].append('extra'),lambda d:d['after_changed_paths'].append(D.FIXTURE),
            lambda d:d['tests'][0].update(expected_count=2),lambda d:d['tests'][1].update(timeout_seconds=600),
            lambda d:d['benchmark'].update(samples=9),lambda d:d['benchmark'].update(trials_per_sample=100),
            lambda d:d['benchmark'].update(timeout_seconds=420),lambda d:d.update(profiling=True),lambda d:d.update(release_cpu_acceptance=True)]
        for change in changes:
            value=inputs();change(value)
            with self.subTest(change=change),self.assertRaises(AssertionError):self.verify(value)

    def test_preflight_keeps_deadline_and_source_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);gh=root/'github-output'
            with mock.patch.object(D.ndr,'git',side_effect=['','driver-sha']),mock.patch.object(D.ndr.time,'time',return_value=1000),mock.patch.dict(os.environ,GITHUB_OUTPUT=str(gh)):
                D.run('preflight',None,root,inputs())
            pre=json.loads((root/'preflight.json').read_text())
            self.assertEqual(pre['schema'],'ndr-kdf-reuse/v1');self.assertEqual(pre['deadline_unix'],2500)
            self.assertEqual(pre['workflow_timeout_seconds'],1800);self.assertIn(inputs()['after'],gh.read_text())


class MetricTests(unittest.TestCase):
    def test_exact_red_and_green_operation_counts(self):
        test=inputs()['tests'][0]
        for red,count in [(True,256),(False,1)]:
            metric,counts=D.operation_result(output(test,red=red),test,101 if red else 0,red)
            self.assertEqual(metric['initializations'],count);self.assertEqual(counts['expected_regression_failure'],red)

    def test_incomplete_wrong_guard_or_unexpected_pass_is_not_red(self):
        test=inputs()['tests'][0];text=output(test,red=True)
        variants=[('could not compile',101),(output(test),0),(text.replace(D.FAILURE,'other panic'),101),
            (text.replace('trials=256','trials=255'),101),(text.replace('initializations=256','initializations=255'),101),
            (text.replace('reference_state=true','reference_state=false'),101),(text+text,101),(text,-9)]
        for candidate,code in variants:
            with self.subTest(candidate=candidate[:60]),self.assertRaises(AssertionError):D.operation_result(candidate,test,code,True)

    def test_non_regression_lib_filter_checks_normal_count(self):
        test=inputs()['tests'][1]
        metric,counts=D.operation_result(output(test),test,0,False)
        self.assertIsNone(metric);self.assertEqual(counts['passed'],11)

    def test_benchmark_retains_all_seven_fixed_samples(self):
        test=D.benchmark_plan();metrics,counts=D.benchmark_result(samples(test),test,0)
        self.assertEqual(counts['passed'],1);self.assertEqual(metrics['samples_seconds'],[0.5+i/100 for i in range(7)])
        self.assertEqual(metrics['median_seconds'],0.53)

    def test_benchmark_rejects_changed_work_nonfinite_and_missing_samples(self):
        test=D.benchmark_plan();text=samples(test)
        variants=[text.replace('sample=6','sample=5'),text.replace('trials=160016','trials=160000'),
            text.replace('ciphertext_bytes=1091','ciphertext_bytes=100'),text.replace('budget=256','budget=512'),
            text.replace('elapsed_seconds=0.5 ','elapsed_seconds=nan '),text.replace('elapsed_seconds=0.5 ','elapsed_seconds=-1 '),
            text.replace('invalid_hmac=true','invalid_hmac=false'),text+text,'\n'.join(text.splitlines()[1:])]
        for candidate in variants:
            with self.subTest(candidate=candidate[:60]),self.assertRaises(AssertionError):D.benchmark_result(candidate,test,0)
        with self.assertRaises(AssertionError):D.benchmark_result(text,test,101)


class StageTests(unittest.TestCase):
    def exercise(self, which, *, compile_failure=False, wrong_red=False, failed_green=None, benchmark_failure=False):
        data=inputs();shared=D.ndr
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve();out=root/'evidence';target=root/'target';out.mkdir();target.mkdir()
            source={'commit':data[which],'parent':data['base'] if which=='before' else data['before'],
                'allocation_fixture_sha256':'same','dependency_inputs':{'lock':'same'}}
            versions={tool:f'{tool} 1.99.1 (test)\n' for tool in ('cargo','rustc')}
            shared.save(out/'preflight.json',{'schema':'ndr-kdf-reuse/v1','inputs':data,'inputs_sha256':shared.sha(D.INPUTS),'toolchain':'1.99.1','deadline_unix':time.time()+1500})
            if which=='after':
                (out/'before').mkdir();shared.save(out/'before/receipt.json',{'accepted':True,'stage':'before','source_path':str(root),'target_path':str(target),'source':dict(source,commit=data['before']),'toolchain':versions})
                shared.save(out/'before/benchmark.json',{'source':data['before'],'test':data['benchmark'],'metrics':{'median_seconds':1.06}})
            selected=data['tests'] if which=='after' else data['tests'][:1]
            artifacts=[]
            for kind,name in dict.fromkeys((t['kind'],t['target']) for t in selected):
                binary=target/name;binary.write_text('immutable executable')
                artifacts.append({'reason':'compiler-artifact','executable':str(binary),'target':{'kind':[kind],'name':name},'profile':{'test':True,'opt_level':'3','debuginfo':1,'debug_assertions':False}})
            calls=[]
            def bounded(command,_root,_out,name,timeout,deadline,_target):
                calls.append((name,command));self.assertEqual(_target,target);self.assertGreater(deadline,time.time())
                if name=='fetch':self.assertEqual(which,'before');return {'exit_code':0},'fetch'
                if name=='build':
                    self.assertEqual(timeout,600);self.assertEqual(command,shared.build_command(selected))
                    return {'exit_code':101 if compile_failure else 0},'\n'.join(map(json.dumps,artifacts))
                self.assertEqual(timeout,300)
                if name=='benchmark':
                    self.assertEqual(command,[str(target/'nostr_double_ratchet'),D.BENCHMARK,'--exact','--ignored','--nocapture','--test-threads=1'])
                    return {'exit_code':101 if benchmark_failure else 0},samples(data['benchmark'])
                index=int(name.split('-')[1]);test=selected[index]
                if which=='before':return {'exit_code':101},output(test,red=True,failure='wrong guard' if wrong_red else None)
                if index==failed_green:return {'exit_code':101},output(test,failure='unrelated correctness failure')
                return {'exit_code':0},output(test)
            failed=compile_failure or wrong_red or failed_green is not None or benchmark_failure
            with mock.patch.object(shared,'source_proof',return_value=source),mock.patch.object(shared,'git',return_value=''), \
                mock.patch.object(shared.subprocess,'check_output',side_effect=lambda cmd,**kw:versions[cmd[0]].encode()), \
                mock.patch.object(shared,'bounded',bounded),mock.patch.dict(os.environ,CARGO_TARGET_DIR=str(target)):
                if failed:
                    with self.assertRaises(AssertionError):D.run(which,root,out,data)
                    self.assertFalse((out/which/'benchmark.json').exists());self.assertFalse((out/'comparison.json').exists())
                else:
                    D.run(which,root,out,data)
                    result=json.loads((out/which/'benchmark.json').read_text());self.assertFalse(result['release_cpu_acceptance'])
                    receipt=json.loads((out/which/'receipt.json').read_text())
                    self.assertEqual(sum(t['counts']['passed'] for t in receipt['tests']),30 if which=='after' else 0)
                    self.assertEqual(result['binary_sha256'],receipt['test_binaries'][0]['sha256'])
                    if which=='after':
                        compare=json.loads((out/'comparison.json').read_text());self.assertEqual(compare['median_after_over_before'],0.5)
                        self.assertFalse(compare['release_cpu_acceptance']);self.assertEqual(len(compare['limitations']),3)
            self.assertEqual(sum(name=='build' for name,_ in calls),1)
            self.assertEqual(sum(name=='benchmark' for name,_ in calls),0 if compile_failure or wrong_red or failed_green is not None else 1)
            self.assertEqual(sum(name=='fetch' for name,_ in calls),int(which=='before'))

    def test_one_binary_reused_for_operation_correctness_and_one_benchmark(self):
        self.exercise('before');self.exercise('after')

    def test_build_or_unaccepted_red_blocks_benchmark(self):
        self.exercise('before',compile_failure=True);self.exercise('after',compile_failure=True);self.exercise('before',wrong_red=True)

    def test_every_focused_failure_blocks_benchmark_and_comparison(self):
        for index in range(len(D.selected_tests())):
            with self.subTest(index=index):self.exercise('after',failed_green=index)

    def test_benchmark_failure_cannot_emit_complete_comparison(self):
        self.exercise('before',benchmark_failure=True);self.exercise('after',benchmark_failure=True)


if __name__=='__main__':unittest.main()
