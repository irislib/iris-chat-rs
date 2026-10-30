#!/usr/bin/env python3
"""Pinned os_trace logger with retained raw target records and deterministic replay.

The transport remains upstream pymobiledevice3. Unknown optional header bytes
are opaque; framed main fields, UTF-8, identity, and microseconds are validated.
This module performs no device operation unless its explicit logger CLI runs.
"""
from __future__ import annotations
import argparse
import asyncio
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import signal
import struct
import sys
import time

MAX_FRAME = 1024 * 1024
MAX_CAPTURE = 8 * 1024 * 1024
MAX_RAW = 16 * 1024 * 1024
MAX_INDEX = 8 * 1024 * 1024
LOSS = re.compile(r'\b(?:dropped|lost) \d+ (?:log )?messages\b', re.I)

def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def binding(path):
    p = Path(path)
    return {'path': str(p), 'sha256': sha(p.read_bytes())}


def bound(value):
    require(set(value) == {'path', 'sha256'}, 'Invalid file binding')
    p = Path(value['path'])
    require(p.is_file() and p.resolve() == p, 'Bound file is missing or redirected')
    data = p.read_bytes()
    require(sha(data) == value['sha256'], 'Bound file changed')
    return data


def save(path, value):
    with Path(path).open('x') as f:
        json.dump(value, f, indent=2, allow_nan=False)
        f.write('\n')


def header(data):
    require(129 <= len(data) <= MAX_FRAME, 'Record size outside bound')
    marker, kind, size, pid = struct.unpack_from('<BIII', data)
    require(marker == 2 and size == 129, 'Unexpected record marker/header size')
    procsize = struct.unpack_from('<H', data, 37)[0]
    seconds, usec = struct.unpack_from('<QI', data, 55)
    image_size = struct.unpack_from('<H', data, 107)[0]
    # The maintained upstream decoder uses u16 here; the following u16 is
    # opaque metadata, not a high word of the string length.
    message_size = struct.unpack_from('<H', data, 109)[0]
    subsystem_size, category_size = (struct.unpack_from('<H', data, i)[0] for i in (117, 121))
    # Optional labels are meaningful only when both are present and fit the tail.
    # Legacy records reuse those header bytes; retain their remainder opaquely.
    main_end = size + procsize + image_size + message_size
    require(main_end <= len(data), 'Main record strings exceed frame')
    label_sizes = [subsystem_size, category_size] if subsystem_size and category_size and main_end + subsystem_size + category_size == len(data) else []
    sizes = [procsize, image_size, message_size] + label_sizes
    require(procsize > 0 and message_size > 0 and usec < 1000000, 'Missing strings or invalid microseconds')
    opaque_tail = data[size + sum(sizes):]
    fields, offset = [], size
    for length in sizes:
        part = data[offset:offset+length]
        require(not length or (part[-1:] == b'\0' and b'\0' not in part[:-1]), 'String terminator or boundary mismatch')
        fields.append(part[:-1].decode('utf-8', errors='strict') if length else '')
        offset += length
    return {'type': kind, 'pid': pid, 'seconds': seconds, 'microseconds': usec,
            'filename': fields[0], 'image_name': fields[1], 'message': fields[2],
            'subsystem': fields[3] if len(fields) > 3 else '', 'category': fields[4] if len(fields) > 4 else '',
            'opaque_tail_bytes': len(opaque_tail), 'opaque_tail_sha256': sha(opaque_tail)}


def compare(decoded, checked):
    require(decoded.pid == checked['pid'] and decoded.filename == checked['filename']
            and decoded.message == checked['message'] and decoded.image_name == checked['image_name'],
            'Independent header and upstream decoder disagree')
    expected = checked['seconds'] + checked['microseconds'] / 1000000
    require(abs(decoded.timestamp.timestamp() - expected) < 0.000001, 'Timestamp decoder disagreement')
    return decoded


def load_transport(config):
    site = Path(config['site_packages'])
    require(site.is_dir() and site.resolve() == site, 'Pinned package root changed')
    required = {'pymobiledevice3/lockdown.py', 'pymobiledevice3/lockdown_service_provider.py',
                'pymobiledevice3/service_connection.py', 'pymobiledevice3/usbmux.py',
                'pymobiledevice3/pair_records.py', 'pymobiledevice3/services/lockdown_service.py',
                'pymobiledevice3/services/os_trace.py'}
    require(isinstance(config['source_pins'], list) and len(config['source_pins']) <= 256,
            'Unbounded transport source manifest')
    provided = {str(Path(value['path']).relative_to(site)) for value in config['source_pins']}
    require(required <= provided, 'Missing critical transport source pins')
    for value in config['source_pins']:
        require(Path(value['path']).is_relative_to(site), 'Unexpected package pin')
        bound(value)
    sys.path[:] = [str(site)] + [p for p in sys.path if not ({'site-packages', 'dist-packages'} & set(Path(p).parts))]
    from pymobiledevice3 import lockdown
    from pymobiledevice3.services import os_trace
    for module in (lockdown, os_trace):
        require(Path(module.__file__).resolve().is_relative_to(site), 'Unexpected imported module')
    return lockdown, os_trace


