#!/usr/bin/env python3
"""Synthetic wire/log contracts only; no physical commands or builds."""
import copy
import hashlib
import json
import os
import subprocess
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import Mock, patch

import ios_voip_background_wake as cold
import ios_relaunch_lifecycle as life
from ios_ordinary_relaunch import EvidenceError, binding
from test_ios_ordinary_relaunch import DistributionFixture
import test_ios_relaunch_lifecycle as logs


def fixture():
    initial_state = logs.state(life.ACTIVE, -2, pid=50).replace(logs.BUNDLE, cold.SETTINGS)
    bootstrap = f'{logs.stamp(-.1)} SpringBoard(FrontBoard)[1] <Notice>: [app<{logs.BUNDLE}>:102] Bootstrap success!\n'
    entry = bootstrap + logs.state(cold.BACKGROUND, 0)
    suspended = logs.state(life.SUSPENDED, 1)
    resumed = logs.state(life.ACTIVE, 137)
    enqueue = logs.startup('restore_enqueued', .1, pending=True, background=True)
    result = logs.startup('restore_result_observed', .7, background=True)
    value = {'startup_text': logs.HEADER + enqueue + result,
             'lifecycle_text': logs.HEADER + initial_state + entry + suspended + resumed,
             'device': logs.DEVICE, 'process_name': 'Chat', 'samples': []}
    initial = {'startup': logs.binding(logs.HEADER), 'lifecycle': logs.binding(logs.HEADER + initial_state)}
    for stamp in [0, 1, *range(6, 137, 5), 137, 142, 147, 152]:
        startup = logs.HEADER + (enqueue if stamp < 1 else enqueue + result)
        lifecycle = logs.HEADER + initial_state + entry + (suspended if stamp >= 1 else '') + (resumed if stamp >= 137 else '')
        row = {'observed_monotonic': stamp, 'pid': logs.PID, 'executable': logs.EXECUTABLE, 'loggers_alive': True, 'stderr': ''}
        for name, text in (('startup', startup), ('lifecycle', lifecycle)):
            row.update({name + '_' + k: v for k, v in logs.binding(text).items()})
        value['samples'].append(row)
    logs.bind_cutoff(value)
    return value, initial, len(value['samples']) - 5


def evaluate(value, initial, resume_index):
    return cold.evaluate_lifecycle(value, initial, resume_index, bundle=logs.BUNDLE, board_pid=1, settings_pid=50,
        proof={'pid': logs.PID, 'previous_pid': 101, 'executable': logs.EXECUTABLE})


