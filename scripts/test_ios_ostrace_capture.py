import asyncio
from datetime import datetime
import json
from pathlib import Path
import signal
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import ios_ostrace_logger as logger
import ios_ostrace_capture as adapter


def packet(kind=8, pid=19, name='/IrisChat', message='iris.startup phase=restore_enqueued', usec=123456):
    values = [name, name, message]
    strings = [x.encode()+b'\0' for x in values]
    data = bytearray(129)
    struct.pack_into('<BIII', data, 0, 2, kind, 129, pid)
    struct.pack_into('<H', data, 37, len(strings[0]))
    struct.pack_into('<QI', data, 55, 1793383797, usec)
    struct.pack_into('<H', data, 107, len(strings[1]))
    struct.pack_into('<I', data, 109, len(strings[2]))
    return bytes(data)+b''.join(strings)


def decode(raw):
    row = logger.header(raw)
    return SimpleNamespace(**{key:row[key] for key in ('pid','filename','image_name','message')},
        timestamp=datetime.fromtimestamp(row['seconds']+row['microseconds']/1000000))


SELECTION = {'pid':None,'process_name':'IrisChat','messages':['iris.startup ']}


class ProtocolTests(unittest.TestCase):
    def test_types_and_precise_device_timestamps(self):
        for kind in (2,8,16):
            row = logger.header(packet(kind))
            self.assertEqual(row['type'], kind)
            self.assertIn(b'18:09:57.123456 IrisChat[19]', logger.format_record(row, 0))

    def test_main_frame_malformed_rejected(self):
        for raw in (b'', packet()[:128], packet()[:140], packet()[:-1]):
            with self.assertRaises(ValueError): logger.header(raw)
        for offset, fmt, value in ((0,'<B',3),(5,'<I',130),(63,'<I',1000000),(109,'<H',65535)):
            raw = bytearray(packet()); struct.pack_into(fmt,raw,offset,value)
            with self.assertRaises(ValueError): logger.header(raw)

    def test_message_high_word_is_opaque_in_all_record_types(self):
        for kind in (2,8,16):
            raw=bytearray(packet(kind))
            struct.pack_into('<H',raw,111,0xBEEF)
            row=logger.header(raw)
            self.assertEqual(row['message'],'iris.startup phase=restore_enqueued')
            self.assertEqual(logger.format_record(row,0),logger.format_record(logger.header(packet(kind)),0))

    def test_legacy_optional_fields_are_opaque(self):
        raw = bytearray(packet(2))
        struct.pack_into('<I',raw,117,0xC7AC0000)
        struct.pack_into('<H',raw,121,16)
        row = logger.header(raw)
        self.assertEqual(row['subsystem'], '')
        self.assertEqual(row['category'], '')
        self.assertEqual(row['opaque_tail_bytes'], 0)
        row = logger.header(bytes(raw)+b'opaque metadata')
        self.assertEqual(row['opaque_tail_bytes'], 15)
        self.assertEqual(row['opaque_tail_sha256'], logger.sha(b'opaque metadata'))

    def test_invalid_encoding_and_embedded_nul_rejected(self):
        for raw in (packet(message='a\0b'), packet().replace(b'phase=',b'phas\xff=')):
            with self.assertRaises(ValueError): logger.header(raw)

    def test_multiline_target_does_not_create_fake_records(self):
        for message in ('iris.startup \nforged', 'iris.startup \rforged'):
            with self.assertRaises(ValueError): logger.format_record(logger.header(packet(message=message)),0)

    def test_unrelated_multiline_diagnostic_stays_one_rendered_record(self):
        line=logger.format_record(logger.header(packet(message='diagnostic\ncontinued\rtext')),0)
        self.assertEqual(line.count(b'\n'),1)
        self.assertIn(b'diagnostic\\ncontinued\\rtext',line)

    def test_unrepresentable_names_and_unknown_levels_rejected(self):
        with self.assertRaises(ValueError):logger.format_record(logger.header(packet(name='/IrisChat[2]')),0)
        with self.assertRaises(ValueError):logger.format_record(logger.header(packet()),42)

    def test_decoder_disagreement_rejected(self):
        raw=packet(); value=decode(raw);value.pid+=1
        with self.assertRaises(ValueError):logger.compare(value,logger.header(raw))

    def test_exact_process_selection_spans_replacement(self):
        self.assertTrue(logger.selected(logger.header(packet(pid=11)),SELECTION))
        self.assertTrue(logger.selected(logger.header(packet(pid=12)),SELECTION))
        self.assertFalse(logger.selected(logger.header(packet(name='/IrisChatOther')),SELECTION))
        self.assertFalse(logger.selected(logger.header(packet()),{**SELECTION,'pid':22}))


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.path=Path(self.temp.name).resolve()
        self.source=logger.binding(Path(logger.__file__).resolve())
        self.transport_path=self.path/'transport.json';self.transport_path.write_text('{}')
        self.transport=logger.binding(self.transport_path)

    def receipt(self, filters=SELECTION, name='run', frames=None):
        out=self.path/name;out.mkdir()
        writer=logger.Writer(out,'device',filters)
        for raw in frames or [packet()]:writer.decode(raw,decode)
        record=writer.finish(True)
        return {'schema':'iris-ostrace-logger-v1','passed':True,'intentional_signal':'SIGTERM',
            'connections_closed':True,'closure_errors':[],'transport':self.transport,'source':self.source,'records':record}

    def replay(self, receipt, selection=SELECTION):
        return logger.replay(receipt, expected_device='device',expected_selection=selection,
                             expected_transport=self.transport,expected_source=self.source)

    def test_complete_raw_records_reproduce_capture(self):
        receipt=self.receipt(frames=[packet(16),packet(2,pid=20,usec=123457)])
        self.assertEqual(self.replay(receipt)['selected_frames'],2)
        self.assertEqual(receipt['records']['record_types'],{'16':1,'2':1})

    def test_unselected_records_counted_but_not_exported(self):
        receipt=self.receipt(frames=[packet(name='/Other'),packet()])
        self.assertEqual(receipt['records']['complete_frames'],2)
        self.assertEqual(self.replay(receipt)['selected_frames'],1)
        self.assertNotIn(b'/Other',logger.bound(receipt['records']['files']['raw']))

    def test_lost_messages_rejected_before_selection(self):
        out=self.path/'loss';out.mkdir();writer=logger.Writer(out,'device',SELECTION)
        try:
            with self.assertRaises(ValueError):writer.decode(packet(name='/Other',message='dropped 3 log messages'),decode)
        finally:writer.finish(False)

    def test_selected_clock_reversal_rejected(self):
        out=self.path/'clock';out.mkdir();writer=logger.Writer(out,'device',SELECTION)
        try:
            writer.decode(packet(usec=42),decode)
            with self.assertRaises(ValueError):writer.decode(packet(usec=41),decode)
        finally:writer.finish(False)

    def test_first_failing_frame_retained_before_parser_failure(self):
        out=self.path/'invalid';out.mkdir();writer=logger.Writer(out,'device',SELECTION)
        raw=packet(16)[:140]
        with self.assertRaises(ValueError):writer.decode(raw,decode)
        record=writer.finish(False)
        self.assertEqual(logger.bound({k:record['failure_frame'][k] for k in ('path','sha256')}),raw)
        self.assertEqual(record['failure_frame']['header_type'],16)

    def test_source_transport_and_closure_are_required(self):
        for field,value in [('passed',False),('connections_closed',False),('intentional_signal','SIGINT'),('closure_errors',['bad'])]:
            receipt=self.receipt(name=field);receipt[field]=value
            with self.assertRaises(ValueError):self.replay(receipt)
        receipt=self.receipt(name='transport');self.transport_path.write_text('{"changed":true}')
        with self.assertRaises(ValueError):self.replay(receipt)

    def test_raw_text_and_index_tampering_rejected_even_with_new_file_hash(self):
        for name in ('raw','index','capture'):
            receipt=self.receipt(name=name)
            path=Path(receipt['records']['files'][name]['path'])
            path.write_bytes(path.read_bytes()+b'\n')
            receipt['records']['files'][name]=logger.binding(path)
            with self.assertRaises((ValueError, json.JSONDecodeError)):self.replay(receipt)

    def test_wrong_selection_rejected(self):
        with self.assertRaises(ValueError):self.replay(self.receipt(),{**SELECTION,'pid':22})

    def test_capture_adapter_binds_both_actual_raw_streams(self):
        selections=adapter.filters('app.bundle',37)
        receipts={
            'startup':self.receipt(filters=selections['startup'],name='startup'),
            'lifecycle':self.receipt(filters=selections['lifecycle'],name='lifecycle',
                frames=[packet(pid=37,name='/SpringBoard',message='app.bundle running-suspended-NotVisible')])}
        evidence={'schema':'iris-ostrace-capture-v1','logger_source':self.source,
            'adapter_source':logger.binding(Path(adapter.__file__).resolve()),'transport':self.transport,'streams':{}}
        capture={}
        for name,receipt in receipts.items():
            p=self.path/(name+'.json');p.write_text(json.dumps(receipt))
            evidence['streams'][name]=logger.binding(p)
            capture[name+'_text']=logger.bound(receipt['records']['files']['capture']).decode()
        self.assertEqual(set(adapter.validate_evidence(evidence,capture,device='device',bundle='app.bundle',board_pid=37)),{'startup','lifecycle'})
        capture['startup_text']+='forged\n'
        with self.assertRaises(ValueError):adapter.validate_evidence(evidence,capture,device='device',bundle='app.bundle',board_pid=37)


