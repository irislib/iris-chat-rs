import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import stat
import subprocess
import sys
import re
import zipfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('release_build_inputs', ROOT/'scripts/release-build-inputs.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
PINS = json.loads(MODULE.PINS.read_text())
LINUX = 'x86_64-unknown-linux-gnu'
WINDOWS = 'x86_64-pc-windows-msvc'


def fixture_archive(path, root, extra=None):
    with tarfile.open(path, 'w:xz') as archive:
        data = b'#!/bin/sh\nexit 99 # fixture must never execute\n'
        info = tarfile.TarInfo(root+'/install.sh')
        info.size = len(data)
        info.mode = 0o755
        archive.addfile(info, io.BytesIO(data))
        if extra is not None:
            archive.addfile(extra, io.BytesIO(b'x') if extra.size else None)
    return MODULE.digest(path)


class Response(io.BytesIO):
    url = 'https://static.rust-lang.org/dist/fixture.tar.xz'


class ReleaseBuildInputsTests(unittest.TestCase):
    def test_all_release_hosts_and_cross_targets_have_exact_pins(self):
        MODULE.check_pins(PINS)
        targets, selected = MODULE.selected_components(
            PINS, 'aarch64-apple-darwin', ['aarch64-apple-ios', 'aarch64-apple-ios-sim',
                                         'x86_64-apple-darwin', 'aarch64-apple-ios'], [])
        self.assertEqual(len(selected), 6)
        self.assertEqual(len(targets), 4)
        targets, selected = MODULE.selected_components(PINS, LINUX, ['aarch64-linux-android'], ['rustfmt', 'clippy'])
        self.assertEqual(len(selected), 6)
        self.assertEqual(targets, ['aarch64-linux-android', LINUX])
        with self.assertRaisesRegex(ValueError, 'target does not match'):
            MODULE.selected_components(PINS, WINDOWS, ['aarch64-apple-ios'], [])

    def test_unknown_architecture_is_not_silently_treated_as_x86(self):
        with patch.object(MODULE.platform, 'system', return_value='Linux'), \
                patch.object(MODULE.platform, 'machine', return_value='riscv64'):
            with self.assertRaisesRegex(ValueError, 'unsupported native runner'):
                MODULE.host_target()

    def test_mutable_urls_missing_hashes_and_missing_targets_fail_closed(self):
        for field, value in [('url', 'https://static.rust-lang.org/dist/rustc-stable.tar.xz'),
                             ('url', 'https://example.invalid/rustc.tar.xz'),
                             ('sha256', '')]:
            pins = copy.deepcopy(PINS)
            pins['rust']['hosts'][LINUX]['rustc'][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                MODULE.check_pins(pins)
        pins = copy.deepcopy(PINS)
        del pins['rust']['std']['aarch64-apple-ios']
        with self.assertRaisesRegex(ValueError, 'missing or unexpected'):
            MODULE.check_pins(pins)

    def test_redirect_cannot_escape_official_https_host(self):
        for url in ['http://static.rust-lang.org/input', 'https://example.invalid/input',
                    'https://user@static.rust-lang.org/input']:
            with self.subTest(url=url), self.assertRaises(ValueError):
                MODULE.OfficialRedirects().redirect_request(None, None, 302, '', {}, url)

    def test_download_and_cached_reuse_rehash_bytes(self):
        data = b'reviewed archive bytes'
        pin = {'url':Response.url, 'sha256':hashlib.sha256(data).hexdigest()}
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(MODULE.urllib.request, 'build_opener') as opener:
            opener.return_value.open.return_value = Response(data)
            target = Path(temporary)/'input.tar.xz'
            self.assertEqual(MODULE.download(pin, target).read_bytes(), data)
            MODULE.download(pin, target)
            self.assertEqual(opener.return_value.open.call_count, 1)
            target.write_bytes(b'tampered cached archive')
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                MODULE.download(pin, target)
            self.assertEqual(opener.return_value.open.call_count, 1)

    def test_corrupt_archive_is_not_parsed_or_executed(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(MODULE.tarfile, 'open') as parse:
            path = Path(temporary)/'bad.tar.xz'
            path.write_bytes(b'not the reviewed compiler')
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                MODULE.unpack_verified(path, {'sha256':'0'*64, 'root':'rustc'}, Path(temporary)/'extract')
            parse.assert_not_called()

    def test_verified_archive_still_rejects_traversal_and_external_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'input.tar.xz'
            destination = Path(temporary)/'extract'
            root = 'rustc-fixture'
            cases = [tarfile.TarInfo(root+'/../../escape'), tarfile.TarInfo(root+'/link')]
            cases[0].size = 1
            cases[1].type = tarfile.SYMTYPE
            cases[1].linkname = '../../escape'
            for info in cases:
                with self.subTest(name=info.name):
                    checksum = fixture_archive(path, root, info)
                    with self.assertRaises(tarfile.FilterError):
                        MODULE.unpack_verified(path, {'sha256':checksum, 'root':root}, destination)
                    self.assertFalse((Path(temporary)/'escape').exists())

    def test_windows_requires_exact_git_bash_before_downloading(self):
        with patch.dict(os.environ, {'GITHUB_ACTIONS':'true'}), \
                patch.object(MODULE, 'host_target', return_value=WINDOWS), \
                patch.object(MODULE, 'download') as download:
            with self.assertRaisesRegex(ValueError, 'Git Bash'):
                MODULE.install(PINS, Path('/unused'), [], [], bash=None)
            download.assert_not_called()

    def test_windows_uses_explicit_shell_and_cygpath_with_space_safe_arguments(self):
        with tempfile.TemporaryDirectory(prefix='git bash ') as temporary:
            bash = Path(temporary)/'bash.exe'
            bash.write_bytes(b'not executable; mocked')
            with patch.object(MODULE.subprocess, 'check_output', return_value='/c/installer/install.sh\n/c/pinned rust\n') as convert:
                command = MODULE.installer_command(Path('C:/installer/install.sh'), Path('C:/pinned rust'), str(bash), True)
            self.assertEqual(convert.call_args.args[0], [str(bash), '-c', 'cygpath -u -- "$@"',
                                                       'iris-rust-paths', 'C:/installer/install.sh', 'C:/pinned rust'])
            self.assertEqual(command, [str(bash), '/c/installer/install.sh', '--prefix=/c/pinned rust', '--disable-ldconfig'])

    def test_installed_wrong_release_host_or_missing_std_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            prefix = Path(temporary)
            env = MODULE.installed_environment(prefix, False, {})
            for rustc in ['release: 1.99.1\nhost: '+LINUX,
                          'release: 1.99.0\nhost: '+WINDOWS]:
                with self.subTest(rustc=rustc), \
                        patch.object(MODULE.subprocess, 'check_output', side_effect=[rustc, 'cargo 1.99.0 (fixture)']):
                    with self.assertRaisesRegex(ValueError, 'release/host mismatch'):
                        MODULE.validate_install(prefix, LINUX, [LINUX], [], env)
            with patch.object(MODULE.subprocess, 'check_output', side_effect=[
                    'release: 1.99.0\nhost: '+LINUX, 'cargo 1.99.0 (fixture)']):
                with self.assertRaisesRegex(ValueError, 'standard library missing'):
                    MODULE.validate_install(prefix, LINUX, [LINUX], [], env)

    def test_off_path_cargo_home_cannot_preempt_verified_external_tools(self):
        # Model Cargo's documented home/bin insertion and execute only tiny fake
        # external commands. The old PATH is a negative control, not a compiler.
        dispatch = """import os, pathlib, shutil, subprocess, sys
home_bin = str(pathlib.Path(os.environ['CARGO_HOME'])/'bin')
paths = os.environ['PATH'].split(os.pathsep)
if home_bin not in paths:
    paths.insert(0, home_bin)
tool = shutil.which('cargo-'+sys.argv[1], path=os.pathsep.join(paths))
raise SystemExit(subprocess.run([tool]).returncode)
"""
        with tempfile.TemporaryDirectory(prefix='cargo home ') as temporary:
            root = Path(temporary)
            prefix, home = root/'verified', root/'existing-cache'
            for directory, identity in [(prefix/'bin', 'verified'), (home/'bin', 'ambient')]:
                directory.mkdir(parents=True)
                for name in ('fmt', 'clippy'):
                    path = directory/('cargo-'+name)
                    path.write_text('#!/bin/sh\nprintf "%s\\n" "'+identity+'"\n')
                    path.chmod(0o755)
            previous = {**os.environ, 'CARGO_HOME':str(home), 'PATH':'/usr/bin:/bin'}
            before = {**previous, 'PATH':str(prefix/'bin')+os.pathsep+previous['PATH']}
            after = MODULE.installed_environment(prefix, False, previous)
            self.assertEqual(after['CARGO_HOME'], previous['CARGO_HOME'])
            for name in ('fmt', 'clippy'):
                self.assertEqual(subprocess.check_output([sys.executable, '-c', dispatch, name], env=before, text=True).strip(), 'ambient')
                self.assertEqual(subprocess.check_output([sys.executable, '-c', dispatch, name], env=after, text=True).strip(), 'verified')
            self.assertEqual(after['PATH'].split(os.pathsep)[:2], [str(prefix/'bin'), str(home/'bin')])
        with self.assertRaisesRegex(ValueError, 'CARGO_HOME must be absolute'):
            MODULE.installed_environment(Path('/verified'), False, {'CARGO_HOME':'relative-cache'})

    def test_macos_objcopy_uses_its_bundled_library_without_loader_overrides(self):
        host = 'aarch64-apple-darwin'
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(MODULE.subprocess, 'check_output', return_value='LLVM fixture\n') as probe:
            prefix = Path(temporary)
            library = prefix/'lib/libLLVM.dylib'
            destination = prefix/'lib/rustlib'/host/'lib/libLLVM.dylib'
            destination.parent.mkdir(parents=True)
            library.write_bytes(b'verified compiler LLVM')
            previous = {'PATH':'/fixture', 'CARGO_HOME':'/existing-cache',
                        'DYLD_LIBRARY_PATH':'/ambient', 'DYLD_FALLBACK_LIBRARY_PATH':'/other'}
            self.assertEqual(MODULE.prepare_rust_helpers(prefix, host, previous), 'LLVM fixture')
            self.assertEqual(os.readlink(destination), '../../../libLLVM.dylib')
            self.assertTrue(destination.samefile(library))
            probe.assert_called_once_with(
                [str(prefix/'lib/rustlib'/host/'bin/rust-objcopy'), '--version'],
                env={'PATH':'/fixture', 'CARGO_HOME':'/existing-cache'}, text=True, timeout=30)
            self.assertIn('DYLD_LIBRARY_PATH', previous)
            MODULE.prepare_rust_helpers(prefix, host, previous)
            self.assertEqual(probe.call_count, 2, 'existing correct layout still needs a fresh probe')

    def test_macos_objcopy_missing_library_and_conflicts_never_execute(self):
        host = 'x86_64-apple-darwin'
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(MODULE.subprocess, 'check_output') as probe:
            prefix = Path(temporary)
            library = prefix/'lib/libLLVM.dylib'
            destination = prefix/'lib/rustlib'/host/'lib/libLLVM.dylib'
            destination.parent.mkdir(parents=True)
            with self.assertRaisesRegex(ValueError, 'missing its bundled LLVM'):
                MODULE.prepare_rust_helpers(prefix, host, {})
            self.assertFalse(destination.is_symlink())
            library.write_bytes(b'verified compiler LLVM')
            destination.write_bytes(b'conflicting library')
            with self.assertRaisesRegex(ValueError, 'another input'):
                MODULE.prepare_rust_helpers(prefix, host, {})
            self.assertEqual(destination.read_bytes(), b'conflicting library')
            destination.unlink()
            destination.symlink_to('../../../missing.dylib')
            with self.assertRaisesRegex(ValueError, 'another input'):
                MODULE.prepare_rust_helpers(prefix, host, {})
            self.assertEqual(os.readlink(destination), '../../../missing.dylib')
            probe.assert_not_called()
            self.assertIsNone(MODULE.prepare_rust_helpers(prefix, LINUX, {}))
            self.assertIsNone(MODULE.prepare_rust_helpers(prefix, WINDOWS, {}))
            probe.assert_not_called()

    def test_objcopy_probe_failure_keeps_install_unpublished(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            host = 'aarch64-apple-darwin'
            pins, fetch, execute, version = self.install_fixture(root, host=host)
            def fail_helper(command, **kwargs):
                if Path(command[0]).name == 'rust-objcopy':
                    raise subprocess.CalledProcessError(1, command)
                return version(command, **kwargs)
            env = {'GITHUB_ACTIONS':'true', 'GITHUB_PATH':str(root/'paths'),
                   'GITHUB_ENV':str(root/'environment')}
            with patch.dict(os.environ, env), patch.object(MODULE, 'host_target', return_value=host), \
                    patch.object(MODULE, 'download', side_effect=fetch), \
                    patch.object(MODULE.subprocess, 'run', side_effect=execute), \
                    patch.object(MODULE.subprocess, 'check_output', side_effect=fail_helper):
                with self.assertRaises(subprocess.CalledProcessError):
                    MODULE.install(pins, root/'work', [], [])
            receipt = json.loads((root/'work/receipt.json').read_text())
            self.assertFalse(receipt['passed'])
            self.assertEqual(len(receipt['components']), 3)
            self.assertFalse((root/'paths').exists())
            self.assertFalse((root/'environment').exists())

    def install_fixture(self, directory, fail_cargo=False, host=LINUX):
        pins = copy.deepcopy(PINS)
        inputs = directory/'inputs'
        inputs.mkdir()
        targets = [host, 'aarch64-linux-android'] if host == LINUX else [host]
        _, selected = MODULE.selected_components(pins, host, targets, [])
        archives = {}
        for pin in selected:
            path = inputs/(pin['root']+'.tar.xz')
            pin['sha256'] = fixture_archive(path, pin['root'])
            archives[pin['root']] = path
        def fetch(pin, destination):
            destination.write_bytes(archives[pin['root']].read_bytes())
            if fail_cargo and pin['root'].startswith('cargo-'):
                destination.write_bytes(b'corrupt')
            return destination
        def execute(command, **kwargs):
            prefix = Path(next(arg.removeprefix('--prefix=') for arg in command if arg.startswith('--prefix=')))
            (prefix/'bin').mkdir(parents=True, exist_ok=True)
            for target in targets:
                lib = prefix/'lib/rustlib'/target/'lib'
                lib.mkdir(parents=True, exist_ok=True)
                (lib/'libstd-fixture.rlib').write_bytes(b'fixture')
            if host.endswith('-apple-darwin'):
                (prefix/'lib/libLLVM.dylib').write_bytes(b'fixture LLVM')
        def version(command, **kwargs):
            if command[0] == 'git':
                return 'b'*40
            if Path(command[0]).name == 'rust-objcopy':
                self.assertEqual(Path(command[0]), directory/'work/toolchain/lib/rustlib'/host/'bin/rust-objcopy')
                return 'LLVM fixture'
            self.assertTrue(command[0].startswith(str(directory/'work/toolchain/bin')))
            self.assertEqual(kwargs['env']['CARGO'], str(directory/'work/toolchain/bin/cargo'))
            return 'rustc 1.99.0 (fixture)\nrelease: 1.99.0\nhost: '+host if command[-1] == '-Vv' else 'cargo 1.99.0 (fixture)'
        return pins, fetch, execute, version

    def test_install_exposes_verified_tools_only_after_all_components_and_targets_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            pins, fetch, execute, version = self.install_fixture(directory)
            env = {'GITHUB_ACTIONS':'true', 'GITHUB_PATH':str(directory/'paths'),
                   'GITHUB_ENV':str(directory/'environment'), 'GITHUB_SHA':'a'*40}
            with patch.dict(os.environ, env), patch.object(MODULE, 'host_target', return_value=LINUX), \
                    patch.object(MODULE, 'download', side_effect=fetch), \
                    patch.object(MODULE.subprocess, 'run', side_effect=execute) as runs, \
                    patch.object(MODULE.subprocess, 'check_output', side_effect=version):
                MODULE.install(pins, directory/'work', ['aarch64-linux-android'], [])
            self.assertEqual(runs.call_count, 4)
            receipt = json.loads((directory/'work/receipt.json').read_text())
            self.assertTrue(receipt['passed'])
            self.assertEqual(len(receipt['components']), 4)
            exports = (directory/'environment').read_text()
            self.assertIn('CARGO='+str(directory/'work/toolchain/bin/cargo'), exports)
            self.assertIn('RUSTDOC='+str(directory/'work/toolchain/bin/rustdoc'), exports)
            self.assertNotIn('RUSTUP_TOOLCHAIN', exports)
            # GitHub puts the last appended GITHUB_PATH entry first. Cargo home
            # remains explicit, but cannot outrank checksum-verified tools.
            next_paths = list(reversed((directory/'paths').read_text().splitlines()))
            self.assertEqual(next_paths[0], str(directory/'work/toolchain/bin'))
            self.assertEqual(next_paths[1], MODULE.cargo_home_bin(os.environ))

    def test_failed_component_leaves_receipt_without_publishing_tool_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            pins, fetch, execute, _ = self.install_fixture(directory, fail_cargo=True)
            env = {'GITHUB_ACTIONS':'true', 'GITHUB_PATH':str(directory/'paths'), 'GITHUB_ENV':str(directory/'environment')}
            with patch.dict(os.environ, env), patch.object(MODULE, 'host_target', return_value=LINUX), \
                    patch.object(MODULE, 'download', side_effect=fetch), \
                    patch.object(MODULE.subprocess, 'run', side_effect=execute) as runs, \
                    patch.object(MODULE, 'checkout_commit', return_value='b'*40):
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    MODULE.install(pins, directory/'work', [], [])
            self.assertEqual(runs.call_count, 1)
            self.assertFalse(json.loads((directory/'work/receipt.json').read_text())['passed'])
            self.assertFalse((directory/'paths').exists())
            self.assertFalse((directory/'environment').exists())

    def test_native_profiles_preserve_historical_versions_and_sdk_selection(self):
        MODULE.check_pins(PINS)
        self.assertEqual([pin['version'] for _, pin in MODULE.native_selection(PINS, 'macos', 'aarch64-apple-darwin')],
                         ['4.4.3', '2.46.0'])
        self.assertEqual([pin['version'] for _, pin in MODULE.native_selection(PINS, 'ios', 'aarch64-apple-darwin')],
                         ['4.4.4', '2.46.0'])
        self.assertEqual(json.loads((ROOT/'global.json').read_text())['sdk'],
                         {'version':'10.0.402', 'rollForward':'disable', 'allowPrerelease':False})
        with self.assertRaisesRegex(ValueError, 'does not match'):
            MODULE.native_selection(PINS, 'windows', LINUX)
        for change in ({'sha256':''}, {'url':'https://example.invalid/2.46.0.zip'}, {'version':'latest'}):
            pins = copy.deepcopy(PINS)
            pins['tools']['xcodegen'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                MODULE.check_pins(pins)

    def native_zip(self, path, member='tool/bin/executable', mode=0o100755, algorithm='sha256'):
        with zipfile.ZipFile(path, 'w') as archive:
            info = zipfile.ZipInfo(member)
            info.external_attr = mode << 16
            archive.writestr(info, b'fixture, never execute')
            archive.writestr('tool/share/preset', b'preserved resource')
        return {'format':'zip', 'root':'tool', 'binary':'tool/bin/executable',
                'size':path.stat().st_size,
                algorithm:hashlib.new(algorithm, path.read_bytes()).hexdigest()}

    def test_native_zip_checks_digest_before_parsing_and_preserves_resources_and_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root/'input.zip'
            pin = self.native_zip(path, algorithm='sha512')
            binary = MODULE.unpack_native(path, pin, root/'good')
            self.assertEqual(binary.read_bytes(), b'fixture, never execute')
            self.assertEqual(binary.stat().st_mode & 0o777, 0o755)
            self.assertEqual((root/'good/tool/share/preset').read_bytes(), b'preserved resource')
            path.write_bytes(b'tampered')
            with patch.object(MODULE.zipfile, 'ZipFile') as parse:
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    MODULE.unpack_native(path, pin, root/'bad')
                parse.assert_not_called()
                self.assertFalse((root/'bad').exists())

    def test_native_zip_rejects_traversal_absolute_paths_symlinks_and_special_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for index, (member, mode) in enumerate([
                    ('tool/../../escape', 0o100644), ('/tool/bin/executable', 0o100644),
                    ('C:/tool/bin/executable', 0o100644), ('tool\\escape', 0o100644),
                    ('tool/bin/executable', stat.S_IFLNK | 0o777),
                    ('tool/bin/executable', stat.S_IFIFO | 0o600)]):
                with self.subTest(member=member, mode=mode):
                    path = root/f'input-{index}.zip'
                    pin = self.native_zip(path, member, mode)
                    with self.assertRaisesRegex(ValueError, 'unsafe native ZIP'):
                        MODULE.unpack_native(path, pin, root/f'output-{index}')
                    self.assertFalse((root/'escape').exists())

    def test_native_download_is_bounded_and_rejects_wrong_redirects_or_hashes(self):
        for url in ('http://github.com/archive', 'https://foreign.invalid/archive',
                    'https://user:password@github.com/archive'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                MODULE.NativeRedirects().redirect_request(None, None, 302, '', {}, url)
        data = b'small reviewed archive'
        pin = {'url':'https://github.com/owner/repo/releases/download/1.0/input.zip',
               'sha256':hashlib.sha256(data).hexdigest(), 'size':len(data)}
        with tempfile.TemporaryDirectory() as temporary, patch.object(MODULE.urllib.request, 'build_opener') as opener:
            response = Response(data)
            response.url = pin['url']
            opener.return_value.open.return_value = response
            path = Path(temporary)/'input.zip'
            MODULE.download_native(pin, path)
            self.assertEqual(path.read_bytes(), data)
            path.write_bytes(b'corrupted cached archive')
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                MODULE.download_native(pin, path)
            self.assertEqual(opener.return_value.open.call_count, 1)

    def test_native_install_versions_and_environment_use_verified_binary_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'archives').mkdir()
            pins = copy.deepcopy(PINS)
            for name in ('cmake-windows', 'dotnet', 'nasm', 'nsis'):
                pin = pins['tools'][name]
                binary = root/('native-'+name)/pin['binary']
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b'mocked executable')
            def unpack(path, pin, destination):
                return destination/pin['binary']
            def version(command, **kwargs):
                self.assertTrue(command[0].startswith(str(root/'native-')))
                if command[0].endswith('dotnet.exe'):
                    self.assertEqual(kwargs['env']['DOTNET_ROOT'], str(root/'native-dotnet'))
                    self.assertEqual(kwargs['env']['DOTNET_MULTILEVEL_LOOKUP'], '0')
                    return '10.0.402'
                return 'cmake version 4.4.4\n' if 'cmake.exe' in command[0] else 'NASM version 3.02 compiled on fixture' if 'nasm.exe' in command[0] else 'v3.13'
            receipt = {'nativeTools':[]}
            with patch.object(MODULE, 'download_native', side_effect=lambda pin, path:path), \
                    patch.object(MODULE, 'unpack_native', side_effect=unpack), \
                    patch.object(MODULE.subprocess, 'check_output', side_effect=version):
                env, paths = MODULE.install_native(pins, 'windows', WINDOWS, root, {'PATH':'existing'}, receipt)
            self.assertEqual(len(paths), 4)
            self.assertEqual(len(receipt['nativeTools']), 4)
            self.assertEqual(env['CMAKE'], str(root/'native-cmake-windows'/pins['tools']['cmake-windows']['binary']))
            with patch.object(MODULE, 'download_native', side_effect=lambda pin, path:path), \
                    patch.object(MODULE, 'unpack_native', side_effect=unpack), \
                    patch.object(MODULE.subprocess, 'check_output', return_value='cmake version 9.0') as call:
                with self.assertRaisesRegex(ValueError, 'version mismatch'):
                    MODULE.install_native(pins, 'windows', WINDOWS, root, {'PATH':'existing'}, {'nativeTools':[]})
                self.assertEqual(call.call_count, 1)

    def test_missing_xcode_is_rejected_before_installing_native_tools(self):
        with patch.object(MODULE.shutil, 'which', return_value=None), \
                patch.object(MODULE.subprocess, 'check_output') as call:
            with self.assertRaisesRegex(ValueError, 'preinstalled Xcode tool missing'):
                MODULE.native_prerequisites('macos')
            call.assert_not_called()

    def test_native_failure_preserves_compiler_receipt_without_exporting_partial_install(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            host = 'aarch64-apple-darwin'
            pins, fetch, execute, version = self.install_fixture(root, host=host)
            env = {'GITHUB_ACTIONS':'true', 'GITHUB_PATH':str(root/'paths'), 'GITHUB_ENV':str(root/'environment')}
            with patch.dict(os.environ, env), patch.object(MODULE, 'host_target', return_value=host), \
                    patch.object(MODULE, 'native_prerequisites', return_value={'xcode':'fixture'}), \
                    patch.object(MODULE, 'download', side_effect=fetch), \
                    patch.object(MODULE.subprocess, 'run', side_effect=execute), \
                    patch.object(MODULE.subprocess, 'check_output', side_effect=version), \
                    patch.object(MODULE, 'install_native', side_effect=ValueError('native archive checksum mismatch')):
                with self.assertRaisesRegex(ValueError, 'native archive checksum'):
                    MODULE.install(pins, root/'work', [], [], tools='macos')
            receipt = json.loads((root/'work/receipt.json').read_text())
            self.assertFalse(receipt['passed'])
            self.assertEqual(receipt['sourceCommit'], 'b'*40)
            self.assertEqual(len(receipt['components']), 3)
            self.assertIn('RUSTC', receipt['versions'])
            self.assertFalse((root/'paths').exists())
            self.assertFalse((root/'environment').exists())

    def test_cli_target_probe_uses_selected_compiler_without_installing_anything(self):
        source = (ROOT/'scripts/cli-release').read_text()
        function = source.split('require_rust_target() {', 1)[1].split('\n}\n', 1)[0]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compiler = root/'fake-rustc'
            library = root/'target lib'
            library.mkdir()
            compiler.write_text('#!/bin/sh\n[ "$1 $2 $3 $4" = "--print target-libdir --target selected-target" ] || exit 91\nprintf "%s\\n" "$FIXTURE_LIB"\n')
            compiler.chmod(0o755)
            env = {**os.environ, 'RUSTC':str(compiler), 'FIXTURE_LIB':str(library)}
            command = ['bash', '-c', 'set -e; require_rust_target() {'+function+'\n}; require_rust_target selected-target']
            missing = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn('no installed standard library', missing.stderr)
            (library/'libstd-fixture.rlib').write_bytes(b'fixture')
            self.assertEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)
        self.assertNotIn('rustup target add', source)

    def test_release_hooks_use_verified_tools_and_preserve_windows_path_priority(self):
        builds = (ROOT/'.github/workflows/build-artifacts.yml').read_text()
        self.assertNotIn('rustup toolchain install stable', builds)
        self.assertNotRegex(builds, r'\b(?:brew|choco) install\b')
        for profile in ('ios', 'macos', 'windows', 'macos-cli'):
            self.assertIn('--tools '+profile, builds)
        self.assertIn('--bash "$(cygpath -w "$BASH")"', builds)
        self.assertEqual(builds.count('name: Retain reviewed build inputs'), 5)
        windows = (ROOT/'scripts/windows-build-local.ps1').read_text()
        self.assertIn('((@($env:PATH) + $paths)', windows)
        self.assertIn('& $compiler --print target-libdir --target $Target', windows)
        self.assertNotIn('rustup target list', windows)
        gradle = (ROOT/'android/app/build.gradle.kts').read_text()
        self.assertIn('System.getenv("CARGO")?.takeIf { it.isNotBlank() }', gradle)


if __name__ == '__main__':
    unittest.main()