class LifecycleTests(unittest.TestCase):
    def test_one_actual_os_bootstrap_is_required_and_never_inferred_from_push(self):
        for old, new in [(' Bootstrap success!', ' No bootstrap!'),
                         (':102] Bootstrap success!', ':103] Bootstrap success!')]:
            self.reject(lambda v, _, a=old, b=new: logs.replace_capture(v, 'lifecycle', a, b))
        line = f'{logs.stamp(-.1)} SpringBoard(FrontBoard)[1] <Notice>: [app<{logs.BUNDLE}>:102] Bootstrap success!\n'
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', line, line + line))
        foreground = f'{logs.stamp(-.2)} SpringBoard(FrontBoard)[1] <Notice>: Bootstrapping app<{logs.BUNDLE}> with intent foreground-interactive\n'
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', line, foreground + line))

    def test_bootstrap_completion_can_follow_initial_background_state(self):
        boot = f'{logs.stamp(.000418)} SpringBoard(FrontBoard)[1] <Notice>: [app<{logs.BUNDLE}>:102] Bootstrap success!\n'
        data = logs.HEADER + logs.state(cold.BACKGROUND, 0) + boot + logs.state(life.SUSPENDED, 1)
        cold.verify_os_bootstrap(data.encode(), len(logs.HEADER.encode()), logs.BUNDLE, 1, {'pid':102})

    def test_bootstrap_must_precede_first_actual_suspension(self):
        boot = f'{logs.stamp(1.1)} SpringBoard(FrontBoard)[1] <Notice>: [app<{logs.BUNDLE}>:102] Bootstrap success!\n'
        data = logs.HEADER + logs.state(cold.BACKGROUND, 0) + logs.state(life.SUSPENDED, 1) + boot
        with self.assertRaises(EvidenceError):
            cold.verify_os_bootstrap(data.encode(), len(logs.HEADER.encode()), logs.BUNDLE, 1, {'pid':102})

    def test_between_sample_exit_cannot_be_hidden_by_same_pid_inventory(self):
        line = f'{logs.stamp(2)} SpringBoard(FrontBoard)[1] <Notice>: [app<{logs.BUNDLE}>:102] Process exited\n'
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 1),
                                                      logs.state(life.SUSPENDED, 1) + line))

    def test_cold_settings_foreground_suspension_and_same_process_resume(self):
        value, initial, index = fixture()
        before = copy.deepcopy(value)
        result = evaluate(value, initial, index)
        self.assertEqual(value, before)
        self.assertEqual((result['entry_state'], result['foreground_activations']), ('background', 0))
        self.assertEqual((result['observed_seconds'], result['resumed_seconds']), (135, 15))
        self.assertFalse(result['physical_release_acceptance'])
        self.assertTrue(result['preserved_state_followup_required'])

    def test_normal_same_pid_activation_transition_does_not_count_as_foreground(self):
        value, initial, index = fixture()
        active = logs.state(life.ACTIVE,137)
        logs.replace_capture(value,'lifecycle',active,logs.state(cold.BACKGROUND,136.5)+active)
        logs.bind_cutoff(value)
        result=evaluate(value,initial,index)
        self.assertEqual((result['observed_seconds'],result['resumed_seconds']),(135,15))

    def test_background_after_first_visible_is_not_continuous_resume(self):
        self.reject(lambda value,_:logs.replace_capture(value,'lifecycle',logs.state(life.ACTIVE,137),
            logs.state(life.ACTIVE,137)+logs.state(cold.BACKGROUND,137.1)+logs.state(life.ACTIVE,137.2)))

    def reject(self, mutate):
        value, initial, index = fixture()
        mutate(value, initial)
        logs.bind_cutoff(value)
        with self.assertRaises((EvidenceError, life.LifecycleError)): evaluate(value, initial, index)

    def test_foreground_activation_between_polls_is_not_a_background_launch(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(cold.BACKGROUND, 0),
                                                      logs.state(life.ACTIVE, 0)))
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 1),
            logs.state(life.ACTIVE, .9) + logs.state(life.SUSPENDED, 1)))

    def test_settings_must_really_be_foreground_and_stay_there(self):
        value, initial, index = fixture()
        bad = value['lifecycle_text'].replace('running-active-Visible', 'running-active-NotVisible', 1)
        old = value['lifecycle_text'][:initial['lifecycle']['bytes']]
        new = bad[:initial['lifecycle']['bytes'] + len('NotVisible') - len('Visible')]
        logs.replace_capture(value, 'lifecycle', old, new)
        initial['lifecycle'] = logs.binding(new)
        with self.assertRaises(EvidenceError): evaluate(value, initial, index)
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 1),
            logs.state(life.SUSPENDED, 1) + logs.state(cold.BACKGROUND, 2, pid=50).replace(logs.BUNDLE, cold.SETTINGS)))

    def test_process_replacement_or_old_process_activity_fails(self):
        for name, old, new in [('startup', 'Chat[102]', 'Chat[101]'), ('lifecycle', 'for 102 ', 'for 103 ')]:
            self.reject(lambda v, _, n=name, a=old, b=new: logs.replace_capture(v, n, a, b))
        self.reject(lambda v, _: v['samples'][5].update(pid=103))

    def test_background_running_does_not_count_as_suspension(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', life.SUSPENDED, cold.BACKGROUND))
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 1),
            logs.state(life.SUSPENDED, 1) + logs.state(cold.BACKGROUND, 2)))

    def test_host_time_cannot_extend_short_device_suspension(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.ACTIVE, 137),
                                                      logs.state(life.ACTIVE, 135)))

    def test_short_host_interval_and_observation_gap_fail(self):
        self.reject(lambda v, _: v['samples'].__delitem__(5))
        self.reject(lambda v, _: [row.update(observed_monotonic=row['observed_monotonic'] * .9) for row in v['samples']])

    def test_background_startup_is_from_new_process_and_successful(self):
        for old, new in [('app_backgrounded=true', 'app_backgrounded=false'),
                         ('restore_result_observed', 'restore_cancelled'),
                         ('phase=restore_enqueued restore_pending_result=true', 'phase=restore_enqueued restore_pending_result=false')]:
            self.reject(lambda v, _, a=old, b=new: logs.replace_capture(v, 'startup', a, b))

    def test_loss_tampering_bad_os_source_and_postcutoff_state_fail(self):
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', '[1]', '[9]'))
        self.reject(lambda v, _: v['samples'][3].update(stderr='error'))
        self.reject(lambda v, _: v['samples'][3].update(startup_sha256='0' * 64))
        self.reject(lambda v, _: logs.replace_capture(v, 'lifecycle', logs.state(life.SUSPENDED, 1),
            logs.state(life.SUSPENDED, 1) + 'lost 1 messages\n'))
        value, initial, index = fixture()
        value['lifecycle_text'] += logs.state(life.SUSPENDED, 153)
        logs.bind_cutoff(value)
        with self.assertRaises(life.LifecycleError): evaluate(value, initial, index)


