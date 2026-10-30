#!/usr/bin/env python3
"""Read-only release acceptance: bind real upgrade evidence to the published APK."""
import argparse
import base64
import hashlib
import shutil
import sqlite3
import subprocess
import tempfile
from contextlib import closing
from functools import lru_cache
import json
import re
from pathlib import Path
from android_upgrade_state import compare, sha256, inspect
from android_upgrade_release import pair

CHECKS = {'previous_fixture', 'in_place_contents', 'contact_edit_background', 'cold_restart', 'no_crashes'}


@lru_cache(maxsize=1)
def reviewed_inputs():
    """Bind execution to committed runner contents, not this validator's revision."""
    root = Path(__file__).resolve().parents[1]
    def committed(name):
        return subprocess.check_output(['git', '-C', str(root), 'show', 'HEAD:' + name])
    names = ('android_upgrade_release.py', 'android_upgrade_state.py', 'android_upgrade_ui.py',
             'android_saved_history.py', 'test_android_release_upgrade.py')
    hashes = {name: hashlib.sha256(committed('scripts/' + name)).hexdigest() for name in names}
    baseline = committed('test-fixtures/android-upgrade/releases.json')
    return hashes, hashlib.sha256(baseline).hexdigest(), json.loads(baseline)['releases']


def provenance(data, folder):
    hashes, baseline_hash, pins = reviewed_inputs()
    if data.get('runner_sha256') != hashes or data.get('baseline_sha256') != baseline_hash:
        raise ValueError('Executed runner or baseline does not match the reviewed committed inputs')
    previous = data['previous']
    pin = pins.get(previous['tag'])
    if not pin or any(previous.get(key) != value for key, value in pin.items()):
        raise ValueError('Previous release does not match its retained immutable pins')
    for label in ('previous', 'candidate'):
        record = data[label]
        badging = (folder / (label + '-badging.txt')).read_text()
        package = re.search(r"^package: name='([^']+)' versionCode='(\d+)' versionName='([^']+)'", badging)
        if not package or package[1] != 'to.iris.chat' or int(package[2]) != record['version_code'] or package[3] != record['tag'].removeprefix('v'):
            raise ValueError('Retained APK package/version verification does not match the receipt')
        signature = (folder / (label + '-signature.txt')).read_text()
        signers = re.findall(r'^Signer #\d+ certificate SHA-256 digest: ([0-9a-f]+)$', signature, re.M)
        if signers != [record['signer_sha256']]:
            raise ValueError('Retained APK signature verification does not match the receipt')
        for kind, suffix in (('apk', '-android-arm64.apk'), ('manifest', '-manifest.json')):
            result = json.loads((folder / (label + '-' + kind + '-attestation.json')).read_text())
            verification = result['verificationResult']; statement = verification['statement']
            signed = result['attestation']['bundle']['dsseEnvelope']['payload']
            if json.loads(base64.b64decode(signed, validate=True)) != statement:
                raise ValueError('Attestation verification differs from its signed statement')
            predicate = statement['predicate']
            if verification['signature']['certificate']['subjectAlternativeName'] != 'https://dotcom.releases.github.com' or statement['predicateType'] != 'https://in-toto.io/attestation/release/v0.2' or predicate.get('repository') != 'irislib/iris-chat-rs' or predicate.get('tag') != record['tag']:
                raise ValueError('Retained release attestation has a different identity or release')
            subjects = [item for item in statement['subject'] if item.get('name') == 'iris-chat-' + record['tag'] + suffix]
            if len(subjects) != 1 or subjects[0].get('digest', {}).get('sha256') != record[kind + '_sha256']:
                raise ValueError('Retained release attestation does not bind the exact asset')


