#!/usr/bin/env python3
"""Render the exact CI-built Compose widget without an account or native core.

The validation APK omits ARM-only native libraries so an isolated x86 emulator
can run this Java/Kotlin-only case. Every other non-signature entry is retained
byte for byte. This is not an APK release or a native-core runtime check.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import zipfile


def run(*args, **kwargs):
    return subprocess.check_output([str(a) for a in args], text=True, timeout=180, **kwargs)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def payload(archive):
    return {item.filename: hashlib.sha256(archive.read(item)).hexdigest()
            for item in archive.infolist() if not item.is_dir()
            and not item.filename.startswith(('META-INF/', 'lib/'))}


def prepare(seed_run):
    assert __debug__ and seed_run.isdecimal()
    producer = json.loads(run('gh', 'api', f'repos/irislib/iris-chat-rs/actions/runs/{seed_run}'))
    assert producer['conclusion'] == 'success'
    source = producer['head_sha']
    assert re.fullmatch('[a-f0-9]{40}', source)
    jobs = json.loads(run('gh', 'api', f'repos/irislib/iris-chat-rs/actions/runs/{seed_run}/jobs'))['jobs']
    assert any(j['name'] == 'Android build and unit tests' and j['conclusion'] == 'success' for j in jobs)
    run('git', 'fetch', '--depth=1', 'origin', source)
    inputs = ['android', 'core', 'protocol', 'Cargo.toml', 'Cargo.lock', 'scripts/android-build',
              'scripts/android-core', 'scripts/build_common.sh', 'scripts/parallel_steps.sh']
    run('git', 'diff', '--quiet', source, 'HEAD', '--', *inputs)
    base = Path('work/android-layout'); base.mkdir(parents=True, exist_ok=False)
    run('gh', 'run', 'download', seed_run, '--repo', 'irislib/iris-chat-rs',
        '--name', 'android-native-test-packages', '--dir', base/'ci')
    sdk = Path(os.environ['ANDROID_HOME'])
    tools = max((sdk/'build-tools').iterdir(), key=lambda p: tuple(int(v) if v.isdecimal() else 0 for v in p.name.split('.')))
    key = base/'validation.keystore'
    run('keytool', '-genkeypair', '-keystore', key, '-alias', 'validation', '-storepass', 'android',
        '-keypass', 'android', '-dname', 'CN=Isolated Compose Validation', '-keyalg', 'RSA', '-validity', '1')
    rows = []
    for kind, path in [('app', 'debug/app-debug.apk'), ('test', 'androidTest/debug/app-debug-androidTest.apk')]:
        original, unsigned, aligned, signed = base/'ci'/path, base/(kind+'-unsigned.apk'), base/(kind+'-aligned.apk'), base/(kind+'.apk')
        with zipfile.ZipFile(original) as archive, zipfile.ZipFile(unsigned, 'w') as output:
            expected = payload(archive)
            omitted = [i.filename for i in archive.infolist() if i.filename.startswith('lib/')]
            for item in archive.infolist():
                if item.filename in expected: output.writestr(item, archive.read(item))
        run(tools/'zipalign', '-p', '4', unsigned, aligned)
        run(tools/'apksigner', 'sign', '--ks', key, '--ks-pass', 'pass:android', '--key-pass', 'pass:android', '--out', signed, aligned)
        run(tools/'apksigner', 'verify', signed)
        with zipfile.ZipFile(signed) as archive: assert payload(archive) == expected
        rows.append(dict(kind=kind, originalSHA256=sha(original), validationSHA256=sha(signed),
                         retainedEntrySHA256=expected, omittedNativeLibraries=omitted))
    (base/'inputs.json').write_text(json.dumps(dict(appSource=run('git','rev-parse','HEAD').strip(),
        actualApkSource=source, producerRun=int(seed_run), unchangedInputScopes=inputs, packages=rows,
        nativeCoreRuntime=False, releasePackage=False), indent=2)+'\n')


def test():
    base = Path('work/android-layout')
    run('adb', 'install', str(base/'app.apk'))
    run('adb', 'install', str(base/'test.apk'))
    output = run('adb','shell','am','instrument','-w','-r','-e','class',
        'to.iris.chat.ui.components.ChatListRowLayoutTest',
        'to.iris.chat.test/to.iris.chat.calls.NativeCallTestRunner')
    (base/'instrumentation.log').write_text(output)
    assert 'INSTRUMENTATION_CODE: -1' in output and re.search(r'OK \(1 test\)', output)
    assert 'previewsKeepTheRowAndFollowingChatInPlace' in output
    assert not any(s in output for s in ['FAILURES!!!','INSTRUMENTATION_FAILED','shortMsg='])
    (base/'result.json').write_text(json.dumps(dict(passed=True, tests=1, widths=[280,352],
        fontScales=[1,1.5], previewStates=8, unreadStates=2, nativeCoreRuntime=False,
        instrumentationSHA256=sha(base/'instrumentation.log'), inputSHA256=sha(base/'inputs.json')),
        indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['prepare','test'])
    parser.add_argument('--seed-run')
    args = parser.parse_args()
    if args.phase == 'prepare': prepare(args.seed_run)
    else: test()
