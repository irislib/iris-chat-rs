#!/usr/bin/env python3
"""Prepare a private, production AdHoc copy of an accepted official IPA.

No device, installation, network or signing operation occurs on import. Actual
preparation requires --execute and separately supplied current private evidence.
This does not produce physical release acceptance or a provenance-owner review.
"""
from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import stat
import struct
import subprocess
import sys
import time
import unicodedata
import uuid
import zipfile

import ios_runtime_acceptance as contract

ROLES = {'app', 'notification-service', 'share-extension'}
MAX_UNPACKED = 256 * 1024**2
SIGNATURE_POLICY = 'terminal-codesign-blob-and-linkedit-extent-v1'


class PreparationError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise PreparationError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def binding(path):
    p = Path(path)
    require(p.is_absolute() and p.is_file() and not p.is_symlink(), 'Expected regular absolute input')
    with p.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(p), 'sha256': digest}


def bound(value):
    require(isinstance(value, dict) and set(value) == {'path', 'sha256'}, 'Invalid binding')
    require(binding(value['path']) == value, 'Input changed')
    return Path(value['path'])


def read_json(value):
    p = bound(value)
    require(p.stat().st_size <= 2 * 1024**2, 'JSON input too large')
    data = p.read_bytes()
    require(sha(data) == value['sha256'], 'Input changed while reading')
    return json.loads(data, object_pairs_hook=contract.unique_object,
                      parse_constant=lambda _: (_ for _ in ()).throw(PreparationError('Nonfinite JSON')))