class SenderTests(unittest.TestCase):
    """Run the production JS function with fake transport and signing only."""
    def run_sender(self, scenario='success', changes=None):
        node = os.environ.get('IRIS_VOIP_TEST_NODE')
        self.assertTrue(node and Path(node).is_file(), 'Run qualification with an explicit existing Node binary')
        args = {'scenario':scenario, 'changes':changes or {}}
        harness = r'''
const {EventEmitter} = require('node:events');
const crypto = require('node:crypto');
const spec = JSON.parse(process.argv[1]);
const pair = spec.scenario==='real-signature' ? crypto.generateKeyPairSync('ec',{namedCurve:'prime256v1'}) : null;
const input = {token:'ab'.repeat(32),topic:'example.test.app.voip',key_path:'/synthetic/key',
 key_sha256:'c'.repeat(64),key_id:'KEYID12345',team_id:'TEAM123456',
 request_id:'12345678-1234-4123-8123-123456789abc',issued_at:1000,...spec.changes};
const seen = {connects:0,requests:0,destroys:0};
let timer;
const session = new EventEmitter(); session.alpnProtocol = 'h2';
session.destroy = () => { seen.destroys++; };
session.request = headers => {
 seen.requests++; seen.headers=headers;
 const stream = new EventEmitter(); stream.destroy = () => {};
 stream.end = payload => {
  seen.payload=JSON.parse(payload);
  queueMicrotask(() => {
   const s=spec.scenario;
   if(s==='goaway') return session.emit('goaway',0,0);
   if(s==='stream-error') return stream.emit('error',new Error(input.token));
   if(s==='timeout') return timer();
   stream.emit('response',{':status':s==='rejected'?400:200,'apns-id':s==='wrong-id'?'other':input.request_id});
   if(s==='body') stream.emit('data',Buffer.from('not-empty'));
   if(s==='oversize') stream.emit('data',Buffer.alloc(8193));
   stream.emit('end');
   session.emit('connect'); // Cannot trigger a second request.
  });
 };
 return stream;
};
sendOnce(input,{
 now:()=>1000,readKey:()=>Buffer.from('private-synthetic-key'),
 hash:()=>spec.scenario==='changed-key'?'d'.repeat(64):'c'.repeat(64),
 createPrivateKey:()=>pair ? pair.privateKey : ({asymmetricKeyType:'ec',asymmetricKeyDetails:{namedCurve:'prime256v1'}}),
 sign:data=>{
  seen.signing=data.toString();
  if(pair){
   const signature=crypto.sign('sha256',data,{key:pair.privateKey,dsaEncoding:'ieee-p1363'});
   seen.signature_verified=crypto.verify('sha256',data,{key:pair.publicKey,dsaEncoding:'ieee-p1363'},signature);
   return signature;
  }
  return Buffer.alloc(spec.scenario==='der-signature'?70:64,7);
 },
 connect:(host,options)=>{seen.connects++;seen.host=host;seen.options=options;queueMicrotask(()=>session.emit('connect'));return session;},
 setTimeout:fn=>{timer=fn;return 1;},clearTimeout:()=>{}
}).then(result=>process.stdout.write(JSON.stringify({result,seen})));
'''
        result = subprocess.run([node,'--no-addons','--disable-proto=throw','-e',cold.NODE_SENDER+harness,json.dumps(args)],
                                stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5,check=True)
        self.assertEqual(result.stderr,b'')
        return json.loads(result.stdout)

    def test_serialized_request_has_no_participant_identity_and_exact_production_headers(self):
        value = self.run_sender(); seen,result=value['seen'],value['result']
        self.assertTrue(result['accepted']); self.assertFalse(result['device_wake_verified'])
        self.assertEqual((seen['connects'],seen['requests'],result['retry_count']), (1,1,0))
        self.assertEqual(seen['host'],'https://api.push.apple.com:443')
        self.assertTrue(seen['options']['rejectUnauthorized'])
        self.assertEqual(seen['options']['ALPNProtocols'],['h2'])
        headers = seen['headers']
        self.assertEqual(set(headers),{':method',':scheme',':authority',':path','authorization','apns-topic',
                         'apns-push-type','apns-priority','apns-expiration','apns-id','content-type'})
        self.assertEqual([headers[x] for x in ('apns-push-type','apns-priority','apns-expiration')],['voip','10','0'])
        self.assertEqual(headers[':path'],'/3/device/'+'ab'*32)
        self.assertEqual(seen['payload'],{'aps':{'content-available':1},'iris_unavailable_call':headers['apns-id']})
        import base64
        jwt = headers['authorization'].removeprefix('bearer ').split('.')
        decode=lambda x: base64.urlsafe_b64decode(x+'='*((-len(x))%4))
        self.assertEqual(json.loads(decode(jwt[0])),{'alg':'ES256','kid':'KEYID12345'})
        self.assertEqual(json.loads(decode(jwt[1])),{'iss':'TEAM123456','iat':1000})
        self.assertEqual(len(decode(jwt[2])),64)
        self.assertNotIn('ab'*32,json.dumps(result)); self.assertNotIn('private-synthetic-key',json.dumps(result))

    def test_transport_rejection_timeout_and_goaway_never_retry(self):
        for scenario in ('goaway','stream-error','timeout','rejected','wrong-id','body','oversize'):
            with self.subTest(scenario=scenario):
                value=self.run_sender(scenario)
                self.assertFalse(value['result']['accepted'])
                self.assertEqual((value['seen']['connects'],value['seen']['requests']), (1,1))
                self.assertEqual(value['result']['retry_count'],0)
                self.assertFalse(value['result']['device_wake_verified'])

    def test_actual_standard_crypto_es256_uses_64_byte_jose_signature(self):
        value=self.run_sender('real-signature')
        self.assertTrue(value['result']['accepted'])
        self.assertTrue(value['seen']['signature_verified'])

    def test_bad_or_changed_secret_input_does_not_open_connection(self):
        for scenario,change in [('changed-key',{}),('der-signature',{}),('success',{'token':'bad'}),
                                ('success',{'issued_at':900}),('success',{'topic':'example.test.app'}),
                                ('success',{'extra':'identity'})]:
            with self.subTest(scenario=scenario,change=change):
                value=self.run_sender(scenario,change)
                self.assertFalse(value['result']['accepted'])
                self.assertEqual((value['seen']['connects'],value['seen']['requests']), (0,0))


class PrivateInputsTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name); self.d=DistributionFixture(self.root)
        self.window={'candidate':self.d.candidate,'device':logs.DEVICE,'bundle_id':self.d.config['bundle_id'],
                     'build':self.d.candidate['apple_build'],'executable':self.d.executable,'process_name':'Example'}
        self.subscription={'apns_tokens':['ab'*32],'apns_environment':'production',
            'apns_topic':self.window['bundle_id']+'.voip','filter':{'kinds':[21111],'#p':['synthetic-key']},'filters':[]}

    def inputs(self, *, observed=1000, lookup_id='synthetic-id', review_device=None, inventory=None):
        inputs={'identity_evidence':[self.d.write('owned-identity',{'synthetic':True,'subscription_id':'synthetic-id'})],
                'phone_continuity':[self.d.write('continuity',{'synthetic':True})],
                'installed_inventory':self.d.write('apps',{'result':{'apps':[inventory or self.d.installed]}}),
                'subscription':self.d.write('subscription',self.subscription)}
        inputs['lookup']=self.d.write('lookup',{'schema':'iris-exact-subscription-read-v1','subscription_id':lookup_id,
            'response':inputs['subscription'],'read_only':True,'record_count':1,
            'scope':'current-owned-phone-call-subscription','observed_utc':observed})
        value={'schema':'iris-owned-voip-token-v1','candidate':self.d.candidate,'device':logs.DEVICE,
               'bundle_id':self.window['bundle_id'],'subscription_id':'synthetic-id','inputs':inputs}
        review={**value,'schema':'iris-owned-voip-token-review-v1','decision':'accepted',
                'valid_from_utc':observed,'valid_until_utc':observed+900,'ownership_basis':'retained-registration-continuity',
                'token_sha256':hashlib.sha256(('ab'*32).encode()).hexdigest()}
        if review_device: review['device']=review_device
        value['review']=self.d.write('token-review',review)
        return {'token_evidence':self.d.write('token-evidence',value)}

    def test_exact_current_phone_registration_and_review_required(self):
        self.assertEqual(cold.token_for_owned_phone(self.inputs(),self.window,now=1001),'ab'*32)
        for kwargs in ({'observed':1},{'observed':1002},{'lookup_id':'another-id'},{'review_device':'another-device'},
                       {'inventory':{**self.d.installed,'bundleVersion':'old'}}):
            with self.subTest(kwargs=kwargs),self.assertRaises(EvidenceError):
                cold.token_for_owned_phone(self.inputs(**kwargs),self.window,now=1001)

    def test_ambiguous_nonproduction_and_noncall_registrations_fail(self):
        original=copy.deepcopy(self.subscription)
        for change in ({'apns_tokens':['ab'*32,'cd'*32]},{'apns_environment':'sandbox'},
                       {'apns_topic':'other.voip'},{'filter':{'kinds':[4]}},{'filters':[{}]}):
            self.subscription={**original,**change}
            with self.subTest(change=change),self.assertRaises(EvidenceError):
                cold.token_for_owned_phone(self.inputs(),self.window,now=1001)

    def test_token_or_review_tampering_is_rejected(self):
        config=self.inputs()
        (self.root/'subscription.json').write_text('{}')
        with self.assertRaises(EvidenceError): cold.token_for_owned_phone(config,self.window,now=1001)

    def test_explicit_execute_flag_is_required_before_any_inputs_or_device_access(self):
        with patch.object(cold,'OwnedWindow') as window,self.assertRaises(EvidenceError):
            cold.observe({},self.root/'never-created')
        window.assert_not_called()

    def test_resume_compares_actual_audit_if_available_and_states_unavailability(self):
        proof={'pid':102,'executable':self.d.executable,'initial_audit_token':None}
        resume={'command':['device','process','launch',self.d.config['bundle_id']],
                'result':self.d.result(replacement=False),'started_utc':10,'returned_utc':10.1,
                'validated_utc':10.3,'snapshot':self.d.snapshot(102,10.2)}
        result=cold.validate_resume(self.d.load(),resume,proof)
        self.assertFalse(result['initial_audit_token_available'])
        self.assertFalse(result['initial_to_resume_audit_equality_claimed'])
        proof['initial_audit_token']=resume['result']['process']['auditToken'][:]
        self.assertTrue(cold.validate_resume(self.d.load(),resume,proof)['initial_to_resume_audit_equality_claimed'])
        resume['result']['process']['auditToken'][-1]+=1
        with self.assertRaises(EvidenceError): cold.validate_resume(self.d.load(),resume,proof)
        resume['result']=self.d.result(103,replacement=False)
        with self.assertRaises(EvidenceError): cold.validate_resume(self.d.load(),resume,{**proof,'initial_audit_token':None})

    def test_core_device_additions_allow_only_settings_and_one_exact_owned_termination(self):
        window=Mock(out=self.root,serial=0,config={'device':logs.DEVICE,'bundle_id':self.window['bundle_id']},
                    tools={'coredevice':Path('/synthetic/devicectl')})
        def spawn(command,label):
            Path(command[command.index('--json-output')+1]).write_text(json.dumps({'result':{'actual':True}}))
            return {}
        window.spawn.side_effect=spawn
        core=cold.WakeCoreDevice(window,101)
        command=['device','process','terminate','--pid','101']
        self.assertEqual(core.run(command),{'actual':True})
        self.assertEqual(window.spawn.call_args.args[0],['/synthetic/devicectl','device','process','terminate',
            '--device',logs.DEVICE,'--timeout','6','--json-output',str(self.root/'coredevice-001.json'),'--pid','101'])
        for bad in (command,['device','process','terminate','--pid','102'],
                    ['device','process','launch','--no-activate',self.window['bundle_id']]):
            with self.assertRaises(EvidenceError): core.run(bad)

    def test_capture_filters_keep_new_process_markers_and_exact_os_source(self):
        window=Mock(out=self.root,config={'device':logs.DEVICE,'bundle_id':self.window['bundle_id'],'deadline_monotonic':100},
                    tools={'syslog':Path('/synthetic/syslog')},clock=lambda:1)
        core=Mock(); core.run.return_value={'runningProcesses':[
            {'processIdentifier':7,'executable':'file:///System/SpringBoard.app/SpringBoard'}]}
        capture=SimpleNamespace(streams={},core=core,window=window,health=lambda:None,prefixes=lambda:None)
        def spawn(command,label): Path(command[-1]).write_text(logs.HEADER); return {}
        window.spawn.side_effect=spawn
        cold.start_capture(capture)
        prefix=['/synthetic/syslog','-u',logs.DEVICE,'--no-colors']
        self.assertEqual([call.args for call in window.spawn.call_args_list],[
            (prefix+['-m','iris.startup ','-o',str(self.root/'startup.log')],'startup'),
            (prefix+['-p','7','-m',self.window['bundle_id'],'-m',cold.SETTINGS,'-o',str(self.root/'lifecycle.log')],'lifecycle')])

    def test_sender_puts_secrets_only_on_stdin_and_refuses_second_attempt(self):
        config=self.inputs()
        key=self.root/'key.p8'; key.write_text('synthetic'); key.chmod(0o600)
        config.update(node=self.d.write('node',{'synthetic':True}),credentials=self.d.write('credentials',{
            'schema':'iris-existing-apns-credentials-v1','environment':'production','bundle_id':self.window['bundle_id'],
            'local_key':binding(key),'remote_key_sha256':binding(key)['sha256'],'key_id':'KEYID12345','team_id':'TEAM123456'}))
        class Sink:
            data=b''
            def write(self,value): self.data+=value
            def close(self): pass
        sink=Sink()
        window=Mock(config=self.window,environment={'PATH':'synthetic','NODE_OPTIONS':'untrusted','SSLKEYLOGFILE':'untrusted'})
        window.spawn.return_value={'process':SimpleNamespace(stdin=sink)}
        sender=cold.SendOnce(window,config)
        with patch.object(cold.time,'time',return_value=1001): sender.start()
        sent=json.loads(sink.data)
        self.assertEqual(sent['token'],'ab'*32)
        self.assertEqual(sent['key_path'],str(key.resolve()))
        self.assertNotIn(sent['token'],repr(window.spawn.call_args))
        self.assertNotIn(str(key.resolve()),repr(window.spawn.call_args))
        self.assertEqual(window.environment,{'PATH':'synthetic'})
        with self.assertRaises(EvidenceError): sender.start()
        self.assertEqual(window.spawn.call_count,1)


