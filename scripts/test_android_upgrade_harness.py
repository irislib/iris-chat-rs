#!/usr/bin/env python3
"""Small negative tests for release continuity acceptance; no build or device."""
import copy
import base64
import json
import sqlite3
from contextlib import closing
import shutil
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
from unittest.mock import patch
from android_upgrade_ui import bounds, clickable, matching, Bridge
from android_upgrade_state import compare, sha256, inspect, PEER
from android_upgrade_release import pair
from test_android_release_upgrade import preflight, offline, launch
from check_android_upgrade_receipt import validate, CHECKS, reviewed_inputs, database_state


def state():
    return dict(owner='a', contact='b', devices=['d'], preferences={'typing': 'false'}, profiles={'a': 'self', 'b': 'peer'},
                pending={'queued': 'event'}, history=dict(threads={'t': 'draft'}, messages={'m': {'chat': 't', 'content': 'body'}}, groups={'g': 'name'}),
                seeded=dict(message=True, group_message=True, draft=True, setting=True, group=True, profile=True, contact=True), edited=False)


def database_fixture(path, edited=False):
    """Real SQLite with synthetic rows, only for validator unit tests."""
    path.unlink(missing_ok=True)
    with closing(sqlite3.connect(path)) as db, db:
        db.executescript("""
          CREATE TABLE app_meta(key TEXT, value TEXT);
          CREATE TABLE preferences(id INTEGER, send_typing_indicators INTEGER);
          CREATE TABLE owner_profiles(owner_pubkey_hex TEXT, name TEXT, display_name TEXT, picture TEXT, about TEXT, nickname TEXT, contact_note TEXT);
          CREATE TABLE ndr_kv(owner_pubkey_hex TEXT, device_pubkey_hex TEXT);
          CREATE TABLE pending_relay_publishes(event_id TEXT, owner_pubkey_hex TEXT, label TEXT, event_json TEXT, inner_event_id TEXT, chat_id TEXT, created_at_secs INTEGER);
          CREATE TABLE threads(chat_id TEXT, draft TEXT);
          CREATE TABLE messages(chat_id TEXT, id TEXT, kind TEXT, author TEXT, body TEXT, is_outgoing INTEGER, created_at_secs INTEGER, attachments_json TEXT);
          CREATE TABLE groups(group_id TEXT, name TEXT, picture TEXT, created_at_ms INTEGER, group_json TEXT);
          INSERT INTO app_meta VALUES('account_owner_pubkey_hex','fixture-owner');
          INSERT INTO preferences VALUES(1,0);
          INSERT INTO owner_profiles VALUES('fixture-owner','Upgrade fixture',NULL,NULL,NULL,NULL,NULL);
          INSERT INTO ndr_kv VALUES('fixture-owner','fixture-device');
          INSERT INTO threads VALUES('fixture-owner','Upgrade draft retained');
          INSERT INTO messages VALUES('fixture-owner','message', 'text','fixture-owner','Upgrade message one',1,1,'[]');
          INSERT INTO messages VALUES('group','group-message','text','fixture-owner','Upgrade group message',1,2,'[]');
          INSERT INTO groups VALUES('group','Upgrade continuity group',NULL,1,'{}');
        """)
        db.execute('INSERT INTO threads VALUES(?,?)', (PEER, ''))
        db.execute('INSERT INTO owner_profiles VALUES(?,NULL,NULL,NULL,NULL,?,?)',
                   (PEER, 'Upgrade saved nickname' if edited else None, 'Upgrade saved note' if edited else None))
        return inspect(db)