def selection(value):
    require(set(value) == {'pid', 'process_name', 'messages'}, 'Wrong logger selection')
    require(value['pid'] is None or type(value['pid']) is int and value['pid'] > 0, 'Invalid selected PID')
    require(value['process_name'] is None or re.fullmatch(r'[A-Za-z][A-Za-z0-9_-]+', value['process_name']), 'Invalid process name')
    require(value['pid'] is not None or value['process_name'] is not None, 'Missing selected process')
    require(isinstance(value['messages'], list) and 1 <= len(value['messages']) <= 4
            and all(isinstance(x, str) and 0 < len(x) <= 128 and '\n' not in x for x in value['messages']), 'Invalid message selection')
    return value


def selected(row, filters):
    return ((filters['pid'] is None or row['pid'] == filters['pid'])
            and (filters['process_name'] is None or Path(row['filename']).name == filters['process_name'])
            and any(x in row['message'] for x in filters['messages']))


def format_record(row, level):
    process_name = Path(row['filename']).name
    image_name = Path(row['image_name']).name
    require(re.fullmatch(r'[A-Za-z0-9_.+-]+', process_name) is not None, 'Unrepresentable process name')
    message = row['message']
    require('\x00' not in message, 'NUL in target record')
    if any(x in message for x in ('\n','\r')):
        require(not any(marker in message for marker in
                        ('iris.startup ', 'Received state update for ', 'Bootstrap success!')),
                'Multiline lifecycle record cannot be represented')
        # Unrelated multiline diagnostics stay one rendered record. Exact bytes
        # remain in records.bin; an embedded line can never impersonate a new
        # authenticated OS/startup record in the existing text evaluator.
        message = message.replace('\r','\\r').replace('\n','\\n')
    level_name = {0: 'Notice', 1: 'Info', 2: 'Debug', 3: 'UserAction', 16: 'Error', 17: 'Fault'}.get(level)
    require(level_name is not None, 'Unknown target log level')
    if image_name and image_name != process_name:
        require(re.fullmatch(r'[A-Za-z0-9_.+-]+', image_name) is not None, 'Unrepresentable image name')
        process_name += '(' + image_name + ')'
    # UTC is explicit and shared by both streams. No host receipt timestamp or
    # fabricated fractional precision is substituted for device seconds/usec.
    stamp = datetime.fromtimestamp(row['seconds'], timezone.utc).strftime('%b %e %H:%M:%S')
    return (f"{stamp}.{row['microseconds']:06d} {process_name}[{row['pid']}] <{level_name}>: {message}\n").encode()


