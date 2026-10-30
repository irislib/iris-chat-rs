#!/usr/bin/env python3
"""Run real previous-version continuity with unchanged, attested release APKs.

Requires an explicitly owned, rooted, disposable, English Android emulator with
no existing Iris installation and no active network. Does not build, clear data,
uninstall, restore snapshots, or modify APKs. Private synthetic evidence remains
in --output even on failure. Install -r is the sole candidate installation.
"""
import argparse
import json
import os
from pathlib import Path
import re
import time
from android_upgrade_ui import Bridge, UI, PACKAGE, matching, clickable
from android_upgrade_state import snapshot, compare, sha256, PEER
from android_upgrade_release import release, pair


def preflight(b, avd):
    if b.shell('getprop', 'ro.kernel.qemu').strip() != '1':
        raise ValueError('Only an emulator may be used')
    if b.run('emu', 'avd', 'name').splitlines()[0] != avd:
        raise ValueError('Selected emulator does not match explicit AVD ownership')
    if b.shell('id', '-u').strip() != '0':
        raise ValueError('Start adb root on the owned disposable emulator first')
    if b.shell('pm', 'list', 'packages', PACKAGE).strip():
        raise ValueError('Existing app installation refused; preserve its data')
    offline(b)


def offline(b):
    links = b.shell('ip', '-o', 'link', 'show', 'up')
    names = re.findall(r'^\d+: ([^:@]+)(?:@[^:]+)?:', links, re.M)
    if not names or any(n not in ('lo', 'dummy0') for n in names):
        raise ValueError('Disable every emulator network interface before synthetic account creation')


def launch(b, u):
    offline(b)
    b.shell('am', 'start', '-W', '-n', PACKAGE + '/.MainActivity')
    # A cold start may restore the previously open conversation.
    u.wait(lambda r: bool(matching(r, description='New chat')) or
           bool(matching(r, description='Add')) or bool(matching(r, text='Create profile')))


def seed(u):
    u.tap(text='Create profile')
    u.wait(lambda r: len(matching(r, kind='android.widget.EditText')) == 1)
    u.input('Upgrade fixture'); u.back(); u.tap(text='Create profile')
    u.wait(lambda r: bool(matching(r, description='New chat')))
    u.note_to_self()
    u.input('Upgrade message one'); u.tap(description='Send')
    u.wait(lambda r: bool(matching(r, text='Upgrade message one')))
    u.input('Upgrade draft retained'); u.back(); u.capture('previous-message-draft')
    u.chats(); u.open_settings(); u.tap(text='Messaging')
    root = u.wait(lambda r: bool(matching(r, text='Typing indicators')))
    if clickable(root, matching(root, text='Typing indicators')).get('checked') != 'true':
        raise ValueError('Unexpected initial typing preference')
    u.tap(text='Typing indicators')
    u.wait(lambda r: clickable(r, matching(r, text='Typing indicators')).get('checked') == 'false')
    u.chats(); u.tap(description='New chat'); u.tap(text='Create group'); u.tap(text='Next')
    u.input('Upgrade continuity group'); u.back(); u.tap(text='Create group')
    u.wait(lambda r: len(matching(r, kind='android.widget.EditText')) == 1)
    u.input('Upgrade group message'); u.tap(description='Send')
    u.wait(lambda r: bool(matching(r, text='Upgrade group message')))
    u.capture('previous-group'); open_peer(u); u.chats()


def chat_contents(u):
    u.chats(); u.note_to_self()
    u.wait(lambda r: bool(matching(r, text='Upgrade message one')) and
           any(n.get('text') == 'Upgrade draft retained' for n in matching(r, kind='android.widget.EditText')))
    u.capture('candidate-message-draft')


def open_peer(u):
    u.chats(); u.tap(description='New chat')
    root = u.wait(lambda r: len(matching(r, kind='android.widget.EditText')) == 1)
    u.tap_node(matching(root, kind='android.widget.EditText')[0])
    u.bridge.shell('input', 'text', PEER)
    # Entering a valid ID automatically navigates away from the input screen.
    u.wait(lambda r: bool(matching(r, text='Wild Falcon')) or bool(matching(r, text='Upgrade saved nickname')))


def peer_details(u):
    open_peer(u)
    root = u.read()
    label = 'Upgrade saved nickname' if matching(root, text='Upgrade saved nickname') else 'Wild Falcon'
    u.tap(text=label)
    u.wait(lambda r: bool(matching(r, text='Nickname and note')))