def receipt_fixture(folder, manifest_path):
    """Deliberately fake runtime evidence for acceptance/distributor unit tests."""
    folder = Path(folder); folder.mkdir(exist_ok=True)
    manifest = json.loads(Path(manifest_path).read_text()); tag = manifest['tag']
    apk = next(a for a in manifest['assets'] if a['name'].endswith('-android-arm64.apk'))
    runner_hashes, baseline_hash, pins = reviewed_inputs()
    previous = dict(tag='v2026.10.28.4', **pins['v2026.10.28.4'],
                    version_code=1, signer_sha256='d' * 64, attested=True)
    candidate = dict(tag=tag, commit=manifest['commit'], apk_sha256=apk['sha256'], manifest_sha256=sha256(manifest_path),
                     version_code=2, signer_sha256='d' * 64, attested=True)
    for stage in ('previous', 'upgraded', 'edited', 'reopened'):
        directory = folder / (stage + '-state'); directory.mkdir(exist_ok=True)
        saved = database_fixture(directory / 'core.sqlite3', stage in ('edited', 'reopened'))
        (directory / 'state.json').write_text(json.dumps(saved))
    for label, record in (('previous', previous), ('candidate', candidate)):
        (folder / (label + '-badging.txt')).write_text("package: name='to.iris.chat' versionCode='" + str(record['version_code']) + "' versionName='" + record['tag'][1:] + "'\n")
        (folder / (label + '-signature.txt')).write_text('Signer #1 certificate SHA-256 digest: ' + record['signer_sha256'] + '\n')
        for kind, suffix in (('apk', '-android-arm64.apk'), ('manifest', '-manifest.json')):
            statement = dict(predicateType='https://in-toto.io/attestation/release/v0.2',
                             predicate=dict(repository='irislib/iris-chat-rs', tag=record['tag']),
                             subject=[dict(name='iris-chat-' + record['tag'] + suffix, digest=dict(sha256=record[kind + '_sha256']))])
            result = dict(verificationResult=dict(statement=statement, signature=dict(certificate=dict(subjectAlternativeName='https://dotcom.releases.github.com'))),
                          attestation=dict(bundle=dict(dsseEnvelope=dict(payload=base64.b64encode(json.dumps(statement).encode()).decode()))))
            (folder / (label + '-' + kind + '-attestation.json')).write_text(json.dumps(result))
    (folder / 'crash.log').write_text('')
    (folder / 'exit-info.log').write_text('reason=10 (USER REQUESTED)')
    (folder / 'candidate-restarted-contact.png').write_bytes(b'unit test image')
    commands = [['install', '--no-streaming', 'iris-chat-v2026.10.28.4-android-arm64.apk'],
                ['install', '--no-streaming', '-r', apk['name']]]
    (folder / 'commands.jsonl').write_text(''.join(json.dumps(dict(args=row)) + '\n' for row in commands))
    data = dict(schema=1, status='passed', runner_sha256=runner_hashes, baseline_sha256=baseline_hash, checks={key: True for key in CHECKS}, previous=previous, candidate=candidate,
                evidence={str(p.relative_to(folder)): sha256(p) for p in folder.rglob('*') if p.is_file() and p.name != 'receipt.json'})
    data['snapshots'] = {stage: {'core.sqlite3': sha256(folder / (stage + '-state/core.sqlite3'))} for stage in ('previous', 'upgraded', 'edited', 'reopened')}
    path = folder / 'receipt.json'; path.write_text(json.dumps(data))
    return path


class Contents(unittest.TestCase):
    def test_preserves_saved_contents(self):
        compare(state(), state())

    def test_same_counts_cannot_hide_lost_contents(self):
        for table in ('threads', 'messages', 'groups'):
            with self.subTest(table=table):
                after = state(); key = next(iter(after['history'][table])); after['history'][table][key] = 'different'
                with self.assertRaises(ValueError): compare(state(), after)

    def test_identity_preferences_queue_and_profile_changes_fail(self):
        for key in ('owner', 'devices', 'preferences', 'pending', 'profiles'):
            with self.subTest(key=key):
                after = state(); after[key] = {} if isinstance(after[key], dict) else 'changed'
                with self.assertRaises(ValueError): compare(state(), after)

    def test_empty_fixture_fails(self):
        before = state(); before['seeded']['draft'] = False
        with self.assertRaises(ValueError): compare(before, before)

    def test_intended_nickname_edit_only(self):
        after = state(); after['profiles']['b'] = 'edited'; after['edited'] = True
        compare(state(), after, edited=True)
        after['profiles']['a'] = 'changed'
        with self.assertRaises(ValueError): compare(state(), after, edited=True)

    def test_missing_edit_fails(self):
        with self.assertRaises(ValueError): compare(state(), state(), edited=True)


