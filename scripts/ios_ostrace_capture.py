#!/usr/bin/env python3
"""Alternative Capture implementation using the pinned, raw-backed trace logger.

The ordinary capture's process ownership, live samples, measurement cutoff and
closure checks are unchanged. Callers must include trace_evidence in their
final record and validate it again during offline acceptance replay.
"""
from __future__ import annotations
import json
from pathlib import Path
import time

from ios_relaunch_adapters import Capture
from ios_ordinary_relaunch import binding, read_bound, require, verify_binding
import ios_ostrace_logger as logger


def filters(bundle, board_pid):
    return {'startup': {'pid': None, 'process_name': 'IrisChat', 'messages': ['iris.startup ']},
            'lifecycle': {'pid': board_pid, 'process_name': 'SpringBoard',
                          'messages': [bundle, 'com.apple.Preferences']}}


class OstraceCapture(Capture):
    def __init__(self, window, coredevice, transport_binding):
        super().__init__(window, coredevice)
        self.transport_binding = transport_binding
        self.transport = read_bound(transport_binding)
        require(self.transport.get('schema') == 'iris-ostrace-transport-v1', 'Wrong trace transport')
        self.python = verify_binding(self.transport['python'])
        self.logger_source = binding(Path(logger.__file__).resolve())
        self.trace_evidence = None
        self.selections = None

    def start(self):
        require(not self.streams, 'Capture already started')
        c = self.window.config
        require(c['process_name'] == 'IrisChat', 'Unexpected captured app')
        rows = self.core.run(['device','info','processes'])['runningProcesses']
        boards = [r for r in rows if r.get('executable','').endswith('/SpringBoard.app/SpringBoard')]
        require(len(boards) == 1, 'Missing unique SpringBoard process')
        self.os_process = boards[0]
        self.selections = filters(c['bundle_id'], self.os_process['processIdentifier'])
        for name, selected in self.selections.items():
            directory = self.window.out / (name + '-trace')
            args = [str(self.python), '-I', '-B', self.logger_source['path'],
                    '--transport', self.transport_binding['path'], '--transport-sha256', self.transport_binding['sha256'],
                    '--device', c['device'], '--output', str(directory), '--process-name', selected['process_name']]
            if selected['pid'] is not None:
                args += ['--pid', str(selected['pid'])]
            for message in selected['messages']:
                args += ['--match', message]
            item = self.window.spawn(args, name)
            item.update(capture=directory/'capture.log', capture_name=name, trace_receipt=directory/'receipt.json')
            self.streams[name] = item
        deadline = min(self.window.clock()+5, c['deadline_monotonic'])
        while True:
            self.window.guard(); self.health()
            if all(item['capture'].exists() and item['capture'].read_bytes().startswith(
                    ('[connected:'+c['device']+']\n').encode()) for item in self.streams.values()):
                self.prefixes()
                return
            require(self.window.clock() < deadline, 'Trace loggers did not become ready')
            time.sleep(.02)

    def final_capture(self, samples):
        result = super().final_capture(samples)
        evidence = {'schema':'iris-ostrace-capture-v1','logger_source':self.logger_source,
                    'adapter_source':binding(Path(__file__).resolve()), 'transport':self.transport_binding,
                    'streams':{name: binding(item['trace_receipt']) for name,item in self.streams.items()}}
        validate_evidence(evidence, result, device=self.window.config['device'],
                          bundle=self.window.config['bundle_id'], board_pid=self.os_process['processIdentifier'])
        self.trace_evidence = evidence
        return result


def validate_evidence(evidence, capture, *, device, bundle, board_pid):
    require(set(evidence) == {'schema','logger_source','adapter_source','transport','streams'}
            and evidence['schema'] == 'iris-ostrace-capture-v1'
            and evidence['logger_source'] == binding(Path(logger.__file__).resolve())
            and evidence['adapter_source'] == binding(Path(__file__).resolve())
            and set(evidence['streams']) == {'startup','lifecycle'}, 'Wrong raw trace source evidence')
    verify_binding(evidence['adapter_source'])
    selected = filters(bundle, board_pid)
    result = {}
    for name in ('startup','lifecycle'):
        receipt = read_bound(evidence['streams'][name])
        proof = logger.replay(receipt, expected_device=device, expected_selection=selected[name],
                              expected_transport=evidence['transport'], expected_source=evidence['logger_source'])
        require(verify_binding(proof['capture']).read_text() == capture[name+'_text'], 'Raw trace differs from measured capture')
        result[name] = proof
    return result
