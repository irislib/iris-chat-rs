#!/usr/bin/env python3
"""Release input integrity tests; never execute APT, Docker, Cargo or downloads."""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class LinuxBuildInputsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.scripts = self.root / "linux/scripts"
        self.pins = self.root / "linux/build-inputs"
        self.bin = self.root / "bin"
        for path in (self.scripts, self.pins, self.bin):
            path.mkdir(parents=True)
        for name in ("pinned-apt.sh", "install-cargo-deb.sh"):
            shutil.copyfile(ROOT / "linux/scripts" / name, self.scripts / name)
        self.calls = self.root / "calls.jsonl"
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        INPUT_TEST_ROOT=str(self.root), INPUT_TEST_MODE="good")
        self.stub("sha256sum", """
import hashlib
if '--check' in sys.argv:
    for line in Path(sys.argv[-1]).read_text().splitlines():
        digest, name = line.split()
        if hashlib.sha256(Path(name).read_bytes()).hexdigest() != digest:
            sys.exit(1)
else:
    print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest(), sys.argv[1])
""")
        rows, metadata = [], {}
        for profile, suite in (("ubuntu", "resolute"), ("ubuntu-noble", "noble"), ("debian", "bookworm")):
            for suffix in ("", "-updates", "-security"):
                name = suite + suffix
                raw = "signed test metadata " + name + "\n"
                metadata[name] = raw
                rows.append("\t".join((profile, name, "http://snapshot.example/fixed",
                                       hashlib.sha256(raw.encode()).hexdigest(), "main")))
        (self.pins / "apt-snapshots.tsv").write_text("\n".join(rows) + "\n")
        (self.root / "metadata.json").write_text(json.dumps(metadata))
        self.stub("apt-get", """
args = sys.argv[1:]
command = 'update' if 'update' in args else 'install'
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps({'command': command, 'args': args}) + '\\n')
if command == 'update':
    if os.environ['INPUT_TEST_MODE'] == 'signature-failure':
        sys.exit(42)
    options = dict(arg.split('=', 1) for arg in args if '=' in arg)
    lists = Path(options['Dir::State::lists'])
    sources = Path(options['Dir::Etc::sourcelist']).read_text().splitlines()
    metadata = json.loads((root / 'metadata.json').read_text())
    for index, source in enumerate(sources):
        suite = source.split()[-2]
        raw = metadata[suite]
        if index == 0 and os.environ['INPUT_TEST_MODE'] == 'corrupt':
            raw += 'changed'
        if index == 0 and os.environ['INPUT_TEST_MODE'] == 'missing':
            continue
        (lists / ('snapshot_dists_' + suite + '_InRelease')).write_text(raw)
    if os.environ['INPUT_TEST_MODE'] == 'extra':
        (lists / 'unexpected_InRelease').write_text('additional repository')
else:
    selected = args[args.index('--') + 1:]
    versions = dict(item.split('=', 1) for item in selected if '=' in item)
    if os.environ['INPUT_TEST_MODE'] == 'stale-installed':
        versions = {name: '9.0' for name in versions}
    (root / 'installed.json').write_text(json.dumps(versions))
""")
        self.stub("apt-cache", """
args = sys.argv[1:]
assert 'Dir::State::status=/dev/null' in args
assert 'Dir::Cache::pkgcache=' in args and 'Dir::Cache::srcpkgcache=' in args
assert 'Dir::Etc::sourceparts=-' in args
version = '(none)' if os.environ['INPUT_TEST_MODE'] == 'missing-version' else '1.2.3'
print(args[-1] + ':\\n  Installed: (none)\\n  Candidate: ' + version)
""")
        self.stub("dpkg-query", """
installed = root / 'installed.json'
versions = json.loads(installed.read_text()) if installed.exists() else {}
if not installed.exists():
    initial = {'newer-installed': '9.0', 'older-installed': '0.8', 'same-installed': '1.2.3'}
    if os.environ['INPUT_TEST_MODE'] in initial:
        versions = {sys.argv[-1]: initial[os.environ['INPUT_TEST_MODE']]}
if sys.argv[-1] not in versions:
    sys.exit(1)
print('install ok installed\\t' + versions[sys.argv[-1]], end='')
""")
        self.stub("dpkg", """
assert sys.argv[1] == '--compare-versions' and sys.argv[3] == 'gt'
parts = lambda value: tuple(int(part) for part in value.split('.'))
sys.exit(0 if parts(sys.argv[2]) > parts(sys.argv[4]) else 1)
""")

    def stub(self, name, body):
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
                        "root = Path(os.environ['INPUT_TEST_ROOT'])\n" + body)
        path.chmod(0o755)

    def run_helper(self, name, *args):
        return subprocess.run(["bash", str(self.scripts / name), *args], env=self.env,
                              text=True, capture_output=True, timeout=10)

    def recorded(self):
        return [json.loads(row) for row in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def test_verified_roots_install_with_isolated_sources_and_integrity_enabled(self):
        for profile in ("ubuntu", "ubuntu-noble", "debian"):
            with self.subTest(profile=profile):
                self.calls.unlink(missing_ok=True)
                (self.root / 'installed.json').unlink(missing_ok=True)
                result = self.run_helper("pinned-apt.sh", profile, "example-package")
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = self.recorded()
                self.assertEqual([row["command"] for row in calls], ["update", "install"])
                for row in calls:
                    for flag in ("Dir::Etc::sourceparts=-", "APT::Update::Error-Mode=any",
                                 "Acquire::AllowInsecureRepositories=false",
                                 "Acquire::AllowDowngradeToInsecureRepositories=false",
                                 "APT::Get::AllowUnauthenticated=false", "APT::Get::allow-downgrades=false"):
                        self.assertIn(flag, row["args"])
                    self.assertTrue(any(arg.startswith("Dir::State::lists=/") for arg in row["args"]))
                self.assertEqual(calls[-1]["args"][-2:], ["--", "example-package=1.2.3"])
                self.assertIn("--reinstall", calls[-1]["args"])

    def test_changed_missing_extra_or_unsigned_metadata_never_installs(self):
        for profile in ("ubuntu", "ubuntu-noble", "debian"):
            for mode in ("corrupt", "missing", "extra", "signature-failure"):
                with self.subTest(profile=profile, mode=mode):
                    self.calls.unlink(missing_ok=True)
                    self.env["INPUT_TEST_MODE"] = mode
                    result = self.run_helper("pinned-apt.sh", profile, "example-package")
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual([row["command"] for row in self.recorded()], ["update"])

    def test_unsupported_profile_cannot_reach_apt(self):
        result = self.run_helper("pinned-apt.sh", "mutable", "example-package")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.recorded(), [])

    def test_newer_installed_package_is_rejected_before_install(self):
        self.env["INPUT_TEST_MODE"] = "newer-installed"
        result = self.run_helper("pinned-apt.sh", "ubuntu-noble", "example-package")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([row["command"] for row in self.recorded()], ["update"])

    def test_missing_snapshot_version_cannot_select_ambient_package(self):
        self.env["INPUT_TEST_MODE"] = "missing-version"
        result = self.run_helper("pinned-apt.sh", "ubuntu-noble", "example-package")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([row["command"] for row in self.recorded()], ["update"])

    def test_wrong_installed_version_after_install_is_rejected(self):
        self.env["INPUT_TEST_MODE"] = "stale-installed"
        result = self.run_helper("pinned-apt.sh", "ubuntu-noble", "example-package")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([row["command"] for row in self.recorded()], ["update", "install"])

    def test_same_or_older_installed_package_is_reinstalled_from_snapshot(self):
        for mode in ("same-installed", "older-installed"):
            with self.subTest(mode=mode):
                self.calls.unlink(missing_ok=True)
                (self.root / 'installed.json').unlink(missing_ok=True)
                self.env["INPUT_TEST_MODE"] = mode
                result = self.run_helper("pinned-apt.sh", "ubuntu-noble", "example-package")
                self.assertEqual(result.returncode, 0, result.stderr)
                args = self.recorded()[-1]["args"]
                self.assertEqual(args[-1], "example-package=1.2.3")
                self.assertIn("--reinstall", args)

    def prepare_cargo_deb(self, mode="good"):
        archive = self.root / "cargo-deb-3.8.0.crate"
        lock = b"version = 3\n"
        with tarfile.open(archive, "w:gz") as packed:
            member = tarfile.TarInfo("cargo-deb-3.8.0/Cargo.lock")
            member.size = len(lock)
            packed.addfile(member, io.BytesIO(lock))
        archive_digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        lock_digest = hashlib.sha256(lock).hexdigest()
        if mode == "changed-lock":
            lock_digest = "0" * 64
        (self.pins / "cargo-deb.sha256").write_text(
            f"{archive_digest}  {archive.name}\n{lock_digest}  cargo-deb-3.8.0/Cargo.lock\n")
        if mode == "changed-archive":
            archive.write_bytes(b"untrusted archive")
        self.stub("curl", """
import shutil
shutil.copyfile(root / 'cargo-deb-3.8.0.crate', sys.argv[sys.argv.index('--output') + 1])
""")
        self.stub("python3", f"""
import subprocess
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps({{'command': 'extract'}}) + '\\n')
sys.exit(subprocess.call([{sys.executable!r}, *sys.argv[1:]]))
""")
        self.stub("cargo", """
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps({'command': 'cargo', 'args': sys.argv[1:]}) + '\\n')
""")

    def test_changed_cargo_archive_is_rejected_before_parsing_or_compilation(self):
        self.prepare_cargo_deb("changed-archive")
        result = self.run_helper("install-cargo-deb.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.recorded(), [])

    def test_changed_bundled_lockfile_is_rejected_before_compilation(self):
        self.prepare_cargo_deb("changed-lock")
        result = self.run_helper("install-cargo-deb.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([row["command"] for row in self.recorded()], ["extract"])

    def test_verified_cargo_archive_installs_its_local_path_with_locked_dependencies(self):
        self.prepare_cargo_deb()
        result = self.run_helper("install-cargo-deb.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded()
        self.assertEqual([row["command"] for row in calls], ["extract", "cargo"])
        self.assertEqual(calls[-1]["args"][0:2], ["install", "--path"])
        self.assertTrue(calls[-1]["args"][2].endswith("/cargo-deb-3.8.0"))
        self.assertEqual(calls[-1]["args"][3:], ["--locked", "--force"])

    def test_release_paths_use_pinned_installers_and_images(self):
        dockerfile = (ROOT / "linux/Dockerfile").read_text()
        shared = (ROOT / "scripts/docker_stage_common.sh").read_text()
        cli = (ROOT / "scripts/cli-release").read_text()
        desktop = (ROOT / "scripts/linux-release").read_text()
        installer_test = (ROOT / "scripts/test_cli_install_docker").read_text()
        self.assertIn("ubuntu:26.04@sha256:c098726dd9b9733f3e885f8de0ef375206a311b0a317eadc40d347b1115fb944", dockerfile)
        self.assertIn("ADD --checksum=sha256:796f1389bfea13c0ed78f663b765baaff54a508e5483786bd6ece73c5e3e3cbe --chmod=0644", dockerfile)
        self.assertIn("https://curl.se/ca/cacert-2026-10-20.pem /tmp/iris-bootstrap-ca.pem", dockerfile)
        bootstrap = "RUN SSL_CERT_FILE=/tmp/iris-bootstrap-ca.pem bash /opt/iris-build/scripts/pinned-apt.sh"
        self.assertLess(dockerfile.index("ADD --checksum"), dockerfile.index(bootstrap))
        self.assertNotIn("ENV SSL_CERT_FILE", dockerfile)
        self.assertIn("rust:1.99.0-bookworm@sha256:114c7a4425406451c2866b6aafe69fe29b1b298832db1277d411ac73c82d04d6", shared)
        self.assertIn("debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3", shared)
        for body in (dockerfile, shared, cli, desktop, installer_test):
            self.assertNotIn("apt-get update", body)
            self.assertIn("pinned-apt.sh", body)
        self.assertIn("--build-arg INSTALL_CARGO_WATCH=0", desktop)
        self.assertIn("bash scripts/install-cargo-deb.sh", desktop)
        self.assertNotIn("cargo install cargo-deb", desktop)
        self.assertIn('apt-snapshots.tsv:/opt/iris-build/build-inputs/apt-snapshots.tsv:ro', installer_test)
        self.assertIn('pinned-apt.sh debian ca-certificates curl tar', installer_test)

    def test_cache_restore_prefixes_cannot_cross_snapshot_or_image_changes(self):
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        linux = workflow.split("\n  linux:\n", 1)[1].split("\n  cli:\n", 1)[0]
        cli = workflow.split("\n  cli:\n", 1)[1]
        for job, pins in ((linux, ("linux/build-inputs/**",)),
                          (cli, ("scripts/docker_stage_common.sh", "linux/build-inputs/**"))):
            key = next(line.split("key: ", 1)[1] for line in job.splitlines() if "          key: " in line)
            restore = next(line.split("restore-keys: ", 1)[1] for line in job.splitlines() if "          restore-keys: " in line)
            for path in pins:
                self.assertIn("iris-chat-rs/" + path, key)
                self.assertIn("iris-chat-rs/" + path, restore)
            self.assertIn("hashFiles(", restore)
        self.assertIn("sudo bash linux/scripts/pinned-apt.sh ubuntu-noble cmake libdbus-1-dev pkg-config", cli)

    def test_installer_smoke_mounts_only_readonly_pins_and_does_not_build(self):
        self.stub("docker", """
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps({'command': 'docker', 'args': sys.argv[1:]}) + '\\n')
""")
        archive = self.root / "candidate archive.tar.gz"
        archive.write_bytes(b"fixture; Docker is mocked")
        result = subprocess.run(["bash", str(ROOT / "scripts/test_cli_install_docker"), "--archive", str(archive)],
                                env=self.env, text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded()
        self.assertEqual(len(calls), 1)
        args = calls[0]["args"]
        self.assertEqual(args[0], "run")
        self.assertIn(str(ROOT / "linux/scripts/pinned-apt.sh") + ":/opt/iris-build/scripts/pinned-apt.sh:ro", args)
        self.assertIn(str(ROOT / "linux/build-inputs/apt-snapshots.tsv") + ":/opt/iris-build/build-inputs/apt-snapshots.tsv:ro", args)
        self.assertIn(str(archive) + ":/tmp/iris-cli.tar.gz:ro", args)
        self.assertIn("bash /opt/iris-build/scripts/pinned-apt.sh debian ca-certificates curl tar", args[-1])
        self.assertNotIn("cargo ", args[-1])


if __name__ == "__main__":
    unittest.main()