class DeviceSafety(unittest.TestCase):
    def test_physical_device_rejected_without_command(self):
        with tempfile.TemporaryDirectory() as folder, patch('subprocess.run') as run:
            with self.assertRaises(ValueError): Bridge('adb', 'phone-123', folder)
            run.assert_not_called()

    def test_active_network_rejected(self):
        class Fake:
            def shell(self, *args): return '1: lo: <UP>\n2: eth0: <UP>\n'
        with self.assertRaises(ValueError): offline(Fake())

    def test_offline_loopback_allowed(self):
        class Fake:
            def shell(self, *args): return '1: lo: <UP>\n3: dummy0: <UP>\n'
        offline(Fake())

    def test_existing_app_refused(self):
        class Fake:
            def shell(self, *args):
                return {('getprop', 'ro.kernel.qemu'): '1', ('id', '-u'): '0', ('pm', 'list', 'packages', 'to.iris.chat'): 'package:to.iris.chat'}[args]
            def run(self, *args): return 'owned\nOK'
        with self.assertRaisesRegex(ValueError, 'Existing app'): preflight(Fake(), 'owned')

    def test_launch_accepts_restored_chat_without_assuming_list(self):
        class FakeBridge:
            def shell(self, *args):
                if args == ('ip', '-o', 'link', 'show', 'up'): return '1: lo: <UP>'
                return ''
        class FakeUI:
            def wait(self, predicate):
                root = ET.fromstring('<hierarchy><node package="to.iris.chat" content-desc="Add"/></hierarchy>')
                if not predicate(root): raise ValueError('Restored conversation was rejected')
        launch(FakeBridge(), FakeUI())

    def test_wrong_avd_refused(self):
        class Fake:
            def shell(self, *args): return '1'
            def run(self, *args): return 'someone-elses-avd\nOK'
        with self.assertRaises(ValueError): preflight(Fake(), 'owned')


class Provenance(unittest.TestCase):
    def test_upgrade_requires_increasing_version_and_same_signer(self):
        old = dict(tag='v1', version_code=1, signer_sha256='signer')
        new = dict(tag='v2', version_code=2, signer_sha256='signer')
        pair(old, new)
        for change in ({'version_code': 1}, {'signer_sha256': 'other'}, {'tag': 'v1'}):
            with self.assertRaises(ValueError): pair(old, dict(new, **change))

    def test_failed_or_skipped_receipt_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            receipt = Path(folder) / 'receipt.json'; manifest = Path(folder) / 'manifest.json'; manifest.write_text('{}')
            for data in ({'schema': 1, 'status': 'failed'}, {'schema': 1, 'status': 'passed', 'checks': {'cold_restart': True}}):
                receipt.write_text(json.dumps(data))
                with self.assertRaises(ValueError): validate(receipt, manifest, 'v2026.10.30')


