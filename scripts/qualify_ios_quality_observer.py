#!/usr/bin/env python3
"""Qualify the quality observer offline; never connect to or operate a device.

Replay retained Activity Monitor exports from an unrelated application to check
the actual schema and units. That trace is never Iris performance acceptance.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

import ios_quality_observer as observer
import ios_quality_trace as trace
from ios_ordinary_relaunch import binding, require, verify_binding


TESTS = ('test_ios_quality_observer', 'test_ios_relaunch_adapters', 'test_ios_relaunch_lifecycle',
         'test_ios_ordinary_relaunch', 'test_ios_runtime_acceptance', 'test_ios_runtime_workflow')
TEST_SOURCES = tuple(name + '.py' for name in TESTS) + ('qualify_ios_quality_observer.py',)


def replay(counters, toc, thermal, reference):
    previous = json.loads(Path(reference).read_text())
    rows = trace.counters(counters, pid=previous['pid'], process_name=previous['processName'])
    # Reproduce only the reference parser's fixed first-120-seconds selection.
    # The production observer uses its own fixed observed intervals instead.
    rows = [row for row in rows if row[0] <= rows[0][0] + 120]
    elapsed = rows[-1][0] - rows[0][0]
    cpu = rows[-1][1] - rows[0][1]
    gap = max(b[0] - a[0] for a, b in zip(rows, rows[1:]))
    require(len(rows) == previous['sampleCount'] and
            all(abs(actual - previous[key]) < 1e-8 for actual, key in
                ((elapsed, 'elapsedSeconds'), (cpu, 'cpuSeconds'),
                 (100 * cpu / elapsed, 'cpuPercent'), (gap, 'maxSampleGapSeconds'))),
            'Actual-format parser does not reproduce retained counter units')
    require(binding(counters)['sha256'] == previous['xmlSha256']
            and binding(toc)['sha256'] == previous['tocSha256'], 'Reference trace input changed')
    trace.nominal_thermal(thermal, rows[0][0], rows[-1][0])
    return {'scope': 'Unrelated app, parser/schema qualification only', 'sample_count': len(rows),
            'elapsed_seconds': elapsed, 'cpu_seconds': cpu, 'cpu_percent': 100 * cpu / elapsed,
            'maximum_sample_gap_seconds': gap, 'written_bytes_delta': rows[-1][2] - rows[0][2],
            'nominal_thermal_seconds': elapsed}


def qualify(out, tagged_validator, counters, toc, thermal, reference):
    root = Path(__file__).resolve().parent
    tagged = binding(tagged_validator)
    require(tagged['sha256'] == binding(root / 'ios_runtime_acceptance.py')['sha256'],
            'Tagged v3 validator must remain unchanged')
    checker_sources = observer.sources()
    test_sources = [binding(root / name) for name in TEST_SOURCES]
    retained = {key: binding(value) for key, value in
                (('counters', counters), ('toc', toc), ('thermal', thermal), ('reference', reference))}
    out = Path(out).resolve()
    out.mkdir(mode=0o700, parents=False, exist_ok=False)
    record = {'schema': 'iris-quality-observer-qualification-v1', 'passed': False,
              'device_actions': False, 'native_builds': False, 'release_acceptance': False,
              'checker_sources': checker_sources, 'test_sources': test_sources,
              'tagged_validator': tagged, 'python': binding(Path(sys.executable).resolve()),
              'retained_format_inputs': retained, 'started_utc': time.time()}
    try:
        command = [sys.executable, '-B', '-m', 'unittest', '-v', *TESTS]
        record['command'] = command
        result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
        (out / 'tests.log').write_bytes(result.stdout)
        record.update(exit_code=result.returncode, tests_log=binding(out / 'tests.log'))
        require(result.returncode == 0, 'Offline observer tests failed')
        record['retained_format_replay'] = replay(counters, toc, thermal, reference)
        for item in [*checker_sources, *test_sources, tagged, *retained.values()]:
            verify_binding(item)
        record['passed'] = True
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        record['failure'] = str(error)
    record['finished_utc'] = time.time()
    (out / 'qualification.json').write_text(json.dumps(record, indent=2, sort_keys=True) + '\n')
    return 0 if record['passed'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('out', 'tagged-validator', 'counters', 'toc', 'thermal', 'reference'):
        parser.add_argument('--' + key, required=True, type=Path)
    args = parser.parse_args()
    return qualify(args.out, args.tagged_validator, args.counters, args.toc, args.thermal, args.reference)


if __name__ == '__main__':
    raise SystemExit(main())
