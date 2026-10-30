"""Read stopped, synthetic release databases; compare contents, never row counts."""
import hashlib
import json
from pathlib import Path
import sqlite3
from android_saved_history import fingerprint, history_snapshot
from android_upgrade_ui import PACKAGE

# Public test key only; generated accounts never connect to a network.
PEER = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def snapshot(bridge, name):
    bridge.shell('am', 'force-stop', PACKAGE)
    if bridge.shell('pidof', PACKAGE, check=False).strip():
        raise RuntimeError('App must be stopped before copying its database')
    folder = bridge.output / name
    folder.mkdir(mode=0o700)
    remote = '/data/user/0/' + PACKAGE + '/no_backup/profiles/active/core.sqlite3'
    for suffix in ('', '-wal'):
        if suffix and not bridge.shell('ls', remote + suffix, check=False).strip():
            continue
        data = bridge.run('exec-out', 'cat', remote + suffix, binary=True)
        if len(data) > 16 * 1024**2:
            raise RuntimeError('Synthetic fixture exceeds its bounded size')
        target = folder / ('core.sqlite3' + suffix)
        target.write_bytes(data)
        target.chmod(0o600)
    hashes = {p.name: sha256(p) for p in folder.iterdir()}
    # Work on a copy: SQLite can checkpoint/delete a copied WAL while opening.
    import shutil
    inspected = folder / 'inspected'
    inspected.mkdir(mode=0o700)
    for p in folder.glob('core.sqlite3*'):
        shutil.copyfile(p, inspected / p.name)
    with sqlite3.connect(inspected / 'core.sqlite3') as db:
        if db.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
            raise ValueError('Saved database integrity check failed')
        state = inspect(db)
    (folder / 'state.json').write_text(json.dumps(state, indent=2) + '\n')
    return dict(state=state, files=hashes)


def inspect(db):
    owner = db.execute("SELECT value FROM app_meta WHERE key='account_owner_pubkey_hex'").fetchone()
    if not owner or not owner[0]:
        raise ValueError('Missing saved account identity')
    prefs = db.execute('SELECT * FROM preferences').fetchall()
    columns = [row[1] for row in db.execute('PRAGMA table_info(preferences)')]
    if len(prefs) != 1:
        raise ValueError('Missing saved preferences')
    preferences = {key: fingerprint(value) for key, value in zip(columns, prefs[0])}
    profiles = {fingerprint(row[0]): fingerprint(row) for row in db.execute(
        'SELECT owner_pubkey_hex,name,display_name,picture,about,nickname,contact_note FROM owner_profiles')}
    devices = sorted(fingerprint(row) for row in db.execute(
        'SELECT DISTINCT owner_pubkey_hex,device_pubkey_hex FROM ndr_kv'))
    if not devices:
        raise ValueError('Missing saved device identity')
    pending = {fingerprint(row[0]): fingerprint(row) for row in db.execute(
        'SELECT event_id,owner_pubkey_hex,label,event_json,inner_event_id,chat_id,created_at_secs FROM pending_relay_publishes')}
    history = history_snapshot(db)
    bodies = [r[0] for r in db.execute('SELECT body FROM messages')]
    drafts = [r[0] for r in db.execute('SELECT draft FROM threads')]
    seeded = {'message': 'Upgrade message one' in bodies,
              'group_message': 'Upgrade group message' in bodies,
              'draft': 'Upgrade draft retained' in drafts,
              'contact': bool(db.execute('SELECT 1 FROM threads WHERE chat_id=?', (PEER,)).fetchone()),
              'setting': dict(zip(columns, prefs[0]))['send_typing_indicators'] == 0,
              'group': bool(db.execute("SELECT 1 FROM groups WHERE name='Upgrade continuity group'").fetchone()),
              'profile': bool(db.execute("SELECT 1 FROM owner_profiles WHERE owner_pubkey_hex=? AND (name=? OR display_name=?)", (owner[0], 'Upgrade fixture', 'Upgrade fixture')).fetchone())}
    edited = bool(db.execute('SELECT 1 FROM owner_profiles WHERE owner_pubkey_hex=? AND nickname=? AND contact_note=?',
                            (PEER, 'Upgrade saved nickname', 'Upgrade saved note')).fetchone())
    return dict(owner=fingerprint(owner[0]), contact=fingerprint(PEER), devices=devices, preferences=preferences,
                profiles=profiles, pending=pending, history=history, seeded=seeded, edited=edited)


def compare(before, after, *, edited=False):
    for key in ('owner', 'contact', 'devices'):
        if before[key] != after[key]:
            raise ValueError('Account continuity failed: ' + key)
    for key in ('preferences', 'pending'):
        if any(after[key].get(k) != v for k, v in before[key].items()):
            raise ValueError('Saved contents changed: ' + key)
    if before['history'] != after['history']:
        raise ValueError('Saved messages, drafts or groups changed')
    for key, value in before['profiles'].items():
        if not (edited and key == before['contact']) and after['profiles'].get(key) != value:
            raise ValueError('Saved profile changed')
    if not all(before['seeded'].values()) or not all(after['seeded'].values()):
        raise ValueError('Synthetic fixture did not retain every required content type')
    if edited and not after['edited']:
        raise ValueError('Edited nickname and note were not saved')