class ReceiptEvidence(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        self.manifest = self.folder / 'manifest.json'
        self.manifest.write_text(json.dumps(dict(tag='v2026.10.30', commit='a' * 40,
            assets=[dict(name='iris-chat-v2026.10.30-android-arm64.apk', sha256='b' * 64)])))
        self.receipt = receipt_fixture(self.folder / 'runtime', self.manifest)

    def test_complete_receipt_accepted(self):
        self.assertEqual(validate(self.receipt, self.manifest, 'v2026.10.30')['status'], 'passed')

    def test_wrong_tag_rejected(self):
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.31')

    def test_wrong_binary_or_source_rejected(self):
        original = json.loads(self.receipt.read_text())
        for key in ('apk_sha256', 'commit', 'manifest_sha256'):
            modified = copy.deepcopy(original); modified['candidate'][key] = 'wrong'
            self.receipt.write_text(json.dumps(modified))
            with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_missing_or_modified_evidence_rejected(self):
        target = self.receipt.parent / 'candidate-restarted-contact.png'; target.write_bytes(b'changed')
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')
        target.unlink()
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_uninstall_or_downgrade_install_rejected_even_with_new_log_digest(self):
        log = self.receipt.parent / 'commands.jsonl'; original = log.read_text()
        for text in (original + json.dumps(dict(args=['uninstall', 'to.iris.chat'])) + '\n', original.replace('"-r"', '"-d"')):
            log.write_text(text); data = json.loads(self.receipt.read_text()); data['evidence']['commands.jsonl'] = sha256(log)
            self.receipt.write_text(json.dumps(data))
            with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_crash_and_anr_rejected_even_with_matching_evidence_hash(self):
        for name, text in (('crash.log', 'FATAL EXCEPTION: main'), ('exit-info.log', 'reason=6 (ANR)')):
            with self.subTest(name=name):
                self.receipt = receipt_fixture(self.folder / 'runtime', self.manifest)
                log = self.receipt.parent / name; log.write_text(text)
                data = json.loads(self.receipt.read_text()); data['evidence'][name] = sha256(log)
                self.receipt.write_text(json.dumps(data))
                with self.assertRaisesRegex(ValueError, 'crash or ANR'): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_retained_original_database_is_required(self):
        data = json.loads(self.receipt.read_text()); data['snapshots']['previous'] = {}
        self.receipt.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'original synthetic database'): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_content_loss_rejected_even_with_matching_state_hash(self):
        path = self.receipt.parent / 'upgraded-state/state.json'
        state_data = json.loads(path.read_text()); next(iter(state_data['history']['messages'].values()))['content'] = 'lost original body'
        path.write_text(json.dumps(state_data))
        data = json.loads(self.receipt.read_text()); data['evidence']['upgraded-state/state.json'] = sha256(path)
        self.receipt.write_text(json.dumps(data))
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def replace_database_digest(self, stage):
        data = json.loads(self.receipt.read_text())
        name = stage + '-state/core.sqlite3'
        digest = sha256(self.receipt.parent / name)
        data['evidence'][name] = digest; data['snapshots'][stage]['core.sqlite3'] = digest
        self.receipt.write_text(json.dumps(data))

    def test_snapshot_content_is_checked_independently_of_summary(self):
        with closing(sqlite3.connect(self.receipt.parent / 'upgraded-state/core.sqlite3')) as db, db:
            db.execute("UPDATE messages SET body='replacement body' WHERE id='message'")
        self.replace_database_digest('upgraded')
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_non_sqlite_bytes_rejected_even_with_matching_digest(self):
        (self.receipt.parent / 'previous-state/core.sqlite3').write_bytes(b'not a database')
        self.replace_database_digest('previous')
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_validation_does_not_modify_any_retained_evidence(self):
        before = {str(p): sha256(p) for p in self.receipt.parent.rglob('*') if p.is_file()}
        validate(self.receipt, self.manifest, 'v2026.10.30')
        self.assertEqual(before, {str(p): sha256(p) for p in self.receipt.parent.rglob('*') if p.is_file()})

    def test_changed_runner_baseline_or_previous_pin_rejected(self):
        original = json.loads(self.receipt.read_text())
        changes = [('runner_sha256', {}), ('baseline_sha256', 'wrong'), ('previous', dict(original['previous'], commit='wrong'))]
        for key, value in changes:
            data = copy.deepcopy(original); data[key] = value; self.receipt.write_text(json.dumps(data))
            with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_unbound_attestation_or_signer_rejected_even_with_matching_digest(self):
        for name, text in (('previous-apk-attestation.json', '{}'), ('candidate-signature.txt', 'Signer #1 certificate SHA-256 digest: changed')):
            self.receipt = receipt_fixture(self.folder / 'runtime', self.manifest)
            path = self.receipt.parent / name; path.write_text(text)
            data = json.loads(self.receipt.read_text()); data['evidence'][name] = sha256(path)
            self.receipt.write_text(json.dumps(data))
            with self.assertRaises((ValueError, KeyError)): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_attestation_must_bind_repository_tag_and_asset(self):
        for field in ('repository', 'tag', 'digest'):
            self.receipt = receipt_fixture(self.folder / 'runtime', self.manifest)
            name = 'previous-apk-attestation.json'; path = self.receipt.parent / name
            result = json.loads(path.read_text()); statement = result['verificationResult']['statement']
            if field == 'digest': statement['subject'][0]['digest']['sha256'] = 'wrong'
            else: statement['predicate'][field] = 'wrong'
            result['attestation']['bundle']['dsseEnvelope']['payload'] = base64.b64encode(json.dumps(statement).encode()).decode()
            path.write_text(json.dumps(result))
            data = json.loads(self.receipt.read_text()); data['evidence'][name] = sha256(path)
            self.receipt.write_text(json.dumps(data))
            with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')

    def test_reads_wal_without_modifying_originals(self):
        original = self.folder / 'original.db'; database_fixture(original)
        destination = self.folder / 'wal-state'; destination.mkdir()
        with closing(sqlite3.connect(original)) as db, db:
            db.execute('PRAGMA journal_mode=WAL'); db.execute('PRAGMA wal_autocheckpoint=0')
            db.execute("UPDATE preferences SET send_typing_indicators=1"); db.commit()
            # Copy while the connection is open, so the committed update exists
            # only in the WAL rather than in the main database.
            for suffix in ('', '-wal'):
                shutil.copyfile(str(original) + suffix, destination / ('core.sqlite3' + suffix))
        files = {p.name: sha256(p) for p in destination.iterdir()}
        self.assertFalse(database_state(destination, files)['seeded']['setting'])
        self.assertEqual(files, {p.name: sha256(p) for p in destination.iterdir()})
        with self.assertRaisesRegex(ValueError, 'Unrecorded SQLite WAL'):
            database_state(destination, {'core.sqlite3': files['core.sqlite3']})

    def test_evidence_path_escape_rejected(self):
        data = json.loads(self.receipt.read_text()); data['evidence']['../manifest.json'] = sha256(self.manifest)
        self.receipt.write_text(json.dumps(data))
        with self.assertRaises(ValueError): validate(self.receipt, self.manifest, 'v2026.10.30')