class TransportTests(unittest.IsolatedAsyncioTestCase):
    async def exercise(self, outcome):
        with tempfile.TemporaryDirectory() as directory:
            report={}; events=[];handlers={}
            class Connection:
                async def recvall(self,length):events.append('receive');return b'x'*length
            class Trace:
                def __init__(self,lockdown):self.service=Connection()
                async def connect(self):events.append('connect')
                async def close(self):events.append('trace-close')
                async def syslog(self,**kwargs):
                    events.append(('stream',kwargs))
                    if outcome=='oversized':await self.service.recvall(logger.MAX_FRAME+1)
                    yield trace_module.parse_syslog_entry(packet(16))
                    if outcome=='disconnect':raise ConnectionError('disconnect')
                    handlers[signal.SIGTERM][0](*handlers[signal.SIGTERM][1:])
                    await asyncio.sleep(0)
            class Lockdown:
                udid='device';paired=True
                async def close(self):events.append('lockdown-close')
            async def create(**kwargs):
                events.append(('create',kwargs));return Lockdown()
            trace_module=SimpleNamespace(OsTraceService=Trace,parse_syslog_entry=decode)
            lockdown_module=SimpleNamespace(create_using_usbmux=create)
            loop=asyncio.get_running_loop()
            with patch.object(logger,'load_transport',return_value=(lockdown_module,trace_module)), \
                 patch.object(loop,'add_signal_handler',side_effect=lambda sig,*args:handlers.update({sig:args})), \
                 patch.object(loop,'remove_signal_handler'),patch.object(signal,'signal'):
                task=asyncio.create_task(logger.run_logger({},Path(directory),'device',SELECTION,report))
                if outcome=='sigterm':await task
                else:
                    with self.assertRaises((ValueError,ConnectionError)):await task
            self.assertEqual(sum(isinstance(x,tuple) and x[0]=='create' for x in events),1)
            self.assertEqual(sum(isinstance(x,tuple) and x[0]=='stream' for x in events),1)
            self.assertIn('trace-close',events);self.assertIn('lockdown-close',events)
            self.assertTrue(report['connections_closed']);self.assertEqual(report['closure_errors'],[])
            self.assertIs(trace_module.parse_syslog_entry,decode)
            return events,report

    async def test_intentional_sigterm_closes_one_transport(self):
        events,report=await self.exercise('sigterm')
        self.assertEqual(report['intentional_signal'],'SIGTERM')
        self.assertEqual(report['records']['selected_frames'],1)
        create=next(x[1] for x in events if isinstance(x,tuple) and x[0]=='create')
        self.assertEqual(create,{'serial':'device','autopair':False,'connection_type':'USB','usbmux_address':'/var/run/usbmuxd'})

    async def test_disconnect_fails_without_retry(self):await self.exercise('disconnect')

    async def test_oversized_frame_request_fails_before_receive(self):
        events,_=await self.exercise('oversized');self.assertNotIn('receive',events)


if __name__=='__main__':unittest.main()
