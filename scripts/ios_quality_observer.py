#!/usr/bin/env python3
"""Bounded Activity Monitor observer for an already installed production app.

Requires an accepted distribution binding and the same exact-device leases as
the ordinary-launch adapter. It never installs, signs, resets, terminates or
relaunches Iris. The one activation must resume the explicitly supplied live PID.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import time

import ios_quality_trace as evaluator
from ios_ordinary_relaunch import binding, exact, read_bound, require, verify_binding
from ios_relaunch_adapters import Capture, CoreDevice, OwnedWindow, foreground_pid


RECORD_SECONDS = 390
SAMPLE_INTERVAL_SECONDS = 1.5
SOURCE_NAMES = ('ios_quality_observer.py', 'ios_quality_trace.py', 'ios_relaunch_adapters.py',
                'ios_relaunch_lifecycle.py', 'ios_ordinary_relaunch.py', 'ios_runtime_acceptance.py', 'native_lab.py')


def sources():
    return [binding(Path(__file__).with_name(name)) for name in SOURCE_NAMES]


def save(path, value):
    with Path(path).open('x') as output:
        json.dump(value, output, indent=2, allow_nan=False)
        output.write('\n')


def ready_text(item):
    stdout, stderr = item['stdout'].read_bytes(), item['stderr'].read_bytes()
    require(len(stdout) + len(stderr) <= 1024 * 1024, 'Oversized xctrace log')
    require(not stderr, 'xctrace reported a diagnostic')
    text = stdout.decode('utf-8')
    return ('Starting recording with the Activity Monitor template. Targeting All Processes.' in text
            and 'Ctrl-C to stop the recording\n' in text)


def sample(capture, core, window, pid):
    record = {'monotonic_before': time.monotonic(), 'utc_before': time.time()}
    record['capture'] = capture.sample(pid)
    require(foreground_pid(capture.prefixes()['lifecycle_text'], window.config['bundle_id'],
                           capture.os_process['processIdentifier']) == pid, 'App is not visibly foreground')
    # Capture.sample's last CoreDevice query is the retained actual inventory.
    record['inventory'] = binding(window.out / f'coredevice-{window.serial:03d}.json')
    record.update(monotonic_after=time.monotonic(), utc_after=time.time())
    require(record['monotonic_after'] - record['monotonic_before'] <= 2.5, 'Slow foreground observation')
    return record


def collect_until(window, capture, core, pid, observations, predicate, *, deadline, boundaries=None):
    while not predicate():
        window.guard()
        require(time.monotonic() < deadline, 'Observation command exceeded its deadline')
        now = time.monotonic()
        if not observations or now - observations[-1]['monotonic_after'] >= SAMPLE_INTERVAL_SECONDS:
            row = sample(capture, core, window, pid)
            observations.append(row)
            require(len(observations) <= 500, 'Observation count exceeded its bound')
            if boundaries is not None:
                if not boundaries:
                    boundaries.append(0)
                elif len(boundaries) < 5:
                    duration = 180 if len(boundaries) == 1 else 60
                    if row['monotonic_after'] - observations[boundaries[-1]]['monotonic_after'] >= duration:
                        boundaries.append(len(observations) - 1)
        time.sleep(.02)


def record(config_binding, output, *, execute=False):
    require(execute, 'Explicit quality observation execution required')
    config = read_bound(config_binding)
    exact(config, {'schema', 'window', 'xctrace', 'pid', 'qualification'})
    require(config['schema'] == 'iris-quality-observer-input-v1'
            and type(config['pid']) is int and config['pid'] > 0, 'Invalid quality observation input')
    qualification = read_bound(config['qualification'])
    checker_sources = sources()
    require(qualification.get('schema') == 'iris-quality-observer-qualification-v1'
            and qualification.get('passed') is True
            and qualification.get('checker_sources') == checker_sources, 'Quality observer source is not qualified')
    trace_tool = verify_binding(config['xctrace'])
    window = OwnedWindow(config['window'], output, output_ceiling=512 * 1024**2)
    evidence = {'schema': 'iris-quality-capture-v1', 'candidate': window.config['candidate'],
                'configuration': config_binding, 'checker_sources': checker_sources, 'physicalAcceptance': False}
    capture = None
    try:
        core = CoreDevice(window)
        apps = core.run(['device', 'info', 'apps', '--filter', "bundleIdentifier == '" + window.config['bundle_id'] + "'"])['apps']
        require(len(apps) == 1 and apps[0].get('bundleIdentifier') == window.config['bundle_id']
                and apps[0].get('bundleVersion') == window.config['build']
                and apps[0].get('url') + window.config['process_name'] == window.config['executable'],
                'Installed quality candidate differs')
        evidence['apps_before'] = binding(window.out / f'coredevice-{window.serial:03d}.json')
        capture = Capture(window, core)
        capture.start()
        rows = core.run(['device', 'info', 'processes'])['runningProcesses']
        require(len([r for r in rows if r.get('executable') == window.config['executable']
                     and r.get('processIdentifier') == config['pid']]) == 1, 'Expected live app is absent')
        evidence['springboard_pid'] = capture.os_process['processIdentifier']
        trace_path = window.out / 'activity.trace'
        operation = {'started_utc': time.time(), 'started_monotonic': time.monotonic()}
        item = window.spawn([str(trace_tool), 'record', '--template', 'Activity Monitor', '--device', window.config['device'],
                             '--all-processes', '--time-limit', str(RECORD_SECONDS) + 's', '--no-prompt',
                             '--output', str(trace_path)], 'record')
        deadline = time.monotonic() + 15
        while not ready_text(item):
            window.guard(); capture.health()
            require(item['process'].poll() is None and time.monotonic() < deadline, 'xctrace did not become ready')
            time.sleep(.02)
        # A fixed two-second setup delay is outside every measured interval.
        # Actual exported trace dates must still prove recording had started.
        end = time.monotonic() + 2
        while time.monotonic() < end:
            window.guard(); capture.health(); time.sleep(.02)
        launched = core.run(['device', 'process', 'launch', window.config['bundle_id']])
        options = launched.get('launchOptions', {})
        exact(options, {'activatedWhenStarted', 'arguments', 'environmentVariables', 'platformSpecificOptions',
                        'startStopped', 'terminateExistingInstances', 'user'})
        require(launched.get('deviceIdentifier') == window.config['coredevice_id']
                and launched.get('process', {}).get('processIdentifier') == config['pid']
                and launched.get('process', {}).get('executable') == window.config['executable']
                and options.get('activatedWhenStarted') is True
                and options.get('terminateExistingInstances') is False and options.get('startStopped') is False
                and options.get('arguments') == [] and options.get('environmentVariables') in ({}, {'TERM': 'dumb'})
                and options.get('platformSpecificOptions') == {} and options.get('user') == {'active': True},
                'Activation was not an ordinary resume of the same process')
        operation.update(ready_utc=time.time(), ready_monotonic=time.monotonic())
        observations, boundaries = [], []
        collect_until(window, capture, core, config['pid'], observations, lambda: item['process'].poll() is not None,
                      deadline=operation['started_monotonic'] + RECORD_SECONDS + 30, boundaries=boundaries)
        window.wait(item, 1)
        require(len(boundaries) == 5, 'Recording ended before every observed interval completed')
        operation.update(finished_utc=time.time(), finished_monotonic=time.monotonic(),
                         stdout=binding(item['stdout']), stderr=binding(item['stderr']))
        evidence['recording'] = operation
        exports = {'toc': ['--toc'], 'counters': ['--xpath', '/trace-toc/run[@number="1"]/data/table[@schema="sysmon-process"]'],
                   'thermal': ['--xpath', '/trace-toc/run[@number="1"]/data/table[@schema="device-thermal-state-intervals"]']}
        for name, arguments in exports.items():
            path = window.out / (name + '.xml')
            item = window.spawn([str(trace_tool), 'export', '--input', str(trace_path), *arguments, '--output', str(path)], name)
            collect_until(window, capture, core, config['pid'], observations, lambda: item['process'].poll() is not None,
                          deadline=time.monotonic() + 45)
            window.wait(item, 1)
            require(not item['stderr'].read_bytes(), 'Trace export reported a diagnostic')
            evidence[name] = binding(path)
        after = core.run(['device', 'info', 'apps', '--filter', "bundleIdentifier == '" + window.config['bundle_id'] + "'"])['apps']
        fields = ('bundleIdentifier', 'bundleVersion', 'url')
        require(len(after) == 1 and all(after[0].get(k) == apps[0].get(k) for k in fields),
                'Installed app inventory changed during measurement')
        evidence['apps_after'] = binding(window.out / f'coredevice-{window.serial:03d}.json')
        observations.append(sample(capture, core, window, config['pid']))
        capture.mark_measurement_cutoff(capture.samples)
        window.close()
        evidence.update(observations=observations, boundaries=boundaries,
                        capture=capture.final_capture(capture.samples))
        require(read_bound(config_binding) == config and sources() == checker_sources, 'Quality inputs changed during observation')
        verify_binding(config['xctrace']); verify_binding(config['qualification'])
        path = window.out / 'capture.json'
        save(path, evidence)
        result = evaluator.evaluate(binding(path))
        save(window.out / 'cpu.json', result)
        return result
    except Exception as error:
        save(window.out / 'failure.json', {'passed': False, 'error': str(error), 'configuration': config_binding,
                                         'candidate': window.config['candidate'], 'checker_sources': checker_sources})
        raise
    finally:
        if not window.closed:
            window.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True)
    parser.add_argument('--input-sha256', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    result = record({'path': str(Path(args.input).resolve()), 'sha256': args.input_sha256}, args.output, execute=args.execute)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