def write_json(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')
    return binding(Path(path).resolve())


def macho_binding(data):
    """Hash all bytes except the structurally bounded code-signature envelope.

    The four normalized fields are the signature offset/size and __LINKEDIT's
    file/VM sizes. The offset itself must remain identical between two files.
    UUID, segment addresses, all other command bytes and every non-signature
    byte remain covered. Raw command bytes/digest and layout are retained too.
    Only thin, little-endian arm64 executables with a terminal signature and
    standard 16KiB __LINKEDIT VM extent are supported.
    """
    require(len(data) >= 32, 'Truncated Mach-O')
    magic, cpu, subtype, kind, count, command_size, flags, reserved = struct.unpack_from('<8I', data)
    require(magic == 0xfeedfacf and cpu == 0x100000c and kind == 2,
            'Only thin arm64 executables are supported')
    require(0 < count <= 256 and 0 < command_size <= len(data) - 32, 'Invalid command table')
    end = 32 + command_size
    offset = 32
    signature = linkedit = identity = None
    segments, referenced_ranges = [], []
    for _ in range(count):
        require(offset + 8 <= end, 'Truncated load command')
        command, size = struct.unpack_from('<II', data, offset)
        require(size >= 8 and size % 8 == 0 and offset + size <= end, 'Invalid load command size')
        if command == 0x1d:  # LC_CODE_SIGNATURE
            require(signature is None and size == 16, 'Duplicate or malformed signature command')
            signature = (offset, *struct.unpack_from('<II', data, offset + 8))
        elif command == 0x1b:  # LC_UUID
            require(identity is None and size == 24, 'Duplicate or malformed UUID')
            identity = str(uuid.UUID(bytes=bytes(data[offset + 8:offset + 24]))).upper()
        elif command == 0x19:  # LC_SEGMENT_64
            require(size >= 72, 'Truncated segment')
            name = data[offset + 8:offset + 24].rstrip(b'\0')
            vmaddr, vmsize, fileoff, filesize = struct.unpack_from('<QQQQ', data, offset + 24)
            nsects = struct.unpack_from('<I', data, offset + 64)[0]
            require(size == 72 + nsects * 80 and fileoff + filesize <= len(data), 'Invalid segment range')
            segments.append((name, fileoff, filesize))
            for section in range(nsects):
                section_at = offset + 72 + section * 80
                section_size = struct.unpack_from('<Q', data, section_at + 40)[0]
                section_off, _, reloc_off, reloc_count, section_flags = struct.unpack_from('<5I', data, section_at + 48)
                if section_flags & 0xff not in (1, 0xc, 0x12):
                    require(fileoff <= section_off and section_off + section_size <= fileoff + filesize,
                            'Section escapes its file segment')
                if reloc_count:
                    referenced_ranges.append((reloc_off, reloc_count * 8))
            if name == b'__LINKEDIT':
                require(linkedit is None and nsects == 0, 'Unsupported linkedit')
                linkedit = (offset, vmaddr, vmsize, fileoff, filesize)
        elif command == 0x2c:  # LC_ENCRYPTION_INFO_64
            require(size == 24 and struct.unpack_from('<I', data, offset + 16)[0] == 0,
                    'Encrypted executables cannot be re-signed for this route')
            start, length = struct.unpack_from('<II', data, offset + 8)
            if length:
                referenced_ranges.append((start, length))
        elif command in (0x26, 0x29, 0x80000033, 0x80000034):
            require(size == 16, 'Invalid linkedit data command')
            start, length = struct.unpack_from('<II', data, offset + 8)
            if length:
                referenced_ranges.append((start, length))
        elif command == 0x2:  # LC_SYMTAB
            require(size == 24, 'Invalid symbol command')
            symbols, symbols_count, strings, string_size = struct.unpack_from('<4I', data, offset + 8)
            referenced_ranges.extend([(symbols, 16 * symbols_count), (strings, string_size)])
        elif command == 0xb:  # LC_DYSYMTAB
            require(size == 80, 'Invalid dynamic symbol command')
            values = struct.unpack_from('<20I', data, offset)
            for index, width in ((8, 8), (10, 56), (12, 4), (14, 4), (16, 8), (18, 8)):
                if values[index + 1]:
                    referenced_ranges.append((values[index], width * values[index + 1]))
        elif command == 0x80000028:  # LC_MAIN
            require(size == 24, 'Invalid entry-point command')
            referenced_ranges.append((struct.unpack_from('<Q', data, offset + 8)[0], 1))
        elif command not in (0xc, 0xe, 0x32, 0x2a, 0x80000018, 0x8000001c):
            raise PreparationError('Unsupported load command; extend and qualify the parser explicitly')
        offset += size
    require(offset == end and signature and linkedit and identity, 'Incomplete executable metadata')
    sig_command, sig_offset, sig_size = signature
    link_command, vmaddr, vm_size, link_offset, link_size = linkedit
    require(end <= link_offset <= sig_offset and sig_size >= 12
            and sig_offset + sig_size == len(data)
            and link_offset + link_size == len(data), 'Signature must terminate the linkedit segment and file')
    require(vm_size == (link_size + 16383) // 16384 * 16384, 'Unsupported linkedit VM extent')
    for name, start, size in segments:
        if name != b'__LINKEDIT' and size:
            require(start + size <= sig_offset, 'Non-linkedit segment overlaps signature')
    for start, length in referenced_ranges:
        require(0 <= start <= sig_offset and 0 <= length <= sig_offset - start,
                'Non-signature data reference overlaps signature')
    blob_magic, blob_length, blob_count = struct.unpack_from('>III', data, sig_offset)
    require(blob_magic == 0xfade0cc0 and 12 + 8 * blob_count <= blob_length <= sig_size,
            'Invalid embedded signature envelope')
    require(not any(data[sig_offset + blob_length:]), 'Nonzero bytes outside signature envelope')
    ranges = []
    for index in range(blob_count):
        slot, start = struct.unpack_from('>II', data, sig_offset + 12 + index * 8)
        require(start >= 12 + blob_count * 8 and start + 8 <= blob_length, 'Invalid signature slot')
        length = struct.unpack_from('>I', data, sig_offset + start + 4)[0]
        require(length >= 8 and start + length <= blob_length, 'Invalid signature blob range')
        require(all(start + length <= a or start >= b for a, b in ranges), 'Overlapping signature blobs')
        ranges.append((start, start + length))
    normalized = bytearray(data[:sig_offset])
    # Both fields are explicit, not a broad removal of the load command.
    struct.pack_into('<II', normalized, sig_command + 8, sig_offset, 0)
    content_size = sig_offset - link_offset
    struct.pack_into('<Q', normalized, link_command + 32, (content_size + 16383) // 16384 * 16384)
    struct.pack_into('<Q', normalized, link_command + 48, content_size)
    return {
        'code': {'uuid': identity, 'loadCommandsSHA256': sha(normalized[32:end]),
                 'unsignedContentSHA256': sha(normalized)},
        'rawLoadCommandsSHA256': sha(data[32:end]),
        'rawLoadCommandsBase64': base64.b64encode(data[32:end]).decode(),
        'fileSHA256': sha(data), 'signaturePolicy': SIGNATURE_POLICY,
        'layout': {'signatureOffset': sig_offset, 'signatureSize': sig_size,
                   'linkeditOffset': link_offset, 'linkeditFileSize': link_size,
                   'linkeditVMSize': vm_size},
    }


def compare_code(original, resigned):
    require(original['signaturePolicy'] == resigned['signaturePolicy'] == SIGNATURE_POLICY,
            'Signature normalization policy changed')
    require(original['code'] == resigned['code'], 'Instructions or non-signature load commands changed')
    for field in ('signatureOffset', 'linkeditOffset'):
        require(original['layout'][field] == resigned['layout'][field], 'Signature or linkedit moved')
    return {'original': original['code'], 'resigned': resigned['code']}


def archive_inventory(ipa):
    with zipfile.ZipFile(ipa) as archive:
        rows = archive.infolist()
        require(0 < len(rows) <= 10000 and sum(row.file_size for row in rows) <= MAX_UNPACKED,
                'Archive exceeds preparation bound')
        names = set()
        for row in rows:
            path = PurePosixPath(row.filename)
            require(not path.is_absolute() and '..' not in path.parts and '\\' not in row.filename
                    and path.parts and path.parts[0] in {'Payload', 'Symbols', 'Signatures'},
                    'Unsafe or unsupported archive member')
            require(str(path) == row.filename.rstrip('/') and row.orig_filename == row.filename,
                    'Noncanonical archive path')
            name = unicodedata.normalize('NFD', row.filename.rstrip('/')).casefold()
            require(name not in names, 'Duplicate archive path')
            names.add(name)
            mode = row.external_attr >> 16
            require(not stat.S_ISLNK(mode) and (stat.S_IFMT(mode) in (0, stat.S_IFREG, stat.S_IFDIR)),
                    'Links and special files are unsupported')
            require(not mode & 0o7000, 'Special permission bits are unsupported')
            require(not row.flag_bits & 1, 'Encrypted ZIP entry')
        return rows


def extract(ipa, destination):
    rows = archive_inventory(ipa)
    destination.mkdir(mode=0o700)
    with zipfile.ZipFile(ipa) as archive:
        for row in rows:
            path = destination.joinpath(*PurePosixPath(row.filename).parts)
            if row.is_dir():
                path.mkdir(parents=True, exist_ok=True, mode=0o755)
                path.chmod((row.external_attr >> 16 & 0o777) or 0o755)
                continue
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
            with path.open('xb') as output, archive.open(row) as source:
                size = 0
                while block := source.read(1024**2):
                    size += len(block)
                    require(size <= row.file_size, 'Archive member expanded beyond declaration')
                    output.write(block)
                require(size == row.file_size, 'Incomplete extraction')
            path.chmod((row.external_attr >> 16 & 0o777) or 0o644)


def bundles(root, candidate):
    apps = list((root / 'Payload').glob('*.app'))
    require(len(apps) == 1, 'Expected one app')
    result = {'app': apps[0]}
    for extension in apps[0].glob('PlugIns/*.appex'):
        info = plistlib.loads((extension / 'Info.plist').read_bytes())
        point = info.get('NSExtension', {}).get('NSExtensionPointIdentifier')
        role = {'com.apple.usernotifications.service': 'notification-service',
                'com.apple.share-services': 'share-extension'}.get(point)
        require(role and role not in result, 'Unexpected or duplicate extension')
        result[role] = extension
    require(set(result) == ROLES, 'Missing required executable role')
    require(not list(apps[0].glob('Frameworks/*')) and not list(apps[0].glob('Watch/*')),
            'Additional nested code needs its own qualified checker')
    records = {}
    for role, path in result.items():
        info = plistlib.loads((path / 'Info.plist').read_bytes())
        require(info['CFBundleShortVersionString'] == candidate['apple_version']
                and info['CFBundleVersion'] == candidate['apple_build'], 'Bundle version differs')
        executable = info['CFBundleExecutable']
        require(isinstance(executable, str) and PurePosixPath(executable).name == executable,
                'Unsafe executable name')
        records[role] = {'path': path, 'executable': path / executable, 'id': info['CFBundleIdentifier']}
    return records


def permitted(value, permission):
    if isinstance(value, str) and isinstance(permission, str):
        return value == permission or (permission.endswith('*') and permission.count('*') == 1
                                       and value.startswith(permission[:-1]))
    if isinstance(value, list) and isinstance(permission, list):
        return all(any(permitted(item, allowed) for allowed in permission) for item in value)
    if isinstance(value, dict) and isinstance(permission, dict):
        return all(key in permission and permitted(item, permission[key]) for key, item in value.items())
    return type(value) is type(permission) and value == permission


def production_entitlements(original):
    require(isinstance(original, dict) and original.get('get-task-allow', False) is False,
            'Original executable permits debugging')
    require(original.get('aps-environment', 'production') == 'production', 'Non-production push environment')
    result = dict(original)
    result.pop('beta-reports-active', None)
    return result


def entitlement_permitted(key, value, permission):
    # Apple's profile is an allowlist, not a copy of signed entitlements
    # (TN2415/TN3125). Associated Domains specifically uses a scalar "*" grant
    # for the app's concrete array of domain strings. Do not generalize this
    # cross-type exception to other capabilities or change the app's values.
    if key == 'com.apple.developer.associated-domains':
        if not isinstance(value, list) or not all(type(item) is str and item for item in value):
            return False
        if permission == '*':
            return True
    return permitted(value, permission)


def require_local_identity(identities, certificate):
    """Select the exact leaf fingerprint, allowing duplicate keychain entries.

    ``find-identity -v`` can list one certificate in several keychains. These
    are the same signing selection, not competing certificate identities. Only
    the fingerprint column of a valid numbered row counts; labels and summary
    text cannot advertise a missing certificate. The SHA-256/profile checks and
    exact actual signed-leaf comparison remain separate requirements.
    """
    fingerprint = hashlib.sha1(certificate).hexdigest().upper()
    rows = re.findall(rb'(?m)^[ \t]*[0-9]+\)[ \t]+([0-9A-Fa-f]{40})[ \t]+"[^\r\n]*"[ \t]*$', identities)
    fingerprints = {row.decode('ascii').upper() for row in rows}
    require(fingerprint in fingerprints, 'Exact distribution private key is unavailable')
    return fingerprint


def validate_profile(profile, api, raw, *, device, bundle_id, entitlements, certificate, now):
    rows = api['data'] if isinstance(api['data'], list) else [api['data']]
    selected = [row for row in rows if row['attributes']['uuid'] == profile['UUID']]
    require(len(selected) == 1, 'Profile API response is missing or ambiguous')
    attributes = selected[0]['attributes']
    require(attributes['profileState'] == 'ACTIVE', 'Profile is not currently active')
    require(attributes['profileType'] == 'IOS_APP_ADHOC'
            and base64.b64decode(attributes['profileContent'], validate=True) == raw
            and attributes['uuid'] == profile['UUID'], 'Profile API content or type differs')
    creation, expiry = profile['CreationDate'], profile['ExpirationDate']
    require(creation.replace(tzinfo=dt.timezone.utc).timestamp() <= now
            < expiry.replace(tzinfo=dt.timezone.utc).timestamp(), 'Profile not currently valid')
    require(profile.get('ProvisionedDevices') == [device] and not profile.get('ProvisionsAllDevices', False),
            'Profile must provision only the reserved device')
    certificates = profile['DeveloperCertificates']
    require(any(sha(value) == sha(certificate) for value in certificates), 'Wrong distribution certificate')
    permissions = profile['Entitlements']
    team = profile['TeamIdentifier']
    require(isinstance(team, list) and len(team) == 1 and profile['ApplicationIdentifierPrefix'] == team,
            'Unsupported signing prefix')
    require(permissions.get('application-identifier') == team[0] + '.' + bundle_id
            and entitlements.get('application-identifier') == permissions['application-identifier']
            and permissions.get('get-task-allow') is False
            and entitlements.get('com.apple.developer.team-identifier') == team[0], 'Profile target or debugging mismatch')
    require(all(key in permissions and entitlement_permitted(key, value, permissions[key])
                for key, value in entitlements.items()), 'Profile does not authorize exact retained entitlements')
    return {'uuid': profile['UUID'], 'team': team[0], 'expiresUTC': expiry.isoformat(),
            'certificateSHA256': sha(certificate), 'bundleID': bundle_id, 'device': device}


def verify_target(target, now):
    require(set(target) == {'device', 'coredevice_id', 'observed_utc', 'inventory', 'reservation'}, 'Wrong target schema')
    require(0 <= now - target['observed_utc'] <= 120, 'Stale target inventory')
    reservation = read_json(target['reservation'])
    require(reservation['device'] == target['device'] and reservation['coredevice_id'] == target['coredevice_id']
            and reservation['started_utc'] <= now < reservation['expires_utc'], 'No current exact-device reservation')
    inventory = read_json(target['inventory'])
    rows = [row for row in inventory['result']['devices'] if row['identifier'] == target['coredevice_id']]
    require(len(rows) == 1 and rows[0]['hardwareProperties']['udid'] == target['device']
            and rows[0]['connectionProperties']['tunnelState'] == 'connected', 'Physical device is not connected')


def validate_inputs(config, now):
    require(config['schema'] == 'iris-adhoc-preparation-input-v1', 'Wrong preparation schema')
    contract.validate_candidate(config['candidate'])
    candidate = config['candidate']
    require(bound(config['ipa']).is_file() and config['ipa']['sha256'] == candidate['ipa_sha256'], 'Wrong official IPA')
    manifest = read_json(config['manifest'])
    require(config['manifest']['sha256'] == candidate['manifest_sha256']
            and manifest['tag'] == candidate['tag'] and manifest['commit'] == candidate['source_commit'], 'Wrong manifest')
    matches = [row for row in manifest['assets'] if row['kind'] == 'ipa' and row['platform'] == 'ios']
    require(len(matches) == 1 and matches[0]['sha256'] == candidate['ipa_sha256']
            and matches[0]['size'] == bound(config['ipa']).stat().st_size, 'IPA differs from manifest')
    accepted = read_json(config['release_acceptance'])
    require(accepted['passed'] is True and accepted['release_immutable'] is True
            and accepted['tag'] == candidate['tag'] and accepted['commit'] == candidate['source_commit']
            and accepted['ipa']['sha256'] == candidate['ipa_sha256']
            and accepted['canonical_manifest']['sha256'] == candidate['manifest_sha256']
            and accepted['ipa']['exact_run_bound'] is True
            and accepted['ipa']['release_attestation_verified'] is True, 'Release acceptance does not bind this IPA')
    authorization = read_json(config['authorization'])
    require(authorization['route'] == contract.ADHOC_CHANNEL and authorization['direct_install_authorized'] is True,
            'Explicit direct-install authorization required')
    verify_target(config['target'], now)
    require(set(config['profiles']) == ROLES and set(config['tools']) == {'security', 'codesign'}, 'Incomplete preparation inputs')
    for value in config['tools'].values():
        bound(value)
    bound(config['certificate'])
    for value in config['profiles'].values():
        bound(value['profile'])
        read_json(value['api'])
        require(0 <= now - value['api_observed_utc'] <= 300, 'Stale profile API observation')


class Commands:
    def __init__(self, tools, output):
        self.tools, self.output, self.records = tools, output, []

    def run(self, tool, *args):
        executable = bound(self.tools[tool])
        command = [str(executable), *map(str, args)]
        started = time.time()
        result = subprocess.run(command, capture_output=True, timeout=45, check=False)
        require(len(result.stdout) + len(result.stderr) <= 2 * 1024**2, 'Tool output exceeded bound')
        prefix = self.output / f'command-{len(self.records):02d}'
        prefix.with_suffix('.stdout').write_bytes(result.stdout)
        prefix.with_suffix('.stderr').write_bytes(result.stderr)
        record = {'command': command, 'startedUTC': started, 'finishedUTC': time.time(), 'exitCode': result.returncode,
                  'stdout': binding(prefix.with_suffix('.stdout')), 'stderr': binding(prefix.with_suffix('.stderr'))}
        self.records.append(record)
        write_json(prefix.with_suffix('.json'), record)
        require(result.returncode == 0, 'Preparation tool failed; preserve output before retrying')
        return result.stdout, result.stderr


def inventory(root, records):
    skipped = {row['executable'].relative_to(root).as_posix() for row in records.values()}
    for row in records.values():
        relative = row['path'].relative_to(root)
        skipped.add((relative / 'embedded.mobileprovision').as_posix())
        skipped.add((relative / '_CodeSignature/CodeResources').as_posix())
    result = {}
    for path in root.rglob('*'):
        require(not path.is_symlink(), 'Unexpected symlink after extraction/signing')
        if path.is_file():
            name = path.relative_to(root).as_posix()
            if name not in skipped:
                result[name] = {'sha256': binding(path)['sha256'], 'mode': stat.S_IMODE(path.stat().st_mode)}
    return result


def prepare(config_binding, output, *, execute=False):
    require(execute and sys.platform == 'darwin', 'Explicit macOS preparation execution required')
    config = read_json(config_binding)
    now = time.time()
    validate_inputs(config, now)
    ipa = bound(config['ipa'])
    rows = archive_inventory(ipa)
    unpacked = sum(row.file_size for row in rows)
    output = Path(output).resolve()
    require(not output.exists() and output.parent.is_dir(), 'Output must be a fresh directory')
    reserve = config['minimum_free_bytes']
    require(type(reserve) is int and reserve > 0, 'Host reserve required')
    fs = os.statvfs(output.parent)
    require(fs.f_bavail * fs.f_frsize >= reserve + unpacked + 64 * 1024**2, 'Insufficient preparation capacity')
    os.umask(0o077)
    output.mkdir(mode=0o700)
    commands = Commands(config['tools'], output)
    checker_sources = [binding(Path(__file__).resolve()), binding(Path(contract.__file__).resolve())]
    receipt = {'schema': 'iris-adhoc-preparation-v1', 'passed': False, 'candidate': config['candidate'],
               'input': config_binding, 'signaturePolicy': SIGNATURE_POLICY, 'physicalAcceptance': False,
               'installationPerformed': False, 'ownerProvenanceReviewRequired': True, 'startedUTC': now,
               'checkerSources': checker_sources}
    try:
        extracted = output / 'candidate'
        extract(ipa, extracted)
        records = bundles(extracted, config['candidate'])
        certificate = bound(config['certificate']).read_bytes()
        identities, _ = commands.run('security', 'find-identity', '-v', '-p', 'codesigning')
        fingerprint = require_local_identity(identities, certificate)
        commands.run('security', 'verify-cert', '-c', bound(config['certificate']), '-p', 'codeSign', '-L', '-R', 'offline')
        before_resources = inventory(extracted, records)
        original, profiles, entitlements = {}, {}, {}
        for role in sorted(ROLES):
            row = records[role]
            commands.run('codesign', '--verify', '--strict', row['path'])
            commands.run('codesign', '-d', '--verbose=4', row['path'])
            original_certificate = output / (role + '-original-certificate-')
            commands.run('codesign', '-d', '--extract-certificates=' + str(original_certificate), row['path'])
            leaf = Path(str(original_certificate) + '0')
            chain = []
            for index in range(10):
                certificate_path = Path(str(original_certificate) + str(index))
                if not certificate_path.exists():
                    break
                chain.extend(['-c', certificate_path])
            require(chain, 'Original certificate chain missing')
            commands.run('security', 'verify-cert', *chain, '-p', 'codeSign', '-L', '-R', 'offline')
            encoded, _ = commands.run('codesign', '-d', '--entitlements', ':-', row['path'])
            original_entitlements = plistlib.loads(encoded)
            retained = production_entitlements(original_entitlements)
            if role == 'app':
                require(retained.get('aps-environment') == 'production', 'Main app lacks production APNs')
            original[role] = {'code': macho_binding(row['executable'].read_bytes()),
                              'entitlements': original_entitlements, 'bundleID': row['id'],
                              'leafCertificate': binding(leaf)}
            profile_input = config['profiles'][role]
            raw = bound(profile_input['profile']).read_bytes()
            decoded, _ = commands.run('security', 'cms', '-D', '-i', bound(profile_input['profile']))
            profile = plistlib.loads(decoded)
            profiles[role] = validate_profile(profile, read_json(profile_input['api']), raw,
                device=config['target']['device'], bundle_id=row['id'], entitlements=retained,
                certificate=certificate, now=time.time())
            entitlement_file = output / (role + '-entitlements.plist')
            entitlement_file.write_bytes(plistlib.dumps(retained, sort_keys=True))
            entitlements[role] = (retained, entitlement_file)
        receipt['originalSigning'] = write_json(output / 'original-signing.json', original)
        receipt['profiles'] = write_json(output / 'profiles.json', {'inputs': config['profiles'], 'validated': profiles})
        # Fail before mutation if the reserved device/evidence changed during inspection.
        validate_inputs(config, time.time())
        require(read_json(config_binding) == config, 'Preparation configuration changed')
        for role in ('notification-service', 'share-extension', 'app'):
            row = records[role]
            (row['path'] / 'embedded.mobileprovision').write_bytes(bound(config['profiles'][role]['profile']).read_bytes())
            commands.run('codesign', '--force', '--sign', fingerprint, '--timestamp=none',
                         '--preserve-metadata=identifier,requirements,flags,runtime',
                         '--generate-entitlement-der', '--entitlements', entitlements[role][1], row['path'])
        after, code_bindings = {}, {}
        for role in sorted(ROLES):
            row = records[role]
            commands.run('codesign', '--verify', '--strict', row['path'])
            encoded, _ = commands.run('codesign', '-d', '--entitlements', ':-', row['path'])
            require(plistlib.loads(encoded) == entitlements[role][0], 'Re-signed entitlements differ')
            code = macho_binding(row['executable'].read_bytes())
            code_bindings[role] = compare_code(original[role]['code'], code)
            prefix = output / (role + '-certificate-')
            commands.run('codesign', '-d', '--extract-certificates=' + str(prefix), row['path'])
            require(Path(str(prefix) + '0').read_bytes() == certificate, 'Unexpected actual signing certificate')
            require((row['path'] / 'embedded.mobileprovision').read_bytes()
                    == bound(config['profiles'][role]['profile']).read_bytes(), 'Embedded profile changed')
            after[role] = code
        commands.run('codesign', '--verify', '--strict', '--deep', records['app']['path'])
        require(inventory(extracted, records) == before_resources, 'Non-signature resources or modes changed')
        require(binding(ipa) == config['ipa'] and read_json(config_binding) == config, 'Original IPA or configuration changed')
        # Recheck the entire chain and its clocks after all signing/observation.
        # No previously loaded success object substitutes for current bindings.
        validate_inputs(config, time.time())
        for source in checker_sources:
            bound(source)
        receipt.update(passed=True, target=config['target'], code_bindings=code_bindings,
                       originalCode={role: row['code'] for role, row in original.items()}, resignedCode=after,
                       signatureLayoutChanges={role: {
                           key: {'original': original[role]['code']['layout'][key], 'resigned': after[role]['layout'][key]}
                           for key in after[role]['layout']
                           if original[role]['code']['layout'][key] != after[role]['layout'][key]} for role in ROLES},
                       rawLoadCommandsEqual={role: original[role]['code']['rawLoadCommandsSHA256']
                                             == after[role]['rawLoadCommandsSHA256'] for role in ROLES},
                       resourcesSHA256=sha(json.dumps(before_resources, sort_keys=True, separators=(',', ':')).encode()),
                       appPath=str(records['app']['path']), bundleID=records['app']['id'],
                       officialIPAUnchanged=True, entitlementDelta='beta-reports-active-removal-only')
    except Exception as error:
        receipt['failure'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        receipt.update(finishedUTC=time.time(), commands=commands.records)
        write_json(output / 'preparation.json', receipt)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True, type=Path)
    parser.add_argument('--input-sha256', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    prepare({'path': str(args.input.resolve()), 'sha256': args.input_sha256}, args.out, execute=args.execute)
    print('Preparation checks passed; installation, independent review and physical acceptance remain required.')


if __name__ == '__main__':
    main()
