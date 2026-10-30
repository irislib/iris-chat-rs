#!/usr/bin/env python3
"""One ordinary warm foreground/suspension/resume observation, without CPU work.

Reuse the qualified capture and raw native parsers. Never terminate Iris, send a
push, alter app configuration or launch XCTest. Physical state preservation is
a separate UI followup; this evidence alone is not release acceptance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import signal
import time

import ios_ordinary_relaunch as ordinary
import ios_relaunch_adapters as adapters
import ios_relaunch_lifecycle as life
import ios_voip_background_wake as native
import ios_ostrace_capture as trace
from ios_ordinary_relaunch import require, exact, finite, binding, read_bound


def sources():
    return {name: binding(Path(__file__).with_name(name)) for name in (
        'ios_warm_lifecycle.py', 'ios_ordinary_relaunch.py', 'ios_relaunch_adapters.py',
        'ios_relaunch_lifecycle.py', 'ios_runtime_acceptance.py', 'ios_voip_background_wake.py',
        'ios_ostrace_capture.py', 'ios_ostrace_logger.py', 'native_lab.py')}


def evaluate_lifecycle(capture, initial, switch_index, resume_index, *, bundle, board_pid, proof):
    """Replay every state transition, including events between process samples."""
    raw, measured = life.measurement_captures(**{k: capture[k] for k in
        ('startup_text', 'lifecycle_text', 'samples', 'measurement_cutoff')}, device=capture['device'])
    samples = capture['samples']
    require(type(switch_index) is int and type(resume_index) is int
            and 0 < switch_index < resume_index < len(samples) - 1, 'Missing phase boundaries')
    exact(initial, {'startup', 'lifecycle'})
    for name, value in initial.items():
        exact(value, {'bytes', 'sha256'})
        life._capture(life._prefix(measured[name], value['bytes'], value['sha256'], complete=True), capture['device'])
    events = native.state_records(measured['lifecycle'], initial['lifecycle']['bytes'], bundle, board_pid)
    target = [r for r in events if r['bundle'] == bundle]
    require(target and all(r['pid'] == proof['pid'] for r in target), 'Missing or replaced warm process')
    before_switch = samples[switch_index]['lifecycle_bytes']
    before_resume = samples[resume_index]['lifecycle_bytes']
    foreground = [r for r in target if r['end_byte'] <= before_switch]
    background = [r for r in target if before_switch < r['end_byte'] <= before_resume]
    resumed = [r for r in target if r['end_byte'] > before_resume]
    # Setup may still emit NotVisible states before the first measured ACTIVE.
    # Once ACTIVE is measured, any subsequent foreground interruption is fatal.
    first_active = next((i for i, row in enumerate(foreground) if row['state'] == life.ACTIVE), None)
    require(first_active is not None and all(row['state'] in {native.BACKGROUND, life.SUSPENDED}
            for row in foreground[:first_active]), 'No measured foreground entry')
    foreground = foreground[first_active:]
    require(foreground[0]['end_byte'] <= samples[0]['lifecycle_bytes']
            and all(r['state'] == life.ACTIVE for r in foreground), 'Warm foreground was interrupted')
    require(background and all(r['state'] in {life.ACTIVE, native.BACKGROUND, life.SUSPENDED} for r in background),
            'Unexpected warm background state')
    left = suspended = False
    for row in background:
        require(not left or row['state'] != life.ACTIVE, 'Foreground flash during background observation')
        require(not suspended or row['state'] == life.SUSPENDED, 'Actual suspension was interrupted')
        left = left or row['state'] != life.ACTIVE
        suspended = suspended or row['state'] == life.SUSPENDED
    require(suspended, 'No actual suspension')
    entry = next(r for r in background if r['state'] != life.ACTIVE)
    suspension = next(r for r in background if r['state'] == life.SUSPENDED)
    require(entry['stamp'] - foreground[0]['stamp'] >= 60, 'Device foreground shorter than 60 seconds')
    activated = transitioning = False
    for row in resumed:
        require(row['state'] in {life.SUSPENDED, native.BACKGROUND, life.ACTIVE}
                and (not activated or row['state'] == life.ACTIVE), 'Resume changed state unexpectedly')
        require(not transitioning or row['state'] != life.SUSPENDED, 'Resume returned to suspension')
        transitioning = transitioning or row['state'] != life.SUSPENDED
        activated = activated or row['state'] == life.ACTIVE
    require(activated, 'No ordinary foreground resume')
    resumed_event = next(r for r in resumed if r['state'] == life.ACTIVE)
    departure = next(r for r in resumed if r['state'] != life.SUSPENDED)
    require(departure['stamp'] - suspension['stamp'] >= 135, 'Device suspension shorter than 135 seconds')
    # A new restoration or another process's marker is not a warm observation.
    marker = re.compile(re.escape(capture['process_name']) + r'(?:\([^)]*\))?\[(\d+)\] <[^>]+>: '
        r'iris\.startup phase=backgrounded restore_pending_result=false '
        r'native_restoring_observed=false app_backgrounded=true$')
    def parse_marker(line):
        if 'iris.startup ' not in line:
            return None
        match = marker.search(line)
        require(match is not None and int(match[1]) == proof['pid'], 'Unexpected warm startup marker')
        return {}
    life._records(measured['startup'], initial['startup']['bytes'], parse_marker)
    previous, sizes = None, {k: v['bytes'] for k, v in initial.items()}
    observed = {'foreground': [], 'suspended': [], 'resumed': []}
    for index, sample in enumerate(samples):
        exact(sample, life.SAMPLE_KEYS)
        now = finite(sample['observed_monotonic'])
        require(previous is None or 0 < now - previous <= life.MAX_SAMPLE_GAP_SECONDS, 'Observation gap or reversed clock')
        previous = now
        require(type(sample['pid']) is int and sample['pid'] == proof['pid'] and sample['executable'] == proof['executable']
                and sample['loggers_alive'] is True and sample['stderr'] == '', 'Process or logger health changed')
        for name, data in measured.items():
            life._prefix(data, sample[name + '_bytes'], sample[name + '_sha256'], complete=True)
            require(sample[name + '_bytes'] >= sizes[name], 'Capture shrank')
            sizes[name] = sample[name + '_bytes']
        visible = [r for r in target if r['end_byte'] <= sample['lifecycle_bytes']]
        require(visible, 'No actual foreground state before first sample')
        state = visible[-1]['state']
        if index <= switch_index:
            require(state == life.ACTIVE, 'Foreground sample was not active')
            observed['foreground'].append(now)
        elif index <= resume_index and state == life.SUSPENDED:
            observed['suspended'].append(now)
        elif index > resume_index and state == life.ACTIVE:
            observed['resumed'].append(now)
    elapsed = {}
    for phase, minimum in (('foreground', 60), ('suspended', 135), ('resumed', 15)):
        times = observed[phase]
        require(len(times) >= 2 and times[-1] - times[0] >= minimum, 'Insufficient live ' + phase + ' observation')
        elapsed[phase + '_seconds'] = times[-1] - times[0]
    require(samples[-1]['observed_monotonic'] - samples[0]['observed_monotonic'] <= life.MAX_OBSERVATION_SECONDS
            and all(sizes[k] == len(v) for k, v in measured.items()), 'Incomplete or excessive observation')
    return {**elapsed, 'process_samples': len(samples), 'unexpected_exits': 0, 'process_replacements': 0,
            'raw_sha256': {k: hashlib.sha256(v).hexdigest() for k, v in raw.items()},
            'physical_release_acceptance': False, 'preserved_state_followup_required': True}


class WarmCoreDevice(adapters.CoreDevice):
    """Only ordinary app activation and inventory; no termination capability."""
    def run(self, command):
        if command != ['device', 'process', 'launch', native.SETTINGS]:
            require(command != ordinary.launch_command(self.window.config['bundle_id'], replacement=True),
                    'Warm check cannot replace the app')
            return super().run(command)
        self.window.guard()
        output = self.window.out / f'coredevice-{self.window.serial + 1:03d}.json'
        args = [str(self.window.tools['coredevice']), *command[:3], '--device', self.window.config['device'],
                '--timeout', '6', '--json-output', str(output), *command[3:]]
        child = self.window.spawn(args, 'coredevice')
        self.window.wait(child, 7)
        require(output.stat().st_size <= 4 * 1024 * 1024, 'Oversized CoreDevice response')
        return json.loads(output.read_text())['result']


def evaluate(record):
    config = read_bound(record['configuration'])
    window = read_bound(config['window'])
    distribution = ordinary.validate_distribution(window['distribution_binding'], window['candidate'])
    qualification = read_bound(config['qualification'])
    require(qualification['passed'] is True and record['checker_sources'] == sources() == qualification['sources'], 'Warm source changed')
    require(all(window[k] == distribution[k] for k in ('device', 'coredevice_id', 'bundle_id'))
            and window['build'] == distribution['candidate']['apple_build']
            and window['executable'] == distribution['installed_url'] + window['process_name'], 'Warm target changed')
    require(record['automatic_retry'] is False, 'Warm observation retried')
    apps = read_bound(config['installed'])['result']['apps']
    require(len(apps) == 1 and apps[0]['bundleIdentifier'] == window['bundle_id']
            and apps[0]['bundleVersion'] == window['build'] and apps[0]['url'] == distribution['installed_url'],
            'Installed candidate changed')
    seed = read_bound(config['seed_launch'])['result']
    native.options(seed, distribution['coredevice_id'], activated=True, replacement=False)
    reference = native.identity(seed, window['executable'])
    require(record['process'] == reference, 'Wrong warm process')
    operations = record['operations']
    require([x['command'] for x in operations] == [
        ['device', 'process', 'launch', native.SETTINGS], ordinary.launch_command(window['bundle_id'], replacement=False),
        ['device', 'process', 'launch', native.SETTINGS], ordinary.launch_command(window['bundle_id'], replacement=False)],
        'Warm operation sequence changed')
    previous = None
    for index, operation in enumerate(operations):
        require(read_bound(operation['raw'])['result'] == operation['result'], 'Actual operation result changed')
        native.options(operation['result'], distribution['coredevice_id'], activated=True, replacement=False)
        begin, end = finite(operation['started_monotonic']), finite(operation['returned_monotonic'])
        start_utc, end_utc = finite(operation['started_utc']), finite(operation['returned_utc'])
        require(distribution['installation_finished_utc'] <= start_utc <= end_utc and 0 <= end - begin <= 8
                and abs((end_utc - start_utc) - (end - begin)) <= ordinary.HOST_CLOCK_TOLERANCE_SECONDS
                and (previous is None or previous <= begin), 'Operation clocks reversed or stale')
        previous = end
        if index in (1, 3):
            require(native.identity(operation['result'], window['executable']) == reference, 'Warm audit identity changed')
    capture = record['capture']
    require(capture['device'] == distribution['device'], 'Wrong capture device')
    require(record['trace_evidence']['transport'] == config['transport'], 'Warm transport changed')
    trace.validate_evidence(record['trace_evidence'], capture, device=distribution['device'],
                            bundle=distribution['bundle_id'], board_pid=record['board_pid'])
    require(read_bound(capture['closure_binding'])['closed'] is True, 'Owned capture not closed')
    require(read_bound(capture['closure_binding'])['measurement_cutoff_sha256'] == life.evidence_sha256(capture['measurement_cutoff']),
            'Closure does not bind cutoff')
    require(len(record['inventories']) == len(capture['samples']), 'Missing raw process observations')
    for raw, sample in zip(record['inventories'], capture['samples']):
        rows = read_bound(raw)['result']['runningProcesses']
        targets = [r for r in rows if r.get('executable') == reference['executable']]
        boards = [r for r in rows if r.get('executable', '').endswith('/SpringBoard.app/SpringBoard')]
        require(len(targets) == 1 and targets[0]['processIdentifier'] == reference['pid']
                and len(boards) == 1 and boards[0]['processIdentifier'] == record['board_pid']
                and not any('-Runner.app/' in r.get('executable', '') for r in rows), 'Raw process observation changed')
    samples = capture['samples']
    require(operations[1]['returned_monotonic'] <= samples[0]['observed_monotonic']
            and samples[record['switch_index']]['observed_monotonic'] <= operations[2]['started_monotonic']
            <= operations[2]['returned_monotonic'] <= samples[record['switch_index'] + 1]['observed_monotonic']
            and samples[record['resume_index']]['observed_monotonic'] <= operations[3]['started_monotonic']
            <= operations[3]['returned_monotonic'] <= samples[record['resume_index'] + 1]['observed_monotonic'],
            'Native operations disagree with phase boundaries')
    return evaluate_lifecycle(capture, record['initial_prefixes'], record['switch_index'], record['resume_index'],
        bundle=distribution['bundle_id'], board_pid=record['board_pid'], proof=reference)


def observe(config_binding):
    config = read_bound(config_binding)
    exact(config, {'schema', 'window', 'installed', 'seed_launch', 'qualification', 'transport'})
    require(config['schema'] == 'iris-warm-lifecycle-input-v1', 'Wrong warm configuration')
    qualification = read_bound(config['qualification'])
    require(qualification['passed'] is True and qualification['sources'] == sources(), 'Unqualified warm sources')
    window = adapters.OwnedWindow(config['window'], read_bound(config['window'])['output_root'])
    core = WarmCoreDevice(window); capture = trace.OstraceCapture(window, core, config['transport'])
    reference = native.identity(read_bound(config['seed_launch'])['result'], window.config['executable'])
    record = {'schema': 'iris-warm-lifecycle-observation-v1', 'configuration': config_binding, 'process': reference,
              'checker_sources': sources(), 'automatic_retry': False, 'operations': [], 'inventories': [], 'passed': False}
    samples = capture.samples
    failure = None
    def activate(bundle):
        operation = native.operation(core, ['device', 'process', 'launch', bundle]); record['operations'].append(operation)
        native.options(operation['result'], window.config['coredevice_id'], activated=True, replacement=False)
        if bundle == window.config['bundle_id']:
            require(native.identity(operation['result'], reference['executable']) == reference, 'Ordinary resume changed audit identity')
        return operation
    def states():
        text = capture.prefixes()['lifecycle_text'].encode()
        return [r for r in native.state_records(text, 0, window.config['bundle_id'], capture.os_process['processIdentifier'])
                if r['bundle'] == window.config['bundle_id']]
    def sample():
        value = capture.sample(reference['pid']); record['inventories'].append(native.inventory_binding(window)); return value
    def phase(desired, minimum, setup_seconds):
        begin = None; deadline = time.monotonic() + setup_seconds
        seen = len(states())
        while True:
            value = sample(); events = states()
            require(all(r['pid'] == reference['pid'] for r in events), 'Warm process was replaced')
            if begin is not None:
                require(all(r['state'] == desired for r in events[seen:]), 'Measured state was interrupted')
            seen = len(events)
            if events and events[-1]['state'] == desired:
                if begin is None: begin = value['observed_monotonic']
                if value['observed_monotonic'] - begin >= minimum: return
            require(begin is not None or time.monotonic() < deadline, 'Actual state not observed within setup bound')
            time.sleep(2)
    try:
        ordinary.validate_distribution(window.config['distribution_binding'], window.config['candidate'])
        capture.start(); record['board_pid'] = capture.os_process['processIdentifier']
        activate(native.SETTINGS)
        setup_deadline = time.monotonic() + 8
        while not states() or states()[-1]['state'] == life.ACTIVE:
            require(time.monotonic() < setup_deadline, 'No initial background transition'); time.sleep(.05)
        record['initial_prefixes'] = life.capture_prefixes(**capture.prefixes(), device=window.config['device'])
        activate(window.config['bundle_id'])
        ready = time.monotonic() + 8
        while not states() or states()[-1]['state'] != life.ACTIVE:
            require(time.monotonic() < ready, 'No foreground transition'); time.sleep(.05)
        phase(life.ACTIVE, 61, 8); record['switch_index'] = len(samples) - 1
        activate(native.SETTINGS)
        phase(life.SUSPENDED, 136, 45); record['resume_index'] = len(samples) - 1
        activate(window.config['bundle_id'])
        phase(life.ACTIVE, 16, 8)
        capture.mark_measurement_cutoff(samples)
    except BaseException as error:
        failure = type(error).__name__ + ': ' + str(error); record['failure'] = failure
    finally:
        try: window.close()
        except BaseException as error:
            record['closure_failure'] = str(error); failure = failure or str(error)
        record['samples'] = samples
        native.save(window.out / 'journal.json', record)
    require(failure is None, failure)
    return finish_observation(record, capture, window)


def finish_observation(record, capture, window):
    # Preserve the actual cutoff and raw replay bindings before offline checks.
    # A checker exception must not erase otherwise completed native evidence.
    record['capture'] = {**capture.final_capture(record['samples']), 'device': window.config['device']}
    record['trace_evidence'] = capture.trace_evidence
    native.save(window.out / 'observed-capture.json', record)
    record['evaluation'] = evaluate(record); record['passed'] = True
    native.save(window.out / 'capture.json', record)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True); parser.add_argument('--input-sha256', required=True)
    parser.add_argument('--execute', action='store_true'); args = parser.parse_args()
    require(args.execute, 'Explicit reviewed execution required')
    def interrupted(number, frame): raise KeyboardInterrupt('Owner stopped warm observation')
    signal.signal(signal.SIGTERM, interrupted)
    observe({'path': str(args.input), 'sha256': args.input_sha256})


if __name__ == '__main__': main()