def edit_contact(u):
    peer_details(u)
    u.tap(text='Nickname and note')
    u.wait(lambda r: len(matching(r, kind='android.widget.EditText')) == 2)
    u.input('Upgrade saved nickname', index=0, expected_count=2)
    u.input('Upgrade saved note', index=1, expected_count=2)
    u.back(); u.tap(text='Save')
    u.wait(lambda r: bool(matching(r, text='Upgrade saved nickname')))
    u.capture('candidate-contact')


def run(args):
    os.umask(0o077)
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    b = Bridge(args.adb, args.serial, output)
    receipt = dict(schema=1, status='failed', checks={}, started_at=int(time.time()))
    receipt['runner_sha256'] = {p.name: sha256(p) for p in Path(__file__).parent.glob('android_upgrade_*.py')}
    receipt['runner_sha256'][Path(__file__).name] = sha256(__file__)
    receipt['runner_sha256']['android_saved_history.py'] = sha256(Path(__file__).with_name('android_saved_history.py'))
    try:
        preflight(b, args.avd_name)
        receipt['android'] = dict(api=b.shell('getprop', 'ro.build.version.sdk').strip(),
                                  fingerprint=b.shell('getprop', 'ro.build.fingerprint').strip())
        tools = Path(args.build_tools).resolve()
        receipt['previous'] = release(args.previous_apk, args.previous_manifest, tools, output, 'previous')
        receipt['candidate'] = release(args.candidate_apk, args.candidate_manifest, tools, output, 'candidate')
        pair(receipt['previous'], receipt['candidate'])
        baseline = Path(__file__).resolve().parents[1] / 'test-fixtures/android-upgrade/releases.json'
        pins = json.loads(baseline.read_text())['releases'].get(receipt['previous']['tag'])
        if not pins or any(receipt['previous'].get(k) != v for k, v in pins.items()):
            raise ValueError('Previous release must match its retained immutable fixture pin')
        receipt['baseline_sha256'] = sha256(baseline)
        b.run('logcat', '-b', 'crash', '-c')
        b.run('install', '--no-streaming', args.previous_apk, timeout=120)
        u = UI(b); launch(b, u); seed(u)
        before = snapshot(b, 'previous-state')
        if not all(before['state']['seeded'].values()):
            raise ValueError('Previous APK did not create the complete synthetic fixture')
        receipt['checks']['previous_fixture'] = True
        b.run('install', '--no-streaming', '-r', args.candidate_apk, timeout=120)
        launch(b, u); chat_contents(u); u.chats()
        upgraded = snapshot(b, 'upgraded-state')
        compare(before['state'], upgraded['state'])
        receipt['checks']['in_place_contents'] = True
        launch(b, u); chat_contents(u); edit_contact(u)
        b.shell('input', 'keyevent', 'HOME')
        b.shell('am', 'start', '-W', '-n', PACKAGE + '/.MainActivity')
        u.wait(lambda r: bool(matching(r, text='Upgrade saved nickname')))
        edited = snapshot(b, 'edited-state')
        compare(before['state'], edited['state'], edited=True)
        receipt['checks']['contact_edit_background'] = True
        launch(b, u); chat_contents(u); peer_details(u)
        u.wait(lambda r: bool(matching(r, text='Upgrade saved nickname')))
        u.capture('candidate-restarted-contact')
        reopened = snapshot(b, 'reopened-state')
        compare(edited['state'], reopened['state'], edited=True)
        receipt['checks']['cold_restart'] = True
        offline(b)
        crashes = b.run('logcat', '-b', 'crash', '-d')
        (output / 'crash.log').write_text(crashes)
        exits = b.shell('dumpsys', 'activity', 'exit-info', PACKAGE)
        (output / 'exit-info.log').write_text(exits)
        if re.search(r'reason=(?:4|5|6)\b', exits):
            raise ValueError('Android recorded a crash or unresponsive app')
        if re.search(r'FATAL EXCEPTION|Fatal signal|backtrace:|ANR in', crashes):
            raise ValueError('Android reported a runtime crash')
        receipt['checks']['no_crashes'] = True
        receipt['snapshots'] = {k: v['files'] for k, v in [('previous', before), ('upgraded', upgraded), ('edited', edited), ('reopened', reopened)]}
        receipt['status'] = 'passed'
    except Exception as exc:
        receipt['error'] = str(exc)
        raise
    finally:
        receipt['finished_at'] = int(time.time())
        receipt['evidence'] = {str(p.relative_to(output)): sha256(p) for p in output.rglob('*') if p.is_file()}
        (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print(json.dumps(dict(status=receipt['status'], checks=receipt['checks'], output=str(output))))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ('adb', 'serial', 'avd-name', 'build-tools', 'previous-apk', 'previous-manifest', 'candidate-apk', 'candidate-manifest', 'output'):
        parser.add_argument('--' + field, required=True)
    run(parser.parse_args())

if __name__ == '__main__':
    main()
