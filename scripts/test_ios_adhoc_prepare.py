#!/usr/bin/env python3
"""Offline tests: no signing, Apple API, device or installation commands."""
import base64
import copy
import datetime as dt
import hashlib
import io
import json
from pathlib import Path
import plistlib
import struct
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile

import ios_adhoc_prepare as subject


def executable(signature_size=64, *, instruction=b'unchanged instructions', extra_command=b''):
    text_size, signature_at = 4096, 4112
    total = signature_at + signature_size
    segment = lambda name, start, size: struct.pack('<II16sQQQQiiII', 0x19, 72, name,
        0x100000000 + start, (size + 16383) // 16384 * 16384, start, size, 7, 5, 0, 0)
    commands = (segment(b'__TEXT', 0, text_size)
        + segment(b'__LINKEDIT', text_size, total - text_size)
        + struct.pack('<II16s', 0x1b, 24, bytes(range(16)))
        + extra_command + struct.pack('<IIII', 0x1d, 16, signature_at, signature_size))
    header = struct.pack('<8I', 0xfeedfacf, 0x100000c, 0, 2, 4 + bool(extra_command), len(commands), 0, 0)
    data = bytearray(total)
    data[:len(header + commands)] = header + commands
    data[1024:1024 + len(instruction)] = instruction
    data[signature_at:signature_at + 12] = struct.pack('>III', 0xfade0cc0, 12, 0)
    return bytes(data)


class MachOTests(unittest.TestCase):
    def test_only_signature_extent_changes_are_normalized_and_raw_commands_retained(self):
        original, resigned = map(subject.macho_binding, (executable(64), executable(20000)))
        self.assertEqual(original['code'], resigned['code'])
        self.assertNotEqual(original['rawLoadCommandsSHA256'], resigned['rawLoadCommandsSHA256'])
        self.assertEqual(subject.sha(base64.b64decode(original['rawLoadCommandsBase64'])),
                         original['rawLoadCommandsSHA256'])
        self.assertEqual(subject.compare_code(original, resigned)['original'], original['code'])

    def test_instruction_mutation_fails(self):
        a, b = subject.macho_binding(executable()), subject.macho_binding(executable(instruction=b'changed instructions'))
        with self.assertRaisesRegex(subject.PreparationError, 'Instructions'):
            subject.compare_code(a, b)

    def test_non_signature_command_mutation_fails(self):
        original = executable()
        changed = bytearray(original)
        struct.pack_into('<Q', changed, 32 + 24, 0x100004000)  # __TEXT VM address must remain exact.
        with self.assertRaises(subject.PreparationError):
            subject.compare_code(subject.macho_binding(original), subject.macho_binding(changed))

    def test_uuid_mutation_fails(self):
        changed = bytearray(executable())
        changed[32 + 144 + 8] ^= 1
        with self.assertRaises(subject.PreparationError):
            subject.compare_code(subject.macho_binding(executable()), subject.macho_binding(changed))

    def test_unknown_load_command_fails(self):
        with self.assertRaisesRegex(subject.PreparationError, 'Unsupported load command'):
            subject.macho_binding(executable(extra_command=struct.pack('<II', 0x777, 8)))

    def test_data_reference_into_signature_fails(self):
        with self.assertRaisesRegex(subject.PreparationError, 'reference overlaps'):
            subject.macho_binding(executable(extra_command=struct.pack('<4I', 0x26, 16, 4112, 4)))

    def test_signature_cannot_hide_trailing_content(self):
        data = bytearray(executable())
        data[-1] = 1
        with self.assertRaisesRegex(subject.PreparationError, 'Nonzero'):
            subject.macho_binding(data)

    def test_signature_cannot_be_nonterminal(self):
        with self.assertRaisesRegex(subject.PreparationError, 'terminate'):
            subject.macho_binding(executable() + b'additional content')

    def test_malformed_or_unsupported_inputs(self):
        for mutate in [lambda b: b[:10], lambda b: b.replace(b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe', 1),
                       lambda b: b[:20] + struct.pack('<I', 0xffffffff) + b[24:]]:
            with self.subTest(mutate=mutate), self.assertRaises(subject.PreparationError):
                subject.macho_binding(mutate(executable()))

    def test_encrypted_image_fails(self):
        command = struct.pack('<6I', 0x2c, 24, 512, 1024, 1, 0)
        with self.assertRaisesRegex(subject.PreparationError, 'Encrypted'):
            subject.macho_binding(executable(extra_command=command))


class Fixture:
    def __init__(self, root):
        self.root = Path(root)
        self.candidate = {'tag': 'v2026.1.1', 'source_commit': 'a' * 40,
                          'apple_version': '2026.1.100', 'apple_build': '2026010100',
                          'ipa_sha256': '0' * 64, 'manifest_sha256': '0' * 64}
        self.cert = b'synthetic public certificate'
        self.now = time.time()
        self.device, self.coredevice = 'synthetic-device', 'synthetic-coredevice'
        self.team = 'EXAMPLETEAM'
        self.entitlements = {'get-task-allow': False, 'aps-environment': 'production',
                             'com.apple.developer.team-identifier': self.team,
                             'keychain-access-groups': [self.team + '.test']}
        self.profile = {'UUID': 'synthetic-profile', 'CreationDate': dt.datetime.fromtimestamp(self.now - 60, dt.timezone.utc).replace(tzinfo=None),
                        'ExpirationDate': dt.datetime.fromtimestamp(self.now + 3600, dt.timezone.utc).replace(tzinfo=None),
                        'ProvisionedDevices': [self.device], 'DeveloperCertificates': [self.cert],
                        'TeamIdentifier': [self.team], 'ApplicationIdentifierPrefix': [self.team]}

    def save(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value if isinstance(value, bytes) else json.dumps(value).encode())
        return subject.binding(path.resolve())

    def profile_for(self, bundle_id):
        profile = copy.deepcopy(self.profile)
        profile['Entitlements'] = {**self.entitlements, 'application-identifier': self.team + '.' + bundle_id,
                                   'keychain-access-groups': [self.team + '.*']}
        entitlements = {**self.entitlements, 'application-identifier': self.team + '.' + bundle_id}
        raw = plistlib.dumps(profile)
        api = {'data': {'attributes': {'profileState': 'ACTIVE', 'profileType': 'IOS_APP_ADHOC',
                                      'uuid': profile['UUID'], 'profileContent': base64.b64encode(raw).decode()}}}
        return profile, api, raw, entitlements

    def config(self):
        ipa = self.root / 'official.ipa'
        with zipfile.ZipFile(ipa, 'w') as archive:
            for role, relative, point in [('app', 'Payload/Example.app', None),
                    ('notification-service', 'Payload/Example.app/PlugIns/Notification.appex', 'com.apple.usernotifications.service'),
                    ('share-extension', 'Payload/Example.app/PlugIns/Share.appex', 'com.apple.share-services')]:
                info = {'CFBundleExecutable': 'Example', 'CFBundleIdentifier': 'example.' + role,
                        'CFBundleVersion': self.candidate['apple_build'],
                        'CFBundleShortVersionString': self.candidate['apple_version']}
                if point:
                    info['NSExtension'] = {'NSExtensionPointIdentifier': point}
                archive.writestr(relative + '/Info.plist', plistlib.dumps(info))
                archive.writestr(relative + '/Example', executable())
                archive.writestr(relative + '/resource.txt', b'unchanged asset')
                archive.writestr(relative + '/embedded.mobileprovision', b'original profile')
                archive.writestr(relative + '/_CodeSignature/CodeResources', b'original seal')
        ipa_bound = subject.binding(ipa)
        self.candidate['ipa_sha256'] = ipa_bound['sha256']
        manifest = self.save('manifest.json', {'tag': self.candidate['tag'], 'commit': self.candidate['source_commit'],
            'assets': [{'kind': 'ipa', 'platform': 'ios', 'sha256': ipa_bound['sha256'], 'size': ipa.stat().st_size}]})
        self.candidate['manifest_sha256'] = manifest['sha256']
        accepted = self.save('accepted.json', {'passed': True, 'release_immutable': True, 'tag': self.candidate['tag'],
            'commit': self.candidate['source_commit'], 'ipa': {'sha256': ipa_bound['sha256'],
             'exact_run_bound': True, 'release_attestation_verified': True}, 'canonical_manifest': manifest})
        target = {'device': self.device, 'coredevice_id': self.coredevice, 'observed_utc': self.now,
            'reservation': self.save('reservation.json', {'device': self.device, 'coredevice_id': self.coredevice,
                'started_utc': self.now - 1, 'expires_utc': self.now + 600}),
            'inventory': self.save('inventory.json', {'result': {'devices': [{'identifier': self.coredevice,
                'hardwareProperties': {'udid': self.device}, 'connectionProperties': {'tunnelState': 'connected'}}]}})}
        profiles = {}
        for role in subject.ROLES:
            _, api, raw, _ = self.profile_for('example.' + role)
            profiles[role] = {'profile': self.save(role + '.mobileprovision', raw),
                              'api': self.save(role + '-api.json', api), 'api_observed_utc': self.now}
        return {'schema': 'iris-adhoc-preparation-input-v1', 'candidate': self.candidate,
                'ipa': ipa_bound, 'manifest': manifest, 'release_acceptance': accepted,
                'authorization': self.save('authorization.json', {'route': subject.contract.ADHOC_CHANNEL,
                                                                'direct_install_authorized': True}),
                'target': target, 'certificate': self.save('certificate.der', self.cert), 'profiles': profiles,
                'tools': {'security': self.save('security', b'synthetic-security'),
                          'codesign': self.save('codesign', b'synthetic-codesign')}, 'minimum_free_bytes': 1024}


class ProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.fixture = Fixture(self.temp.name)
        self.profile, self.api, self.raw, self.entitlements = self.fixture.profile_for('example.app')

    def validate(self):
        return subject.validate_profile(self.profile, self.api, self.raw, device=self.fixture.device,
            bundle_id='example.app', entitlements=self.entitlements, certificate=self.fixture.cert, now=self.fixture.now)

    def test_exact_active_production_profile(self):
        self.assertEqual(self.validate()['bundleID'], 'example.app')

    def test_another_device_or_multiple_devices_fail(self):
        for devices in [['other-device'], [self.fixture.device, 'other-device'], []]:
            with self.subTest(devices=devices):
                self.profile['ProvisionedDevices'] = devices
                with self.assertRaises(subject.PreparationError):
                    self.validate()

    def test_expired_inactive_or_wrong_type_fails(self):
        for key, value in [('profileState', 'INVALID'), ('profileType', 'IOS_APP_DEVELOPMENT'), ('uuid', 'other')]:
            with self.subTest(key=key):
                old = self.api['data']['attributes'][key]
                self.api['data']['attributes'][key] = value
                with self.assertRaises(subject.PreparationError):
                    self.validate()
                self.api['data']['attributes'][key] = old
        self.profile['ExpirationDate'] = dt.datetime.fromtimestamp(self.fixture.now - 1, dt.timezone.utc)
        with self.assertRaises(subject.PreparationError):
            self.validate()

    def test_wrong_certificate_or_extra_entitlement_fails(self):
        self.profile['DeveloperCertificates'] = [b'other certificate']
        with self.assertRaises(subject.PreparationError):
            self.validate()
        self.profile['DeveloperCertificates'] = [self.fixture.cert]
        self.entitlements['com.apple.private.unexpected'] = True
        with self.assertRaises(subject.PreparationError):
            self.validate()

    def test_only_beta_removal_and_false_debugging(self):
        original = {**self.entitlements, 'beta-reports-active': True}
        self.assertEqual(subject.production_entitlements(original), self.entitlements)
        for key, value in [('get-task-allow', True), ('get-task-allow', 0), ('aps-environment', 'development')]:
            with self.subTest(key=key), self.assertRaises(subject.PreparationError):
                subject.production_entitlements({**original, key: value})

    def test_permissions_are_typed_and_only_trailing_wildcard_matches(self):
        self.assertTrue(subject.permitted(['TEAM.first'], ['TEAM.*']))
        self.assertFalse(subject.permitted(True, 1))
        self.assertFalse(subject.permitted('TEAM.first', 'TEAM.*first'))
        self.assertFalse(subject.permitted(['TEAM.first', 'OTHER.second'], ['TEAM.*']))

    def test_raw_api_array_is_supported_and_duplicate_uuid_rejected(self):
        self.api['data'] = [self.api['data']]
        self.assertEqual(self.validate()['bundleID'], 'example.app')
        self.api['data'].append(self.api['data'][0])
        with self.assertRaisesRegex(subject.PreparationError, 'ambiguous'):
            self.validate()


class ArchiveAndInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = Fixture(self.root)

    def test_traversal_duplicate_symlink_and_outside_payload_rejected(self):
        cases = [('Payload/../escape', 0), ('elsewhere/file', 0), ('/Payload/abs', 0), ('Payload/link', 0o120777)]
        for index, (name, mode) in enumerate(cases):
            with self.subTest(name=name):
                ipa = self.root / f'bad-{index}.zip'
                with zipfile.ZipFile(ipa, 'w') as archive:
                    info = zipfile.ZipInfo(name)
                    info.external_attr = mode << 16
                    archive.writestr(info, b'bad')
                with self.assertRaises(subject.PreparationError):
                    subject.archive_inventory(ipa)

    def test_acceptance_and_device_are_bound(self):
        config = self.fixture.config()
        subject.validate_inputs(config, self.fixture.now)
        for key in ['ipa_sha256', 'manifest_sha256', 'source_commit']:
            broken = copy.deepcopy(config)
            broken['candidate'][key] = 'b' * len(broken['candidate'][key])
            with self.subTest(key=key), self.assertRaises(subject.PreparationError):
                subject.validate_inputs(broken, self.fixture.now)
        with self.assertRaisesRegex(subject.PreparationError, 'Stale target'):
            subject.validate_inputs(config, self.fixture.now + 121)
        broken = copy.deepcopy(config)
        broken['target']['device'] = 'other-device'
        with self.assertRaises(subject.PreparationError):
            subject.validate_inputs(broken, self.fixture.now)

    def test_changed_file_is_rejected(self):
        value = self.fixture.save('file.json', {'a': 1})
        Path(value['path']).write_text('{"a":2}')
        with self.assertRaisesRegex(subject.PreparationError, 'changed'):
            subject.read_json(value)

    def test_case_colliding_archive_members_rejected(self):
        ipa = self.root / 'case.zip'
        with zipfile.ZipFile(ipa, 'w') as archive:
            archive.writestr('Payload/Example.app/A', b'one')
            archive.writestr('Payload/Example.app/a', b'two')
        with self.assertRaisesRegex(subject.PreparationError, 'Duplicate'):
            subject.archive_inventory(ipa)

    def test_official_symbol_and_signature_sidecars_remain_exact(self):
        ipa = self.root / 'sidecars.zip'
        with zipfile.ZipFile(ipa, 'w') as archive:
            archive.writestr('Payload/Example.app/resource', b'app')
            archive.writestr('Symbols/example.symbols', b'unchanged symbols')
            archive.writestr('Signatures/example.signature', b'unchanged sidecar')
        output = self.root / 'extracted'
        subject.extract(ipa, output)
        self.assertEqual((output / 'Symbols/example.symbols').read_bytes(), b'unchanged symbols')
        self.assertEqual((output / 'Signatures/example.signature').read_bytes(), b'unchanged sidecar')

    def test_expired_reservation_and_stale_api_rejected(self):
        config = self.fixture.config()
        reservation = json.loads(Path(config['target']['reservation']['path']).read_text())
        reservation['expires_utc'] = self.fixture.now - 1
        config['target']['reservation'] = self.fixture.save('reservation.json', reservation)
        with self.assertRaisesRegex(subject.PreparationError, 'reservation'):
            subject.validate_inputs(config, self.fixture.now)
        reservation['expires_utc'] = self.fixture.now + 600
        config['target']['reservation'] = self.fixture.save('reservation.json', reservation)
        config['profiles']['app']['api_observed_utc'] = self.fixture.now - 301
        with self.assertRaisesRegex(subject.PreparationError, 'Stale profile'):
            subject.validate_inputs(config, self.fixture.now)

    def test_no_execute_does_not_create_output_or_run_tools(self):
        config = self.fixture.save('config.json', self.fixture.config())
        with patch.object(subject.subprocess, 'run') as run, self.assertRaises(subject.PreparationError):
            subject.prepare(config, self.root / 'output')
        run.assert_not_called()
        self.assertFalse((self.root / 'output').exists())

    def test_extra_nested_code_rejected(self):
        config = self.fixture.config()
        root = self.root / 'extracted'
        subject.extract(Path(config['ipa']['path']), root)
        framework = root / 'Payload/Example.app/Frameworks/Extra.framework'
        framework.mkdir(parents=True)
        with self.assertRaisesRegex(subject.PreparationError, 'nested code'):
            subject.bundles(root, config['candidate'])


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = Fixture(self.root)
        self.config = self.fixture.config()
        self.config_binding = self.fixture.save('config.json', self.config)
        self.calls = []
        self.signed = set()
        self.mutate_instruction = False
        self.mutate_resource = False

    def fake_run(self, commands, tool, *args):
        self.calls.append((tool, args))
        if tool == 'security' and args[0] == 'find-identity':
            return hashlib.sha1(self.fixture.cert).hexdigest().upper().encode(), b''
        if tool == 'security' and args[0] == 'cms':
            return Path(args[-1]).read_bytes(), b''
        if tool == 'codesign' and args[:2] == ('-d', '--entitlements'):
            role = plistlib.loads((Path(args[-1]) / 'Info.plist').read_bytes())['CFBundleIdentifier'].split('.', 1)[1]
            ent = self.fixture.profile_for('example.' + role)[3]
            if role not in self.signed:
                ent['beta-reports-active'] = True
            return plistlib.dumps(ent), b''
        if tool == 'codesign' and args[0] == '--force':
            path = Path(args[-1])
            role = plistlib.loads((path / 'Info.plist').read_bytes())['CFBundleIdentifier'].split('.', 1)[1]
            self.signed.add(role)
            (path / 'Example').write_bytes(executable(20000, instruction=b'changed' if self.mutate_instruction else b'unchanged instructions'))
            (path / '_CodeSignature/CodeResources').write_bytes(b'new seal')
            if self.mutate_resource:
                (path / 'resource.txt').write_bytes(b'changed resource')
        if tool == 'codesign' and args[:2] == ('-d', '--extract-certificates'):
            Path(str(args[2]) + '0').write_bytes(self.fixture.cert)
        return b'', b''

    def run_preparation(self):
        run = lambda commands, tool, *args: self.fake_run(commands, tool, *args)
        with patch.object(subject.sys, 'platform', 'darwin'), patch.object(subject.Commands, 'run', run):
            return subject.prepare(self.config_binding, self.root / 'output', execute=True)

    def test_copy_only_signing_order_and_private_receipt(self):
        original = Path(self.config['ipa']['path']).read_bytes()
        receipt = self.run_preparation()
        self.assertTrue(receipt['passed'])
        self.assertFalse(receipt['physicalAcceptance'])
        self.assertFalse(receipt['installationPerformed'])
        self.assertTrue(receipt['ownerProvenanceReviewRequired'])
        self.assertEqual(Path(self.config['ipa']['path']).read_bytes(), original)
        signed = [Path(args[-1]).name for tool, args in self.calls if tool == 'codesign' and args[0] == '--force']
        self.assertEqual(signed, ['Notification.appex', 'Share.appex', 'Example.app'])
        self.assertEqual(set(receipt['code_bindings']), subject.ROLES)
        self.assertTrue(all(pair['original'] == pair['resigned'] for pair in receipt['code_bindings'].values()))
        self.assertTrue(all(not same for same in receipt['rawLoadCommandsEqual'].values()))
        self.assertEqual(set(receipt['signatureLayoutChanges']['app']),
                         {'signatureSize', 'linkeditFileSize', 'linkeditVMSize'})
        for tool, args in self.calls:
            if tool == 'security' and args[0] == 'verify-cert':
                self.assertEqual(args[-5:], ('-p', 'codeSign', '-L', '-R', 'offline'))

    def test_instruction_failure_preserves_receipt_and_original(self):
        self.mutate_instruction = True
        with self.assertRaisesRegex(subject.PreparationError, 'Instructions'):
            self.run_preparation()
        receipt = json.loads((self.root / 'output/preparation.json').read_text())
        self.assertFalse(receipt['passed'])
        self.assertIn('failure', receipt)
        self.assertEqual(subject.binding(Path(self.config['ipa']['path'])), self.config['ipa'])

    def test_resource_mutation_fails(self):
        self.mutate_resource = True
        with self.assertRaisesRegex(subject.PreparationError, 'resources'):
            self.run_preparation()

    def test_wrong_identity_fails_before_signing(self):
        run = self.fake_run
        def wrong_identity(commands, tool, *args):
            if tool == 'security' and args[0] == 'find-identity':
                return b'other identity', b''
            return run(commands, tool, *args)
        with patch.object(subject.sys, 'platform', 'darwin'), patch.object(subject.Commands, 'run', wrong_identity):
            with self.assertRaisesRegex(subject.PreparationError, 'private key'):
                subject.prepare(self.config_binding, self.root / 'output', execute=True)
        self.assertFalse(self.signed)

    def test_profile_api_mutation_during_signing_fails_final_validation(self):
        run = self.fake_run
        def mutate(commands, tool, *args):
            result = run(commands, tool, *args)
            if tool == 'codesign' and args[0] == '--force':
                Path(self.config['profiles']['app']['api']['path']).write_text('{}')
            return result
        with patch.object(subject.sys, 'platform', 'darwin'), patch.object(subject.Commands, 'run', mutate):
            with self.assertRaisesRegex(subject.PreparationError, 'Input changed'):
                subject.prepare(self.config_binding, self.root / 'output', execute=True)
        self.assertFalse(json.loads((self.root / 'output/preparation.json').read_text())['passed'])

    def test_target_clock_is_rechecked_after_signing(self):
        current = [self.fixture.now]
        run = self.fake_run
        def delayed(commands, tool, *args):
            result = run(commands, tool, *args)
            if tool == 'codesign' and args[:4] == ('--verify', '--strict', '--deep', args[-1]):
                current[0] += 121
            return result
        with patch.object(subject.sys, 'platform', 'darwin'), patch.object(subject.time, 'time', lambda: current[0]), \
                patch.object(subject.Commands, 'run', delayed):
            with self.assertRaisesRegex(subject.PreparationError, 'Stale target'):
                subject.prepare(self.config_binding, self.root / 'output', execute=True)


if __name__ == '__main__':
    unittest.main()
