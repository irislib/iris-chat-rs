"""Wire-log scheduling checks; no phone, signing or network actions."""
import copy
from pathlib import Path
from types import SimpleNamespace
import tempfile
import time
import unittest

import ios_early_background_observer as observer
import ios_ordinary_relaunch as relaunch
import ios_relaunch_lifecycle as lifecycle
from test_ios_ordinary_relaunch import DistributionFixture
from test_ios_relaunch_lifecycle import stamp


class MarkerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.f = DistributionFixture(self.root)
        self.header = '[connected:synthetic-device]\n'
        self.paths = {k: self.root / (k + '.log') for k in ('startup', 'lifecycle')}
        for path in self.paths.values(): path.write_text(self.header)
        config = {'process_name':'Example', 'bundle_id':self.f.config['bundle_id'],
                  'deadline_monotonic': time.monotonic() + 30}
        self.capture = SimpleNamespace(window=SimpleNamespace(config=config),
            os_process={'processIdentifier':1},
            streams={k:{'capture':p} for k,p in self.paths.items()},
            prefixes=lambda:{k+'_text':p.read_text() for k,p in self.paths.items()}, health=lambda:None)
        self.controller = relaunch.OrdinaryForegroundRelaunch(self.f.config_binding, self.f.candidate,
                                                            self.f.installed,self.f.executable)
        r = self.controller.record
        r.update(launch_count=1, trigger_count=1, installed=self.f.installed, result=self.f.result(),
            command=relaunch.launch_command(self.f.config['bundle_id'],replacement=True),
            started_utc=10,returned_utc=10.2,executable=self.f.executable,before=self.f.snapshot(101,9.9))
        self.proof = relaunch.validate_launch_result(self.controller.distribution,r['installed'],r['result'],
            r['command'],10,10.2,10.3,executable=r['executable'],before=r['before'])
        r['launch_result_proof'] = self.proof
        self.calls = []
        self.home = SimpleNamespace(arm=lambda:{'armed':True}, fire=lambda armed,proof:self.calls.append(proof) or {'sent_count':1})
        self.w = observer.ObservedRestorationHome(self.controller,self.capture,self.home)
        self.addCleanup(self.close_watcher)

    def close_watcher(self):
        try:self.w.close()
        except Exception:pass

    def startup(self,phase='restore_enqueued',pending=True,background=False,pid=102,t=.1):
        return (f'{stamp(t)} Example[{pid}] <Notice>: iris.startup phase={phase} '
                f'restore_pending_result={str(pending).lower()} native_restoring_observed=false '
                f'app_backgrounded={str(background).lower()}\n')

    def state(self,value=lifecycle.ACTIVE,pid=102,t=0):
        return (f'{stamp(t)} SpringBoard(RunningBoardServices)[1] <Notice>: Received state update for '
                f'{pid} (app<{self.f.config["bundle_id"]}(uuid)>, {value}\n')

    def feed(self,startup=None,state=None):
        for kind,text in [('startup',startup),('lifecycle',state)]:
            if text:
                with self.paths[kind].open('a') as f:f.write(text)

    def read_synchronously(self):
        if not self.w.tails:
            self.w.tails={k:observer._Tail(p,self.header.encode()) for k,p in self.paths.items()}
        self.w.record['records'].extend(self.w._read_batch())

    def test_real_validated_launch_and_pending_foreground_marker_fire_once(self):
        armed=self.w.arm();self.feed(self.startup(),self.state())
        self.assertEqual(self.w.fire(armed,self.proof),{'sent_count':1})
        self.w.close();self.assertEqual(self.calls,[self.proof])
        self.assertFalse(self.w.record['physical_overlap_claimed'])
        with self.assertRaises(relaunch.EvidenceError):self.w.fire(armed,self.proof)
        self.assertEqual(len(self.calls),1)

    def test_coalesced_completion_never_fires(self):
        self.feed(self.startup()+self.startup('restore_result_observed',False,t=.2),self.state())
        self.read_synchronously()
        with self.assertRaises(relaunch.EvidenceError):self.w._eligible(102)
        self.assertEqual(self.calls,[])

    def test_restore_already_backgrounded_never_qualifies(self):
        self.feed(self.startup(background=True),self.state());self.read_synchronously()
        with self.assertRaises(relaunch.EvidenceError):self.w._eligible(102)

    def test_no_foreground_evidence_waits(self):
        self.feed(self.startup());self.read_synchronously();self.assertIsNone(self.w._eligible(102))

    def test_old_process_marker_cannot_trigger_new_process(self):
        self.feed(self.startup(pid=101),self.state(pid=101));self.read_synchronously()
        self.assertIsNone(self.w._eligible(102))

    def test_process_already_switched_away_rejects(self):
        self.feed(self.startup(),self.state()+self.state('running-active-NotVisible',t=.2))
        self.read_synchronously()
        with self.assertRaises(relaunch.EvidenceError):self.w._eligible(102)

    def test_no_device_action_before_real_controller_proof(self):
        armed=self.w.arm()
        with self.assertRaises(relaunch.EvidenceError):self.w.fire(armed,copy.deepcopy(self.proof))
        self.assertEqual(self.calls,[])

    def test_mutated_actual_launch_result_rejects(self):
        armed=self.w.arm();self.controller.record['result']['launchOptions']['arguments']=['override']
        with self.assertRaises(relaunch.EvidenceError):self.w.fire(armed,self.proof)
        self.assertEqual(self.calls,[])

    def test_incomplete_line_is_not_an_event(self):
        self.read_synchronously();text=self.startup();self.feed(text[:-1],self.state())
        self.read_synchronously();self.assertIsNone(self.w._eligible(102))
        self.feed('\n');self.read_synchronously();self.assertEqual(self.w._eligible(102)['phase'],'restore_enqueued')

    def test_replaced_capture_rejects(self):
        self.read_synchronously();path=self.paths['startup'];path.rename(path.with_suffix('.old'));path.write_text(self.header)
        with self.assertRaises(relaunch.EvidenceError):self.read_synchronously()

    def test_shrunk_capture_rejects(self):
        self.read_synchronously();self.paths['startup'].write_text('')
        with self.assertRaises(relaunch.EvidenceError):self.read_synchronously()

    def test_reconnected_capture_rejects(self):
        self.read_synchronously();self.feed('[connected:synthetic-device]\n')
        with self.assertRaises(relaunch.EvidenceError):self.read_synchronously()

    def test_reversed_device_clock_rejects(self):
        self.feed(self.startup(t=.2)+self.startup(t=.1))
        with self.assertRaises(relaunch.EvidenceError):self.read_synchronously()

    def test_malformed_target_marker_rejects(self):
        self.feed(f'{stamp(.1)} Example[102] <Notice>: iris.startup malformed\n')
        with self.assertRaises(relaunch.EvidenceError):self.read_synchronously()

    def test_observer_error_prevents_trigger(self):
        armed=self.w.arm();self.w.error=RuntimeError('logger ended')
        with self.assertRaises(RuntimeError):self.w.fire(armed,self.proof)
        self.assertEqual(self.calls,[])


if __name__=='__main__':unittest.main()