class Writer:
    def __init__(self, directory, device, filters):
        self.directory, self.device, self.filters = Path(directory), device, selection(filters)
        self.paths = {name: self.directory / suffix for name, suffix in
                      (('capture','capture.log'),('raw','records.bin'),('index','records.jsonl'))}
        self.files = {name: path.open('xb') for name, path in self.paths.items()}
        self.count, self.kept, self.types, self.ready = 0, 0, {}, False
        self.stream_hash = hashlib.sha256()
        self.previous_stamp = None
        self.failure_frame = None

    def decode(self, raw, original):
        try:
            return self._decode(raw, original)
        except BaseException:
            if self.failure_frame is None and len(raw) <= MAX_FRAME:
                path = self.directory / 'first-failing-frame.bin'
                with path.open('xb') as stream:
                    stream.write(raw)
                self.failure_frame = {**binding(path), 'bytes': len(raw),
                    'header_type': struct.unpack_from('<I',raw,1)[0] if len(raw)>=5 else None}
            raise

    def _decode(self, raw, original):
        self.count += 1
        self.stream_hash.update(struct.pack('<I', len(raw)) + raw)
        row = header(raw)
        result = compare(original(raw), row)
        self.types[str(row['type'])] = self.types.get(str(row['type']), 0) + 1
        require(not LOSS.search(row['message']), 'Device stream reports lost messages')
        if not self.ready:
            self.files['capture'].write(('[connected:' + self.device + ']\n').encode())
            self.files['capture'].flush()
            self.ready = True
        if not selected(row, self.filters):
            return result
        stamp = (row['seconds'], row['microseconds'])
        require(self.previous_stamp is None or stamp >= self.previous_stamp, 'Target timestamp moved backwards')
        line = format_record(row, raw[68])
        self.previous_stamp = stamp
        raw_offset, text_offset = self.files['raw'].tell(), self.files['capture'].tell()
        require(raw_offset + 4 + len(raw) <= MAX_RAW and text_offset + len(line) <= MAX_CAPTURE, 'Target capture cap reached')
        item = {'sequence': self.kept, 'stream_sequence': self.count, 'raw_offset': raw_offset,
                'raw_length': len(raw), 'raw_sha256': sha(raw), 'text_offset': text_offset,
                'text_length': len(line), 'text_sha256': sha(line), 'type': row['type'],
                'pid': row['pid'], 'seconds': row['seconds'], 'microseconds': row['microseconds']}
        encoded = (json.dumps(item, sort_keys=True, separators=(',',':')) + '\n').encode()
        require(self.files['index'].tell() + len(encoded) <= MAX_INDEX, 'Record index cap reached')
        # Preserve exact source bytes before exposing its rendered record live.
        self.files['raw'].write(struct.pack('<I', len(raw)) + raw); self.files['raw'].flush()
        self.files['index'].write(encoded); self.files['index'].flush()
        self.files['capture'].write(line); self.files['capture'].flush()
        self.kept += 1
        return result

    def finish(self, intentional):
        if intentional and self.ready:
            self.files['capture'].write(('[disconnected:' + self.device + ']\n').encode())
        for stream in self.files.values():
            stream.close()
        return {'device': self.device, 'selection': self.filters, 'ready': self.ready,
                'complete_frames': self.count, 'selected_frames': self.kept, 'record_types': self.types,
                'failure_frame': self.failure_frame,
                'framed_stream_sha256': self.stream_hash.hexdigest(), 'files': {k: binding(p) for k,p in self.paths.items()}}


def replay(receipt, *, expected_device, expected_selection, expected_transport, expected_source):
    """Bind and re-render every selected raw record; never trust its index alone."""
    require(receipt.get('schema') == 'iris-ostrace-logger-v1' and receipt.get('passed') is True
            and receipt.get('intentional_signal') == 'SIGTERM' and receipt.get('connections_closed') is True
            and receipt.get('closure_errors') == [] and receipt.get('transport') == expected_transport
            and receipt.get('source') == expected_source, 'Logger did not finish the reviewed live stream')
    bound(expected_transport); bound(expected_source)
    record = receipt['records']
    require(record['device'] == expected_device and record['selection'] == selection(expected_selection)
            and record['ready'] is True, 'Wrong raw-record target')
    paths = record['files']
    require(Path(paths['raw']['path']).stat().st_size <= MAX_RAW
            and Path(paths['index']['path']).stat().st_size <= MAX_INDEX
            and Path(paths['capture']['path']).stat().st_size <= MAX_CAPTURE + 128, 'Replay input cap exceeded')
    raw, index, text = (bound(paths[name]) for name in ('raw','index','capture'))
    require(not index or index.endswith(b'\n'), 'Truncated record index')
    rows = [json.loads(line) for line in index.splitlines()]
    output = ('[connected:' + expected_device + ']\n').encode()
    offset, previous_stamp, previous_sequence = 0, None, 0
    for sequence, item in enumerate(rows):
        require(len(raw) - offset >= 4, 'Truncated raw frame length')
        length = struct.unpack_from('<I', raw, offset)[0]
        require(129 <= length <= MAX_FRAME and offset + 4 + length <= len(raw), 'Truncated raw frame')
        data = raw[offset+4:offset+4+length]
        checked = header(data)
        require(selected(checked, expected_selection), 'Raw record is outside selection')
        rendered = format_record(checked, data[68])
        stamp = (checked['seconds'], checked['microseconds'])
        require(previous_stamp is None or stamp >= previous_stamp, 'Replay clock reversed')
        require(type(item.get('stream_sequence')) is int and item['stream_sequence'] > previous_sequence, 'Stream sequence reordered')
        expected = {'sequence': sequence, 'stream_sequence': item['stream_sequence'], 'raw_offset': offset,
                    'raw_length': length, 'raw_sha256': sha(data), 'text_offset': len(output),
                    'text_length': len(rendered), 'text_sha256': sha(rendered), 'type': checked['type'],
                    'pid': checked['pid'], 'seconds': checked['seconds'], 'microseconds': checked['microseconds']}
        require(item == expected, 'Raw-record index mismatch')
        output += rendered
        offset += 4 + length
        previous_stamp, previous_sequence = stamp, item['stream_sequence']
    output += ('[disconnected:' + expected_device + ']\n').encode()
    require(offset == len(raw) and output == text and len(rows) == record['selected_frames']
            and record['complete_frames'] >= previous_sequence, 'Raw records do not reproduce complete capture')
    return {'selected_frames': len(rows), 'capture': paths['capture'], 'raw': paths['raw'], 'index': paths['index']}


