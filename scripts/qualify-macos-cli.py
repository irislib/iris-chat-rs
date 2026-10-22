#!/usr/bin/env python3
"""Source-bound macOS development CLI qualification; parser smoke only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile
import time

GIB = 1024**3
INPUTS = ('core/Cargo.toml', 'core/Cargo.lock', 'core/build.rs',
          'chat-protocol/Cargo.toml', 'chat-protocol/Cargo.lock')
TUPLE = {'nvpn-fips-core': '0.4.108', 'nvpn-fips-endpoint': '0.4.108',
         'nvpn-fips-tcp-endpoint': '0.2.43', 'nostr-pubsub-fips': '0.5.38'}
COMMIT = '0bcc193ecd43e3740291ea6d774290c84dcada46'
TREE = '5f4fd4c6285fe919d68a3ff8a4b6a4241ff3c6d5'


def require(value, message):
    if not value:
        raise RuntimeError(message)


def bind(path):
    path = Path(path)
    with path.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest}


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def stop_group(child):
    if child is None or child.poll() is not None:
        return
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        child.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=10)


def checked_artifact(records):
    artifacts = [row for row in records if row.get('reason') == 'compiler-artifact'
                 and row.get('target', {}).get('name') == 'iris']
    require(len(artifacts) == 1, 'Exactly one actual CLI compiler artifact required')
    artifact = artifacts[0]
    require(artifact['target']['kind'] == ['bin'] and artifact['features'] == [],
            'Expected normal CLI with no extra features')
    expected = {'opt_level': '0', 'debuginfo': 2, 'debug_assertions': True,
                'overflow_checks': True, 'test': False}
    require(all(artifact['profile'][key] == value for key, value in expected.items()),
            'The unchanged development profile is required')
    return artifact


def checked_macho(description, libraries_output, load_commands):
    require('Mach-O 64-bit executable arm64' in description and 'universal' not in description.lower(),
            'Expected a native arm64 Mach-O executable')
    libraries = [line.strip().split(' (', 1)[0]
                 for line in libraries_output.splitlines()[1:] if line.strip()]
    require(libraries and all(path.startswith(('/usr/lib/', '/System/Library/')) for path in libraries),
            'Unexpected non-system runtime library')
    blocks = re.findall(r'cmd LC_BUILD_VERSION\n(.*?)(?=Load command|\Z)',
                        load_commands, flags=re.S)
    versions = []
    for block in blocks:
        require(re.search(r'(?mi)^\s*platform (?:1|macos)\s*$', block), 'Not a macOS platform binary')
        versions.extend(re.findall(r'(?m)^\s*minos (\d+)\.(\d+)(?:\.\d+)?\s*$', block))
    if not blocks:
        blocks = re.findall(r'cmd LC_VERSION_MIN_MACOSX\n(.*?)(?=Load command|\Z)',
                            load_commands, flags=re.S)
        versions = [version for block in blocks for version in
                    re.findall(r'(?m)^\s*version (\d+)\.(\d+)(?:\.\d+)?\s*$', block)]
    require(len(versions) == 1 and tuple(map(int, versions[0])) <= (15, 0),
            'Missing or incompatible macOS deployment target')
    return {'architecture': 'arm64', 'minimum_macos': '.'.join(versions[0]),
            'system_libraries': libraries}


def main(source, output):
    output.mkdir(mode=0o700)
    target = Path(os.environ['CARGO_TARGET_DIR']).resolve()
    target.mkdir(exist_ok=True)
    child = None
    receipt = {'schema': 'iris-hosted-development-cli/v1', 'passed': False,
               'compiled_source_commit': COMMIT, 'compiled_source_tree': TREE,
               'workflow_revision': os.environ['GITHUB_SHA'],
               'workflow_run_id': os.environ['GITHUB_RUN_ID'],
               'workflow_run_attempt': os.environ['GITHUB_RUN_ATTEMPT'],
               'limits': {'compile_seconds': 1200, 'target_bytes': 12*GIB,
                          'free_floor_bytes': 8*GIB, 'jobs': 2, 'target_measurement': 'allocated du -sk bytes'},
               'cli_parser_smoke_only': True, 'no_account_service_network_runtime': True,
               'release_binary_or_attestation_claim': False,
               'started_at_unix': time.time(), 'resource_samples': []}

    def git(*args):
        return subprocess.check_output(['git', '-C', str(source), *args], timeout=30).decode().strip()

    def source_proof():
        require(git('rev-parse', 'HEAD') == COMMIT, 'Wrong application commit')
        require(git('rev-parse', 'HEAD^{tree}') == TREE, 'Wrong application tree')
        require(not git('status', '--porcelain'), 'Application checkout changed')
        return {'commit': COMMIT, 'tree': TREE, 'clean': True,
                'inputs': {name: bind(source/name) for name in INPUTS},
                'subtrees': {name: git('rev-parse', 'HEAD:'+name)
                             for name in ('core', 'chat-protocol', 'test-fixtures')}}

    def resources():
        for attempt in range(3):
            size_result = subprocess.run(['du', '-sk', str(target)], capture_output=True, timeout=10)
            if size_result.returncode == 0:
                break
            errors = size_result.stderr.decode().splitlines()
            transient = bool(errors) and all(str(target)+'/' in error and 'No such file or directory' in error
                                             for error in errors)
            require(transient and attempt < 2, 'Cannot measure complete target size')
        size = int(size_result.stdout.split()[0]) * 1024
        free = shutil.disk_usage(target).free
        row = {'at_unix': time.time(), 'target_bytes': size, 'free_bytes': free}
        receipt['resource_samples'].append(row)
        require(size <= 12*GIB, 'Target exceeded its 12 GiB ceiling')
        require(free >= 8*GIB, 'Free disk crossed its 8 GiB floor')
        return row

    def run(argv, label, seconds):
        nonlocal child
        started = time.monotonic()
        with (output/(label+'.stdout')).open('xb') as stdout, (output/(label+'.stderr')).open('xb') as stderr:
            child = subprocess.Popen(argv, cwd=source, stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                while child.poll() is None:
                    resources()
                    require(time.monotonic()-started <= seconds, label+' deadline exceeded')
                    time.sleep(2)
                code = child.wait()
            finally:
                stop_group(child)
        resources()
        require(time.monotonic()-started <= seconds, label+' deadline exceeded')
        record = {'argv': argv, 'exit_code': code, 'seconds': time.monotonic()-started,
                  'stdout': bind(output/(label+'.stdout')), 'stderr': bind(output/(label+'.stderr'))}
        receipt.setdefault('commands', []).append(record)
        require(code == 0, label+' failed')
        return record

    def terminate(*_):
        raise RuntimeError('Qualification interrupted')

    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, terminate)
    try:
        for key in os.environ:
            require(not key.startswith('CARGO_PROFILE_'), 'Profile override forbidden: '+key)
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'):
            require(not os.environ.get(key), 'Compiler override forbidden: '+key)
        require(os.environ['SOURCE_COMMIT'] == os.environ['IRIS_BUILD_GIT_SHA'] == COMMIT, 'Full source SHA required')
        require(os.environ['SOURCE_TREE'] == TREE and os.environ['RUSTUP_TOOLCHAIN'] == '1.95.0', 'Source/toolchain mismatch')
        require(os.environ['CARGO_BUILD_JOBS'] == '2' and os.uname().sysname == 'Darwin' and os.uname().machine == 'arm64', 'Native macOS arm64/two jobs required')
        require(shutil.disk_usage(target).free >= 20*GIB, 'Need 20 GiB before the cold build')
        require((os.cpu_count() or 0) >= 2, 'Two native CPUs required')
        require(os.environ['TOKIO_WORKER_THREADS'] == os.environ['RAYON_NUM_THREADS'] == '2',
                'Two worker threads required')
        # Darwin does not expose Linux CPU affinity; these are worker limits.
        receipt['hard_cpu_affinity'] = False
        receipt['worker_limits'] = {'cargo_jobs': 2, 'rayon': 2, 'tokio': 2}
        receipt['source_before'] = source_proof()
        receipt['environment'] = {key: os.environ[key] for key in
            ('CARGO_TARGET_DIR', 'CARGO_BUILD_JOBS', 'RUSTUP_TOOLCHAIN', 'IRIS_BUILD_GIT_SHA',
             'IRIS_BUILD_CHANNEL', 'IRIS_FIPS_WEBSOCKET_SEED_URLS', 'TOKIO_WORKER_THREADS', 'RAYON_NUM_THREADS')}
        for label, argv in [('rustc', ['rustc', '-vV']), ('cargo', ['cargo', '--version']),
                            ('cmake', ['cmake', '--version']), ('xcode', ['xcodebuild', '-version']),
                            ('clang', ['xcrun', 'clang', '--version']),
                            ('sdk-path', ['xcrun', '--sdk', 'macosx', '--show-sdk-path']),
                            ('sdk-version', ['xcrun', '--sdk', 'macosx', '--show-sdk-version'])]:
            run(argv, label, 30)
        toolchain = (output/'rustc.stdout').read_text()
        require('rustc 1.95.0 ' in toolchain and 'host: aarch64-apple-darwin' in toolchain,
                'Unexpected native compiler')
        import tomllib
        packages = tomllib.loads((source/'core/Cargo.lock').read_text())['package']
        for name, version in TUPLE.items():
            require({p['version'] for p in packages if p['name'] == name} == {version}, 'Dependency tuple differs')
        run(['cargo', 'fetch', '--manifest-path', 'core/Cargo.toml', '--locked',
             '--target', 'aarch64-apple-darwin'], 'fetch', 300)
        command = ['cargo', 'build', '--manifest-path', 'core/Cargo.toml', '--locked', '--offline',
                   '--bin', 'iris', '-j', '2', '--message-format=json-render-diagnostics']
        build = run(command, 'build', 1200)
        records = [json.loads(line) for line in (output/'build.stdout').read_text().splitlines()
                   if line.startswith('{')]
        artifact = checked_artifact(records)
        binary = Path(artifact['executable'])
        require(binary.resolve() == target/'debug/iris', 'Unexpected binary output')
        require(Path(artifact['manifest_path']).resolve() == source/'core/Cargo.toml', 'Unexpected source manifest')
        scripts = [row for row in records if row.get('reason') == 'build-script-executed'
                   and ['IRIS_BUILD_GIT_SHA', COMMIT] in row.get('env', [])]
        require(len(scripts) == 1, 'Exactly one full-SHA build script record required')
        receipt['source_after'] = source_proof()
        require(receipt['source_before'] == receipt['source_after'], 'Application source changed')
        build.update(source_clean_before=True, source_clean_after=True,
                     source_commit_before=COMMIT, source_commit_after=COMMIT,
                     target='aarch64-apple-darwin', environment=receipt['environment'])
        manifest = {'schema': 'iris-cli-source-build/v1', 'source_commit': COMMIT,
                    'source_tree': TREE, 'source_path': str(source),
                    'source_subtrees': receipt['source_before']['subtrees'],
                    'source_inputs': receipt['source_before']['inputs'], 'dependency_tuple': TUPLE,
                    'binary': bind(binary), 'target': 'aarch64-apple-darwin', 'build': build,
                    'workflow_revision': receipt['workflow_revision'], 'release_binary_or_attestation_claim': False}
        write(output/'artifact-manifest.json', manifest)
        write(output/'build-script-proof.json', {'records': scripts, 'source_commit': COMMIT})
        for label, argv in [('macho-header', ['otool', '-hv']), ('macho-libraries', ['otool', '-L']),
                            ('macho-load-commands', ['otool', '-l']), ('binary-file', ['file'])]:
            run([*argv, str(binary)], label, 30)
        receipt['macho'] = checked_macho(
            (output/'binary-file.stdout').read_text(),
            (output/'macho-libraries.stdout').read_text(),
            (output/'macho-load-commands.stdout').read_text())
        smoke_data = output/'smoke-data-must-remain-absent'
        require(not smoke_data.exists(), 'Smoke data path already exists')
        os.environ['IRIS_DATA_DIR'] = str(smoke_data)
        run([str(binary), '--version'], 'smoke-version', 15)
        run([str(binary), 'invite', 'create', '--help'], 'smoke-full-invite-help', 15)
        require((output/'smoke-version.stdout').read_text().startswith('iris '), 'CLI version smoke failed')
        require('--full' in (output/'smoke-full-invite-help.stdout').read_text(), 'Full invite option missing')
        require(not smoke_data.exists(), 'Parser smoke unexpectedly created application data')
        receipt['parser_smoke_passed'] = True
        receipt['artifact_manifest'] = bind(output/'artifact-manifest.json')
        receipt['compiler_artifact'] = artifact
        receipt['binary'] = manifest['binary']
        archive = output/'iris-macos-cli-development.tar.gz'
        export_started = time.monotonic()

        class BoundedArchive:
            def __init__(self, stream):
                self.stream = stream
                self.bytes_written = 0

            def write(self, data):
                require(time.monotonic()-export_started <= 180, 'Export deadline exceeded')
                require(self.bytes_written+len(data) <= GIB, 'Compressed export exceeds 1 GiB')
                require(shutil.disk_usage(output).free-len(data) >= 8*GIB, 'Export disk floor crossed')
                count = self.stream.write(data)
                self.bytes_written += count
                return count

            def tell(self):
                return self.bytes_written

            def flush(self):
                self.stream.flush()

        with archive.open('xb') as stream:
            with tarfile.open(fileobj=BoundedArchive(stream), mode='w:gz', compresslevel=1) as tar:
                tar.add(binary, arcname='target/debug/iris', recursive=False)
        require(bind(binary) == manifest['binary'], 'Binary changed during export')
        require(archive.stat().st_size <= 1024**3, 'Compressed export exceeds its 1 GiB ceiling')
        resources()
        receipt['archive'] = bind(archive)
        receipt['no_full_source_copy'] = True
        receipt['passed'] = True
    except BaseException as error:
        receipt['error'] = type(error).__name__+': '+str(error)
        stop_group(child)
    finally:
        receipt['completed_at_unix'] = time.time()
        write(output/'qualification-receipt.json', receipt)
    print(json.dumps({'passed': receipt['passed'], 'error': receipt.get('error'),
                      'binary': receipt.get('binary'), 'archive': receipt.get('archive')}))
    return 0 if receipt['passed'] else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    raise SystemExit(main(args.source.resolve(), args.output.resolve()))
