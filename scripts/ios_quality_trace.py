#!/usr/bin/env python3
"""Read-only validation of real Activity Monitor counters and live observations.

This is a newly qualified measurement method, not a reconstruction of the old
XCTest observer. All windows are fixed before their CPU values are inspected.
"""
from __future__ import annotations

from datetime import datetime
import hashlib
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET

import ios_relaunch_lifecycle as lifecycle
from ios_relaunch_adapters import foreground_pid
from ios_ordinary_relaunch import read_bound, require, verify_binding, validate_distribution


MAX_XML_BYTES = 256 * 1024**2


def finite(value):
    require(type(value) in (int, float) and math.isfinite(value), 'Invalid measurement clock')
    return value


def integer(element, kind):
    require(element.tag == kind and element.text is not None and element.text.isdecimal(),
            'Missing, sentinel or malformed numeric trace value')
    return int(element.text)


def xml(path):
    path = Path(path)
    require(path.stat().st_size <= MAX_XML_BYTES, 'Trace export exceeds its bound')
    data = path.read_bytes()
    require(b'<!DOCTYPE' not in data and b'<!ENTITY' not in data, 'Unsupported trace XML declarations')
    return ET.fromstring(data)


def table(path, expected):
    root = xml(path)
    schemas = root.findall('.//schema')
    require(len(schemas) == 1 and schemas[0].get('name') == expected, 'Wrong trace table')
    columns = [c.findtext('mnemonic') for c in schemas[0].findall('col')]
    require(columns and None not in columns and len(set(columns)) == len(columns), 'Invalid trace columns')
    ids = {}
    for element in root.iter():
        key = element.get('id')
        if key is not None:
            require(key not in ids, 'Duplicate trace reference')
            ids[key] = element

    def resolve(element):
        seen = set()
        while element.get('ref') is not None:
            key = element.get('ref')
            require(key not in seen and key in ids, 'Missing or cyclic trace reference')
            seen.add(key)
            element = ids[key]
        return element

    rows = []
    for row in root.findall('.//row'):
        require(len(row) == len(columns), 'Malformed trace row')
        rows.append({key: resolve(value) for key, value in zip(columns, row)})
    return rows


def recording(path, *, device, started, ready, finished):
    root = xml(path)
    runs = root.findall('run')
    require(len(runs) == 1, 'Expected exactly one recording run')
    target = runs[0].find('info/target/device')
    require(target is not None and target.get('platform') == 'iOS'
            and target.get('uuid', '').lower() == device.lower(), 'Wrong physical trace device')
    summary = runs[0].find('info/summary')
    require(summary is not None and summary.findtext('template-name') == 'Activity Monitor'
            and summary.findtext('end-reason') == 'Time limit reached', 'Incomplete Activity Monitor recording')
    dates = [datetime.fromisoformat(summary.findtext(key, '')) for key in ('start-date', 'end-date')]
    require(all(d.tzinfo is not None for d in dates), 'Trace dates lack a timezone')
    start, end = (d.timestamp() for d in dates)
    duration = float(summary.findtext('duration', 'nan'))
    require(finite(duration) > 0 and abs((end - start) - duration) <= .002, 'Inconsistent trace clock')
    # xctrace reports millisecond wall dates. These must fit the actual host
    # spawn/readiness/finish observations, not a caller-selected offset.
    require(started - .002 <= start <= ready + .002 and end <= finished + .002,
            'Trace clock does not fit the actual recording operation')
    return start, duration