class DriverTests(unittest.TestCase):
    setUp = PrivateInputsTests.setUp
    inputs = PrivateInputsTests.inputs

    def run_driver(self, *, accepted=True, foreground=False, replacement=False):
        d=self.d
        config=self.inputs()
        window_config={**self.window,'distribution_binding':d.config_binding,'coredevice_id':d.config['coredevice_id']}
        key=self.root/'synthetic-key'; key.write_bytes(b'not-a-real-key'); key.chmod(0o600)
        node=d.write('synthetic-node',{'not_executed':True})
        credentials=d.write('credentials',{'schema':'iris-existing-apns-credentials-v1','environment':'production',
            'bundle_id':window_config['bundle_id'],'local_key':binding(key),'remote_key_sha256':binding(key)['sha256']})
        config.update(schema='iris-voip-background-input-v1',existing_pid=101,node=node,credentials=credentials,
            trace_transport=d.write('trace-transport',{'synthetic':True}),
            window=d.write('driver-window',window_config),qualification=d.write('driver-qualification',{
                'schema':'iris-voip-wake-qualification-v1','passed':True,'exit_code':0,'device_actions':False,
                'network_requests':0,'checker_sources':cold.sources(),'tests':list(cold.TESTS),
                'tests_log':d.write('test-log',{'synthetic':True}),'node':node,
                'tagged_validator':binding(Path(cold.__file__).with_name('ios_runtime_acceptance.py'))}))
        config_binding=d.write('driver-config',config)
        clock=SimpleNamespace(now=0.)
        def advance(seconds): clock.now+=seconds
        output=self.root/'driver'; output.mkdir()
        cores=[]; attempts=[]
        class Window:
            def __init__(self,*_):
                self.config,self.out,self.serial=window_config,output,0
                self.capture,self.closed=None,False
                self.clock=lambda:clock.now
            def guard(self):
                if clock.now>300: raise EvidenceError('synthetic deadline')
            def close(self):
                self.closed=True
                cold.save(output/'closure.json',{'schema':'iris-relaunch-adapter-closure-v1',
                    'candidate':d.candidate,'device':logs.DEVICE,'closed':self.capture.cutoff is not None,
                    'failures':[] if self.capture.cutoff is not None else ['missing actual cutoff'],
                    'groups':[1,2,3], 'measurement_cutoff_sha256':None if self.capture.cutoff is None
                        else life.evidence_sha256(self.capture.cutoff)})
        class Core:
            def __init__(self,window,old):
                self.window,self.commands=window,[]
                self.settings,self.terminated,self.wake,self.resumed=False,False,None,None
                cores.append(self)
            def run(self,command):
                self.commands.append(command); advance(.05)
                if command[:3]==['device','info','apps']: result={'apps':[d.installed]}
                elif command==['device','info','processes']:
                    rows=[{'processIdentifier':1,'executable':'file:///System/SpringBoard.app/SpringBoard'}]
                    if not self.terminated or self.wake is not None:
                        rows.append({'processIdentifier':102 if self.wake is not None else 101,'executable':d.executable})
                    result={'runningProcesses':rows}
                elif command==['device','process','launch',cold.SETTINGS]:
                    self.settings=True; result=d.result(50,replacement=False)
                    result['process']['executable']='file:///Applications/Preferences.app/Preferences'
                elif command==['device','process','terminate','--pid','101']:
                    self.terminated=True; result={'terminated':101}
                elif command==['device','process','launch',d.config['bundle_id']]:
                    self.resumed=clock.now; result=d.result(103 if replacement else 102,replacement=False)
                else: raise AssertionError('unexpected native operation')
                self.window.serial+=1
                cold.save(output/f'coredevice-{self.window.serial:03d}.json',{'result':result})
                return result
        class Capture(cold.Capture):
            def __init__(self,window,core,transport):
                super().__init__(window,core)
                self.os_process={'processIdentifier':1,'executable':'file:///System/SpringBoard.app/SpringBoard'}
                self.trace_evidence={'transport':transport}
            def start(self): pass
            def snapshot(self):
                rows=self.core.run(['device','info','processes'])['runningProcesses']
                return {'observed_utc':1000+clock.now,'processes':[
                    {'pid':r['processIdentifier'],'executable':r['executable']} for r in rows if r['executable']==d.executable],
                    'foreground_pid':102 if self.core.resumed is not None else (101 if not self.core.settings else None)}
            def prefixes(self):
                state=lambda value,t,pid=102:logs.state(value,t,pid=pid).replace(logs.BUNDLE,d.config['bundle_id'])
                startup,oslog=logs.HEADER,logs.HEADER
                if self.core.settings:
                    oslog+=state(life.ACTIVE,-2,50).replace(d.config['bundle_id'],cold.SETTINGS)+state(life.SUSPENDED,-1,101)
                if self.core.wake is not None:
                    elapsed=clock.now-self.core.wake
                    oslog+=f'{logs.stamp(0)} SpringBoard(FrontBoard)[1] <Notice>: [app<{d.config["bundle_id"]}>:102] Bootstrap success!\n'
                    oslog+=state(life.ACTIVE if foreground else cold.BACKGROUND,.05)
                    if elapsed>=1: oslog+=state(life.SUSPENDED,1)
                    if elapsed>=.1: startup+=logs.startup('restore_enqueued',.1,pending=True,background=True)
                    if elapsed>=.7: startup+=logs.startup('restore_result_observed',.7,background=True)
                    if self.core.resumed is not None:
                        oslog+=state(cold.BACKGROUND,self.core.resumed-self.core.wake)
                        oslog+=state(life.ACTIVE,self.core.resumed-self.core.wake+.01)
                return {'startup_text':startup.replace(' Chat[',' Example['),'lifecycle_text':oslog}
            def mark_measurement_cutoff(self,samples):
                value={**self.prefixes(),'samples':samples}; logs.bind_cutoff(value)
                self.cutoff=value['measurement_cutoff']
            def final_capture(self,samples):
                return {**self.prefixes(),'samples':samples,'measurement_cutoff':self.cutoff,
                        'process_name':'Example','closure_binding':binding(output/'closure.json')}
        class Sender:
            def __init__(self,window,configuration): self.window=window
            def start(self):
                attempts.append('push'); self.started={'started_utc':1000+clock.now,'started_monotonic':clock.now}
                cores[0].wake=clock.now
                self.item={'process':SimpleNamespace(poll=lambda:0)}
                self.result={'schema':'iris-apns-single-request-v1','accepted':accepted,
                    'failure':None if accepted else 'apns-rejected','http_status':200 if accepted else 400,
                    'apns_id':'synthetic-id','request_count':1,'connection_count':1,'retry_count':0,'device_wake_verified':False}
                cold.save(output/'sender.stdout',self.result); (output/'sender.stderr').write_text('')
            def finish(self):
                return {**self.started,'finished_utc':1000+clock.now,'finished_monotonic':clock.now,
                    'result':self.result,'request_id':'synthetic-id','returncode':0 if accepted else 1,
                    'stdout':binding(output/'sender.stdout'),'stderr':binding(output/'sender.stderr'),
                    **{k:config[k] for k in ('node','credentials','token_evidence')}}
        with patch.object(cold,'OwnedWindow',Window),patch.object(cold,'WakeCoreDevice',Core), \
             patch.object(cold,'OstraceCapture',Capture),patch.object(cold,'validate_trace_evidence'),patch.object(cold,'SendOnce',Sender), \
             patch.object(cold.time,'monotonic',side_effect=lambda:clock.now), \
             patch.object(cold.time,'time',side_effect=lambda:1000+clock.now),patch.object(cold.time,'sleep',side_effect=advance):
            error=None
            try: cold.observe(config_binding,output,execute=True)
            except (EvidenceError,life.LifecycleError) as caught: error=caught
        record=json.loads((output/'capture.json').read_text())
        return record,cores[0].commands,attempts,error

    def test_full_driver_uses_one_os_request_and_retains_raw_evidence(self):
        record,commands,attempts,error=self.run_driver()
        self.assertIsNone(error)
        self.assertTrue(record['passed']); self.assertFalse(record['physical_release_acceptance'])
        self.assertEqual(attempts,['push'])
        self.assertEqual([x for x in commands if x[:2]==['device','process']], [
            ['device','process','launch',cold.SETTINGS],['device','process','terminate','--pid','101'],
            ['device','process','launch',self.d.config['bundle_id']]])
        self.assertIsNone(record['process']['initial_audit_token'])
        self.assertTrue(record['evaluation']['preserved_state_followup_required'])
        self.assertEqual(len(record['inventories']),len(record['samples']))

    def test_rejected_push_preserves_first_failure_without_resume_or_retry(self):
        record,commands,attempts,error=self.run_driver(accepted=False)
        self.assertIsNotNone(error); self.assertFalse(record['passed'])
        self.assertIn('not accepted',record['failure']); self.assertEqual(attempts,['push'])
        self.assertNotIn(['device','process','launch',self.d.config['bundle_id']],commands)
        self.assertEqual(record['push']['result']['http_status'],400)

    def test_foreground_boot_does_not_retry_or_gain_a_cold_pass(self):
        record,commands,attempts,error=self.run_driver(foreground=True)
        self.assertIsNotNone(error); self.assertFalse(record['passed']); self.assertEqual(attempts,['push'])
        self.assertNotIn(['device','process','launch',self.d.config['bundle_id']],commands)

    def test_actual_resume_replacement_keeps_failure_and_partial_samples(self):
        record,commands,attempts,error=self.run_driver(replacement=True)
        self.assertIsNotNone(error); self.assertFalse(record['passed']); self.assertEqual(attempts,['push'])
        self.assertIn('replaced',record['failure']); self.assertGreater(len(record['samples']),50)
        self.assertEqual(commands.count(['device','process','launch',self.d.config['bundle_id']]),1)

    def test_replay_rejects_forged_initial_audit_and_changed_native_inventory(self):
        record,_,_,error=self.run_driver(); self.assertIsNone(error)
        altered=copy.deepcopy(record); altered['process']['initial_audit_token']=[0,501,501,501,501,102,0,22]
        with self.assertRaises(EvidenceError): cold.evaluate(altered)
        altered=copy.deepcopy(record)
        altered['inventories'][2]=self.d.write('replacement-inventory',{'result':{'runningProcesses':[
            {'processIdentifier':103,'executable':self.d.executable},
            {'processIdentifier':1,'executable':'file:///System/SpringBoard.app/SpringBoard'}]}})
        with self.assertRaises(EvidenceError): cold.evaluate(altered)


if __name__ == '__main__': unittest.main()