async def run_logger(config, out, device, filters, report):
    lockdown_module, trace_module = load_transport(config)
    writer = Writer(out, device, filters)
    lockdown, trace, stream = None, None, None
    original = trace_module.parse_syslog_entry
    current = asyncio.current_task()
    requested = []
    loop = asyncio.get_running_loop()
    def stop(sig):
        requested.append(sig)
        current.cancel()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop, sig)
    closed, errors = False, []
    try:
        lockdown = await asyncio.wait_for(lockdown_module.create_using_usbmux(
            serial=device, autopair=False, connection_type='USB', usbmux_address='/var/run/usbmuxd'), 8)
        require(lockdown.udid == device and lockdown.paired, 'Wrong or unpaired device')
        trace = trace_module.OsTraceService(lockdown)
        await asyncio.wait_for(trace.connect(), 6)
        real_receive = trace.service.recvall
        async def limited_receive(length):
            require(type(length) is int and 0 < length <= MAX_FRAME, 'Oversized transport frame')
            return await real_receive(length)
        trace.service.recvall = limited_receive
        trace_module.parse_syslog_entry = lambda raw: writer.decode(raw, original)
        stream = trace.syslog(pid=filters['pid'] if filters['pid'] is not None else -1,
                              message_filter=0xFFFF, stream_flags=0x3C)
        async with asyncio.timeout(600):
            async for entry in stream:
                require(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss <= 256*1024**2, 'Logger memory cap reached')
    except asyncio.CancelledError:
        require(requested == [signal.SIGTERM], 'Logger interrupted without one intentional SIGTERM')
    finally:
        for sig in (signal.SIGTERM, signal.SIGINT):
            loop.remove_signal_handler(sig)
            signal.signal(sig, signal.SIG_IGN)
        trace_module.parse_syslog_entry = original
        for name, value, method in [('stream',stream,'aclose'),('trace',trace,'close'),('lockdown',lockdown,'close')]:
            if value is not None:
                try: await asyncio.wait_for(getattr(value, method)(), 3)
                except BaseException as error: errors.append(name+':'+type(error).__name__)
        closed = not errors
        intentional = requested == [signal.SIGTERM] and closed
        report.update(intentional_signal='SIGTERM' if requested == [signal.SIGTERM] else None,
                      connections_closed=closed, closure_errors=errors, records=writer.finish(intentional))
        require(closed, 'Logger connection closure failed')
    require(intentional and writer.ready, 'Logger exited before intentional close')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--transport', required=True); p.add_argument('--transport-sha256', required=True)
    p.add_argument('--device', required=True); p.add_argument('--output', required=True)
    p.add_argument('--pid', type=int); p.add_argument('--process-name'); p.add_argument('--match', action='append', required=True)
    a = p.parse_args(); os.umask(0o077)
    filters = selection({'pid': a.pid, 'process_name': a.process_name, 'messages': a.match})
    require(re.fullmatch(r'[0-9A-Fa-f]{8}-[0-9A-Fa-f]{16}|[0-9A-Fa-f]{40}', a.device) is not None, 'Invalid device')
    transport = {'path': a.transport, 'sha256': a.transport_sha256}
    config = json.loads(bound(transport))
    require(set(config) == {'schema','site_packages','source_pins','python'}
            and config['schema'] == 'iris-ostrace-transport-v1', 'Wrong transport manifest')
    bound(config['python'])
    require(Path(sys.executable).resolve() == Path(config['python']['path']), 'Wrong pinned interpreter')
    out = Path(a.output); out.mkdir(mode=0o700, exist_ok=False)
    report = {'schema':'iris-ostrace-logger-v1','passed':False,'source':binding(__file__),
              'transport':transport,'started_utc':time.time(),'automatic_retry':False}
    try:
        asyncio.run(run_logger(config, out, a.device, filters, report))
        report['passed'] = True
    except BaseException as error:
        report['failure'] = type(error).__name__ + ': ' + str(error)
        # No device records, credentials or transport response bodies are logged.
        print('Logger capture failed: ' + type(error).__name__, file=sys.stderr, flush=True)
    report['finished_utc'] = time.time()
    save(out/'receipt.json', report)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