def database_state(folder, files):
    """Read original DB/WAL bytes; open a private copy read-only to preserve evidence.

    SQLite may create a shared-memory sidecar when reading a WAL. Never open the
    original evidence for SQLite, and never use immutable=1 (which ignores WAL).
    """
    if (folder / 'core.sqlite3-wal').exists() and 'core.sqlite3-wal' not in files:
        raise ValueError('Unrecorded SQLite WAL would change the saved state')
    with tempfile.TemporaryDirectory(prefix='iris-upgrade-validation-') as temporary:
        work = Path(temporary)
        for name, digest in files.items():
            source = folder / name
            if source.stat().st_size > 16 * 1024**2:
                raise ValueError('Saved database exceeds the synthetic fixture bound')
            shutil.copyfile(source, work / name)
            if sha256(work / name) != digest:
                raise ValueError('Database evidence changed while being read')
        try:
            with closing(sqlite3.connect((work / 'core.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                db.execute('PRAGMA query_only=ON')
                if db.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
                    raise ValueError('Retained database integrity check failed')
                return inspect(db)
        except sqlite3.Error as exc:
            raise ValueError('Invalid retained SQLite database: ' + folder.name) from exc


def validate(receipt_path, manifest_path, tag):
    path = Path(receipt_path).resolve()
    data = json.loads(path.read_text())
    manifest = json.loads(Path(manifest_path).read_text())
    if data.get('schema') != 1 or data.get('status') != 'passed':
        raise ValueError('A successful Android upgrade receipt is required')
    if set(data.get('checks', {})) != CHECKS or not all(v is True for v in data['checks'].values()):
        raise ValueError('Missing, skipped or failed upgrade checks')
    previous, candidate = data['previous'], data['candidate']
    pair(previous, candidate)
    assets = [a for a in manifest['assets'] if a['name'] == 'iris-chat-' + tag + '-android-arm64.apk']
    if len(assets) != 1 or manifest['tag'] != tag or candidate['tag'] != tag:
        raise ValueError('Upgrade receipt does not match the release tag')
    if candidate['commit'] != manifest['commit'] or candidate['apk_sha256'] != assets[0]['sha256'] or candidate['manifest_sha256'] != sha256(manifest_path):
        raise ValueError('Upgrade receipt does not match the exact release source and assets')
    if previous.get('attested') is not True or candidate.get('attested') is not True:
        raise ValueError('Both unchanged release APKs must be attested')
    evidence = data['evidence']
    required = {'commands.jsonl', 'crash.log', 'exit-info.log', 'candidate-restarted-contact.png'}
    required.update(stage + '-state/state.json' for stage in ('previous', 'upgraded', 'edited', 'reopened'))
    required.update(label + '-' + kind + '-attestation.json' for label in ('previous', 'candidate') for kind in ('apk', 'manifest'))
    required.update(label + '-' + kind + '.txt' for label in ('previous', 'candidate') for kind in ('badging', 'signature'))
    if not required <= evidence.keys():
        raise ValueError('Incomplete retained runtime evidence')
    for name, digest in evidence.items():
        target = (path.parent / name).resolve()
        if path.parent not in target.parents or not target.is_file() or sha256(target) != digest:
            raise ValueError('Missing or changed runtime evidence: ' + name)
    if re.search(r'FATAL EXCEPTION|Fatal signal|backtrace:|ANR in', (path.parent / 'crash.log').read_text()) or re.search(r'reason=(?:4|5|6)\b', (path.parent / 'exit-info.log').read_text()):
        raise ValueError('Retained evidence contains an Android crash or ANR')
    provenance(data, path.parent)
    states = {}
    for stage in ('previous', 'upgraded', 'edited', 'reopened'):
        files = data['snapshots'][stage]
        if 'core.sqlite3' not in files:
            raise ValueError('Missing original synthetic database fixture')
        for name, digest in files.items():
            if name not in ('core.sqlite3', 'core.sqlite3-wal') or evidence.get(stage + '-state/' + name) != digest:
                raise ValueError('Snapshot does not match retained database evidence')
        states[stage] = database_state(path.parent / (stage + '-state'), files)
        summary = json.loads((path.parent / (stage + '-state/state.json')).read_text())
        if states[stage] != summary:
            raise ValueError('Saved-state summary differs from the original database: ' + stage)
    compare(states['previous'], states['upgraded'])
    compare(states['previous'], states['edited'], edited=True)
    compare(states['edited'], states['reopened'], edited=True)
    commands = [json.loads(line)['args'] for line in (path.parent / 'commands.jsonl').read_text().splitlines()]
    installs = [row for row in commands if row[0] == 'install']
    if len(installs) != 2 or installs[0][:-1] != ['install', '--no-streaming'] or installs[1][:-1] != ['install', '--no-streaming', '-r']:
        raise ValueError('Expected one fresh previous install and one data-preserving candidate update')
    if Path(installs[0][-1]).name != 'iris-chat-' + previous['tag'] + '-android-arm64.apk' or Path(installs[1][-1]).name != assets[0]['name']:
        raise ValueError('Installed binaries do not match the verified releases')
    if any('uninstall' in row or ('pm' in row and 'clear' in row) for row in commands):
        raise ValueError('App data was removed during the upgrade test')
    return dict(status='passed', previous=previous['tag'], candidate=tag, apk_sha256=candidate['apk_sha256'], checks=sorted(CHECKS))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipt', required=True)
    parser.add_argument('--manifest', required=True)
    parser.add_argument('--tag', required=True)
    args = parser.parse_args()
    print(json.dumps(validate(args.receipt, args.manifest, args.tag)))

if __name__ == '__main__':
    main()