def counters(path, *, pid, process_name):
    rows = table(path, 'sysmon-process')
    needed = {'time', 'process', 'pid', 'recently-died', 'cpu-total-system', 'cpu-total-user', 'disk-bytes-written'}
    require(rows and needed <= rows[0].keys(), 'Missing process counter columns')
    samples, identities = [], set()
    for row in rows:
        if row['pid'].tag == 'sentinel':
            continue
        if integer(row['pid'], 'pid') != pid:
            continue
        process = row['process']
        require(process.tag == 'process' and process.get('fmt') == f'{process_name} ({pid})'
                and process.get('id'), 'Trace PID belongs to another process')
        identities.add(process.get('id'))
        require(integer(row['recently-died'], 'boolean') == 0, 'Target process exited')
        samples.append((integer(row['time'], 'start-time') / 1e9,
                        (integer(row['cpu-total-system'], 'duration-on-core')
                         + integer(row['cpu-total-user'], 'duration-on-core')) / 1e9,
                        integer(row['disk-bytes-written'], 'size-in-bytes')))
    require(len(identities) == 1 and len(samples) >= 2, 'Process identity changed or counters are absent')
    require(all(b[0] > a[0] and b[1] >= a[1] and b[2] >= a[2] for a, b in zip(samples, samples[1:])),
            'Counter reset or trace time reversed')
    return samples


def nominal_thermal(path, start, end):
    rows = table(path, 'device-thermal-state-intervals')
    needed = {'start', 'duration', 'end', 'thermal-state', 'track-label', 'is-induced'}
    require(rows and needed <= rows[0].keys(), 'Missing thermal intervals')
    covered, last = start, None
    for row in rows:
        a, duration, b = (integer(row[key], kind) / 1e9 for key, kind in
                          (('start', 'start-time'), ('duration', 'duration'), ('end', 'start-time')))
        require(b > a and abs(b - a - duration) < 1e-6 and (last is None or a >= last),
                'Thermal intervals overlap or have invalid clocks')
        last = b
        if b <= start or a >= end:
            continue
        require(row['thermal-state'].tag == 'thermal-state' and row['thermal-state'].text == 'Nominal'
                and row['track-label'].tag == 'string' and row['track-label'].text == 'Current'
                and integer(row['is-induced'], 'boolean') == 0, 'Thermal state was not naturally nominal')
        require(a <= covered + 1e-9, 'Thermal observation has a gap')
        covered = max(covered, b)
    require(covered >= end, 'Thermal evidence does not cover the measurement')


def observe(records, capture, *, device, bundle, pid, executable, springboard_pid):
    require(isinstance(records, list) and 2 <= len(records) <= 512, 'Missing bounded host observations')
    samples = [record['capture'] for record in records]
    _, live = lifecycle.measurement_captures(device=device, samples=samples,
        startup_text=capture['startup_text'], lifecycle_text=capture['lifecycle_text'],
        measurement_cutoff=capture['measurement_cutoff'])
    # measurement_captures returns the exact last-live prefixes; shutdown is
    # independently checked and never extends measured duration.
    data = live['lifecycle']
    first_clock, previous, maximum_gap = None, None, 0
    for index, record in enumerate(records):
        a, b = finite(record['monotonic_before']), finite(record['monotonic_after'])
        wa, wb = finite(record['utc_before']), finite(record['utc_after'])
        require(a <= record['capture']['observed_monotonic'] <= b and a <= b,
                'Observation clocks are reordered')
        require(abs((wb - wa) - (b - a)) <= .25, 'Wall clock jumped during observation')
        if first_clock is None:
            first_clock = (a, wa)
        require(abs((wa - first_clock[1]) - (a - first_clock[0])) <= .25
                and abs((wb - first_clock[1]) - (b - first_clock[0])) <= .25, 'Host clocks disagree')
        if previous is not None:
            require(a >= previous[0] and b > previous[1], 'Observation clock reversed')
            # Count command time as an observation gap too. Never stamp a slow
            # query at its midpoint to disguise missing foreground coverage.
            maximum_gap = max(maximum_gap, b - previous[1], b - a)
        else:
            maximum_gap = b - a
        require(maximum_gap <= 2.5, 'Foreground observation gap exceeds 2.5 seconds')
        previous = (a, b)
        inventory = read_bound(record['inventory'], limit=4 * 1024**2)['result']['runningProcesses']
        matching = [r for r in inventory if r.get('executable') == executable]
        require(len(matching) == 1 and matching[0].get('processIdentifier') == pid, 'Observed app process changed')
        require(any(r.get('processIdentifier') == springboard_pid and r.get('executable', '').endswith('/SpringBoard.app/SpringBoard')
                    for r in inventory), 'Observed SpringBoard process changed')
        require(not any('-Runner.app/' in r.get('executable', '') for r in inventory), 'XCTest runner present')
        sample = record['capture']
        require(set(sample) == lifecycle.SAMPLE_KEYS and type(sample['pid']) is int
                and sample['pid'] == pid and sample['executable'] == executable
                and sample['loggers_alive'] is True and sample['stderr'] == '', 'Capture target or health changed')
        lifecycle._prefix(live['startup'], sample['startup_bytes'], sample['startup_sha256'], complete=True)
        if index:
            prior = records[index - 1]['capture']
            require(all(sample[name + '_bytes'] >= prior[name + '_bytes'] for name in ('startup', 'lifecycle')),
                    'Observed log prefix moved backwards')
        prefix = lifecycle._prefix(data, sample['lifecycle_bytes'], sample['lifecycle_sha256'], complete=True)
        require(foreground_pid(prefix, bundle, springboard_pid) == pid, 'App was not observed in the foreground')
    # Reject an intervening background transition even when the next poll sees
    # foreground again. This is still syslog evidence, not a claim that every OS
    # transition is guaranteed to be logged.
    first = samples[0]['lifecycle_bytes']
    for line in data[first:].decode().splitlines():
        if 'Received state update for ' in line and bundle in line:
            require(foreground_pid(line + '\n', bundle, springboard_pid) == pid,
                    'Intervening nonforeground app transition')
    return maximum_gap


