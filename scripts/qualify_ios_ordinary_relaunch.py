#!/usr/bin/env python3
"""Qualify generic relaunch evidence checks offline; never operate a phone.

Outputs are private and new-only. A synthetic qualification is not launch,
provisioning, device or release acceptance. Optional provenance binding is only
for composing a later real runtime proof after separately qualified installation.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

from ios_ordinary_relaunch import binding, require, verify_binding


SOURCES = ('ios_ordinary_relaunch.py', 'ios_relaunch_lifecycle.py',
           'ios_relaunch_adapters.py', 'test_ios_relaunch_adapters.py', 'native_lab.py',
           'test_ios_ordinary_relaunch.py', 'test_ios_relaunch_lifecycle.py',
           'ios_runtime_acceptance.py', 'test_ios_runtime_acceptance.py',
           'test_ios_runtime_workflow.py', 'qualify_ios_ordinary_relaunch.py')
TESTS = ('test_ios_ordinary_relaunch', 'test_ios_relaunch_lifecycle', 'test_ios_relaunch_adapters',
         'test_ios_runtime_acceptance', 'test_ios_runtime_workflow')


def qualify(out, tagged_validator, provenance_qualification=None):
    root = Path(__file__).resolve().parent
    tagged = binding(tagged_validator)
    require(tagged['sha256'] == binding(root / 'ios_runtime_acceptance.py')['sha256'],
            'Tagged v3 validator must remain unchanged')
    provenance = None if provenance_qualification is None else binding(provenance_qualification)
    sources = [binding(root / name) for name in SOURCES]
    out = Path(out).resolve()
    out.mkdir(mode=0o700, parents=False, exist_ok=False)
    record = {'schema': 'iris-ordinary-relaunch-qualification-v1', 'synthetic_only': True,
              'device_actions': False, 'native_builds': False, 'release_acceptance': False,
              'sources': sources, 'tagged_validator': tagged,
              'provenance_qualification': provenance, 'python': binding(Path(sys.executable).resolve()),
              'tests': list(TESTS), 'started_utc': time.time()}
    command = [sys.executable, '-B', '-m', 'unittest', '-v', *TESTS]
    record['command'] = command
    result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
    (out / 'tests.log').write_bytes(result.stdout)
    record.update(exit_code=result.returncode, tests_log=binding(out / 'tests.log'), finished_utc=time.time())
    try:
        for source in sources:
            verify_binding(source)
        verify_binding(tagged)
        if provenance is not None:
            verify_binding(provenance)
        record['sources_unchanged'] = True
    except (OSError, ValueError) as error:
        record.update(sources_unchanged=False, failure=str(error), exit_code=1)
    (out / 'qualification.json').write_text(json.dumps(record, sort_keys=True, indent=2) + '\n')
    return record['exit_code']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--tagged-validator', required=True, type=Path)
    parser.add_argument('--provenance-qualification', type=Path)
    args = parser.parse_args()
    return qualify(args.out, args.tagged_validator, args.provenance_qualification)


if __name__ == '__main__':
    raise SystemExit(main())