class HostedSelection(unittest.TestCase):
    def test_each_upgrade_or_smoke_input_triggers_release_contract(self):
        import fnmatch
        import re
        root = Path(__file__).resolve().parents[1]
        workflow = (root / '.github/workflows/release-contract.yml').read_text()
        changed_inputs = ['scripts/android_upgrade_release.py', 'scripts/android_upgrade_state.py',
                          'scripts/android_upgrade_ui.py', 'scripts/android_saved_history.py',
                          'scripts/check_android_upgrade_receipt.py', 'scripts/test_android_release_upgrade.py',
                          'scripts/test_android_upgrade_harness.py', 'test-fixtures/android-upgrade/releases.json',
                          'scripts/android_runtime_smoke.py', 'scripts/test_android_runtime_smoke.py',
                          'scripts/test_native_contract.sh']
        for event in ('push', 'pull_request'):
            body = re.search(r'^  ' + event + r':\n(.*?)(?=^  \S|\Z)', workflow, re.M | re.S).group(1)
            block = re.search(r'^    paths:\n((?:      - .+\n)+)', body, re.M).group(1)
            paths = [json.loads(line.removeprefix('      - ')) for line in block.splitlines()]
            for changed in changed_inputs:
                with self.subTest(event=event, changed=changed):
                    self.assertTrue(any(fnmatch.fnmatchcase(changed, pattern) for pattern in paths))
        self.assertIn('scripts/test_release_publish_guard', workflow)
        guard = (root / 'scripts/test_release_publish_guard').read_text()
        self.assertIn('unittest discover', guard)
        self.assertIn("-p 'test_*.py'", guard)


class Selectors(unittest.TestCase):
    def test_ambiguous_or_foreign_controls_rejected(self):
        root = ET.fromstring('<hierarchy><node package="to.iris.chat" text="Save" clickable="true" enabled="true" bounds="[0,0][20,20]"/><node package="to.iris.chat" text="Save" clickable="true" enabled="true" bounds="[30,0][50,20]"/><node package="other" text="Private"/></hierarchy>')
        self.assertEqual(matching(root, text='Private'), [])
        with self.assertRaises(ValueError): clickable(root, matching(root, text='Save'))

    def test_empty_bounds_rejected(self):
        with self.assertRaises(ValueError): bounds(ET.fromstring('<node bounds="[0,0][0,0]"/>'))


if __name__ == '__main__':
    unittest.main()
