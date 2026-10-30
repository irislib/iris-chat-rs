#!/usr/bin/env python3
"""Read-only release acceptance: bind real upgrade evidence to the published APK."""
import argparse
import json
import re
from pathlib import Path
from android_upgrade_state import compare, sha256
from android_upgrade_release import pair

CHECKS = {'previous_fixture', 'in_place_contents', 'contact_edit_background', 'cold_restart', 'no_crashes'}


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
    if not required <= evidence.keys():
        raise ValueError('Incomplete retained runtime evidence')
    for name, digest in evidence.items():
        target = (path.parent / name).resolve()
        if path.parent not in target.parents or not target.is_file() or sha256(target) != digest:
            raise ValueError('Missing or changed runtime evidence: ' + name)
    if re.search(r'FATAL EXCEPTION|Fatal signal|backtrace:|ANR in', (path.parent / 'crash.log').read_text()) or re.search(r'reason=(?:4|5|6)\b', (path.parent / 'exit-info.log').read_text()):
        raise ValueError('Retained evidence contains an Android crash or ANR')
    for stage in ('previous', 'upgraded', 'edited', 'reopened'):
        files = data['snapshots'][stage]
        if 'core.sqlite3' not in files:
            raise ValueError('Missing original synthetic database fixture')
        for name, digest in files.items():
            if name not in ('core.sqlite3', 'core.sqlite3-wal') or evidence.get(stage + '-state/' + name) != digest:
                raise ValueError('Snapshot does not match retained database evidence')
    states = {stage: json.loads((path.parent / (stage + '-state/state.json')).read_text())
              for stage in ('previous', 'upgraded', 'edited', 'reopened')}
    # This publication boundary must remain fail-closed with Python -O too;
    # the shared history helper also uses assertions for ordinary test runners.
    if any(saved['history'] != states['previous']['history'] for saved in states.values()):
        raise ValueError('Saved messages, drafts or groups changed during the upgrade')
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