def evaluate(evidence_binding):
    evidence = read_bound(evidence_binding, limit=4 * 1024**2)
    require(evidence.get('schema') == 'iris-quality-capture-v1', 'Wrong quality evidence schema')
    configuration = read_bound(evidence['configuration'])
    qualification = read_bound(configuration['qualification'])
    require(qualification.get('schema') == 'iris-quality-observer-qualification-v1'
            and qualification.get('passed') is True
            and qualification.get('checker_sources') == evidence['checker_sources'], 'Unqualified quality checker')
    for source in evidence['checker_sources']:
        verify_binding(source)
    window = read_bound(configuration['window'])
    candidate = window['candidate']
    validate_distribution(window['distribution_binding'], candidate)
    require(evidence['candidate'] == candidate, 'Quality candidate changed')
    for key in ('apps_before', 'apps_after'):
        apps = read_bound(evidence[key], limit=4 * 1024**2)['result']['apps']
        require(len(apps) == 1 and apps[0].get('bundleIdentifier') == window['bundle_id']
                and apps[0].get('bundleVersion') == candidate['apple_build']
                and apps[0].get('url') + window['process_name'] == window['executable'],
                'Retained installed-app inventory does not match candidate')
    closure = read_bound(evidence['capture']['closure_binding'])
    cutoff = evidence['capture']['measurement_cutoff']
    require(closure.get('closed') is True and closure.get('failures') == []
            and closure.get('candidate') == candidate and closure.get('device') == window['device']
            and closure.get('measurement_cutoff_sha256') == lifecycle.evidence_sha256(cutoff), 'Capture did not close cleanly')
    records = evidence['observations']
    gap = observe(records, evidence['capture'], device=window['device'], bundle=window['bundle_id'],
                  pid=configuration['pid'], executable=window['executable'], springboard_pid=evidence['springboard_pid'])
    boundaries = evidence['boundaries']
    require(len(boundaries) == 5 and boundaries[0] == 0 and all(type(i) is int for i in boundaries)
            and all(a < b < len(records) for a, b in zip(boundaries, boundaries[1:])), 'Invalid fixed measurement boundaries')
    durations = []
    for index, (a, b) in enumerate(zip(boundaries, boundaries[1:])):
        target = 180 if index == 0 else 60
        elapsed = records[b]['monotonic_after'] - records[a]['monotonic_after']
        require(target - .1 <= elapsed <= target + 3, 'Observed interval is truncated or stalled')
        require(records[b - 1]['monotonic_after'] - records[a]['monotonic_after'] < target,
                'Measurement skipped an earlier eligible boundary')
        durations.append(elapsed)
    operation = evidence['recording']
    first = records[0]
    for name in ('started', 'ready', 'finished'):
        utc, monotonic = finite(operation[name + '_utc']), finite(operation[name + '_monotonic'])
        require(abs((utc - first['utc_before']) - (monotonic - first['monotonic_before'])) <= .25,
                'Recording and observation clocks disagree')
    require(operation['started_monotonic'] < operation['ready_monotonic'] <= records[0]['monotonic_before']
            < records[boundaries[-1]]['monotonic_after'] <= operation['finished_monotonic'],
            'Recording operation does not cover observed measurement')
    recording_log = verify_binding(operation['stdout']).read_text()
    require(not verify_binding(operation['stderr']).read_bytes()
            and 'Starting recording with the Activity Monitor template. Targeting All Processes.' in recording_log
            and 'Reached specified time limit, ending recording...' in recording_log
            and 'Recording completed. Saving output file...' in recording_log,
            'Recording did not retain its successful command output')
    start, trace_duration = recording(verify_binding(evidence['toc']), device=window['device'],
        started=operation['started_utc'], ready=operation['ready_utc'], finished=operation['finished_utc'])
    samples = counters(verify_binding(evidence['counters']), pid=configuration['pid'], process_name=window['process_name'])
    first_time = records[0]['utc_after'] - start
    end_time = records[boundaries[-1]]['utc_after'] - start
    require(0 <= first_time < end_time <= trace_duration, 'Trace does not cover the observed interval')
    nominal_thermal(verify_binding(evidence['thermal']), first_time, end_time)
    process_gap = max([gap] + [b[0] - a[0] for a, b in zip(samples, samples[1:])
                               if a[0] < end_time and b[0] > first_time])
    require(process_gap <= 8, 'Process counter observation gap exceeds 8 seconds')
    windows = []
    for index in (2, 3):
        a, b = boundaries[index:index + 2]
        relative_start, relative_end = records[a]['utc_after'] - start, records[b]['utc_after'] - start
        selected = [s for s in samples if relative_start <= s[0] <= relative_end]
        require(len(selected) >= 2, 'Missing metric interval')
        elapsed = selected[-1][0] - selected[0][0]
        observed = durations[index]
        require(57 <= elapsed <= 72 and abs(observed - elapsed) <= 2, 'Metric clock does not cover its observed window')
        cpu = 100 * (selected[-1][1] - selected[0][1]) / elapsed
        writes = (selected[-1][2] - selected[0][2]) / 1024**2 * 60 / elapsed
        require(0 <= cpu <= 5 and 0 <= writes <= 5, 'Retained CPU or disk-write budget exceeded')
        windows.append({'observed_seconds': observed, 'metric_seconds': elapsed,
                        'cpu_percent_one_core': cpu, 'disk_write_mib_per_minute': writes})
    return {'build': candidate['apple_build'], 'status': 'passed', 'method': 'installed-foreground-cpu-v1',
            'evidence_sha256': evidence_binding['sha256'], 'qualification_sha256': configuration['qualification']['sha256'],
            'warmup_seconds': durations[0], 'discarded_interval_seconds': durations[1], 'windows': windows,
            'foreground_sample_max_gap_seconds': gap, 'process_sample_max_gap_seconds': process_gap,
            'invalid_observations': 0, 'process_replacements': 0, 'thermal_state': 'nominal'}
