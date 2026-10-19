#!/usr/bin/env python3
"""Reuse an exact CI-produced core for Swift-only tests, never release packages."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

INPUTS = ['core', 'protocol', 'Cargo.toml', 'Cargo.lock', 'scripts/ios-build',
          'scripts/build_common.sh', 'scripts/parallel_steps.sh', 'ios/project.yml']
if not __debug__: raise RuntimeError('Core cache reuse requires active provenance checks')


def cache_proof(jobs, caches, key, ref):
    job = next(j for j in jobs if j['name'] == 'iOS simulator build and composer tests')
    steps = {step['name']: step for step in job['steps']}
    build, save = steps['iOS simulator build'], steps['Save successful iOS compiler cache']
    assert build['conclusion'] == save['conclusion'] == 'success', 'Producer build/cache save did not succeed'
    timestamp = lambda value: dt.datetime.fromisoformat(value.replace('Z', '+00:00'))
    start, end = timestamp(save['started_at']), timestamp(save['completed_at'])
    matches = [c for c in caches if c['key'] == key and c['ref'] == ref and
               start-dt.timedelta(seconds=5) <= timestamp(c['created_at']) <= end+dt.timedelta(seconds=5)]
    assert len(matches) == 1, 'Restored cache has no unique source-bound producer'
    return matches[0]


def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))


def run(command):
    subprocess.run([str(value) for value in command], check=True, timeout=120)


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024*1024), b''): value.update(block)
    return value.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--producer-run', required=True)
    args = parser.parse_args()
    assert args.producer_run.isdecimal() and os.environ['IOS_CORE_CACHE_HIT'] == 'true'
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)
    producer = api('repos/irislib/iris-chat-rs/actions/runs/' + args.producer_run)
    seed = producer['head_sha']
    assert re.fullmatch('[0-9a-f]{40}', seed) and producer['event'] == 'workflow_dispatch'
    assert producer['head_branch'] == os.environ['GITHUB_REF_NAME']
    jobs = api(f'repos/irislib/iris-chat-rs/actions/runs/{args.producer_run}/jobs')['jobs']
    caches = api('repos/irislib/iris-chat-rs/actions/caches?per_page=100&ref=refs/heads/' + producer['head_branch'])['actions_caches']
    proof = cache_proof(jobs, caches, os.environ['IOS_CORE_CACHE_KEY'], os.environ['GITHUB_REF'])
    run(['git', 'fetch', '--depth=1', 'origin', seed])
    run(['git', 'diff', '--quiet', seed, 'HEAD', '--', *INPUTS])
    target = Path(os.environ['CARGO_TARGET_DIR']).resolve()
    binary = target/'debug/uniffi-bindgen'
    dynamic = target/'debug/libiris_chat_core.dylib'
    library = target/'aarch64-apple-ios-sim/debug/libiris_chat_core.a'
    files = [binary, dynamic, library]
    assert all(p.is_file() and p.stat().st_size > 1024 for p in files)
    digests = {str(p.relative_to(target)): digest(p) for p in files}
    bindings, headers, framework = root/'ios/Bindings', root/'ios/.build/headers', root/'ios/Frameworks/IrisChatCore.xcframework'
    for directory in [bindings, headers, framework]:
        assert directory.resolve().is_relative_to(root/'ios')
        if directory.exists(): shutil.rmtree(directory)
    bindings.mkdir(parents=True); headers.mkdir(parents=True); framework.parent.mkdir(parents=True, exist_ok=True)
    run([binary, 'generate', '--library', dynamic, '--language', 'swift', '--out-dir', bindings,
         '--config', root/'core/uniffi.toml'])
    for pattern in ['*.h', '*.modulemap']:
        matches = list(bindings.glob(pattern)); assert len(matches) == 1
        shutil.copy2(matches[0], headers/('module.modulemap' if pattern.endswith('modulemap') else matches[0].name))
    run(['xcodebuild', '-create-xcframework', '-library', library, '-headers', headers, '-output', framework])
    record = {'appSource': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'actualCoreSource': seed, 'producerRun': int(args.producer_run), 'cache': proof,
              'coreInputScopesEqual': INPUTS, 'cachedFilesSHA256': digests,
              'buildMetadata': 'Core retains producer version/SHA/timestamp; this is Swift-only development validation.',
              'releasePackage': False, 'producerTestSuccessClaimed': False}
    destination = root/'work/clipboard'; destination.mkdir(parents=True, exist_ok=True)
    (destination/'ios-core-reuse.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps(record, indent=2))


if __name__ == '__main__': main()
