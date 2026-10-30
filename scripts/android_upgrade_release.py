"""Verify exact published APKs before the disposable-emulator upgrade test."""
import json
from pathlib import Path
import re
import subprocess
from android_upgrade_state import sha256
from android_upgrade_ui import PACKAGE


def command(args, log):
    result = subprocess.run(list(map(str, args)), capture_output=True, timeout=120)
    log.write_bytes(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError('Release verification failed; see ' + log.name)
    return result.stdout.decode()


def release(apk, manifest, tools, output, label):
    apk, manifest = Path(apk).resolve(), Path(manifest).resolve()
    data = json.loads(manifest.read_text())
    tag, commit = data['tag'], data['commit']
    if not re.fullmatch(r'v\d{4}\.\d{1,2}\.\d{1,2}(?:\.\d{1,2})?', tag) or not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('Invalid release provenance')
    name = 'iris-chat-' + tag + '-android-arm64.apk'
    assets = [a for a in data['assets'] if a['name'] == name]
    if len(assets) != 1 or apk.name != name or manifest.name != 'iris-chat-' + tag + '-manifest.json':
        raise ValueError('Expected canonical release APK and manifest')
    asset = assets[0]
    if sha256(apk) != asset['sha256'] or apk.stat().st_size != asset['size']:
        raise ValueError('APK does not match release manifest')
    for kind, path in (('apk', apk), ('manifest', manifest)):
        command(['gh', 'release', 'verify-asset', tag, path, '--repo', 'irislib/iris-chat-rs', '--format', 'json'],
                output / (label + '-' + kind + '-attestation.json'))
    badging = command([tools / 'aapt2', 'dump', 'badging', apk], output / (label + '-badging.txt'))
    package = re.search(r"^package: name='([^']+)' versionCode='(\d+)' versionName='([^']+)'", badging)
    if not package or package[1] != PACKAGE or package[3] != tag.removeprefix('v'):
        raise ValueError('APK package/version does not match release')
    signing = command([tools / 'apksigner', 'verify', '--print-certs', apk], output / (label + '-signature.txt'))
    signers = re.findall(r'^Signer #\d+ certificate SHA-256 digest: ([0-9a-f]+)$', signing, re.M)
    if len(signers) != 1:
        raise ValueError('Expected one verified APK signer')
    return dict(tag=tag, commit=commit, apk_sha256=asset['sha256'], manifest_sha256=sha256(manifest),
                version_code=int(package[2]), signer_sha256=signers[0], attested=True)


def pair(previous, candidate):
    if previous['tag'] == candidate['tag'] or previous['version_code'] >= candidate['version_code']:
        raise ValueError('An upgrade requires a distinct, older released version')
    if previous['signer_sha256'] != candidate['signer_sha256']:
        raise ValueError('APK signers differ; this is not an in-place upgrade')
