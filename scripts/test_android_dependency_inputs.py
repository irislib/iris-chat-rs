#!/usr/bin/env python3
"""Offline source/guard tests: never launch Gradle, Java, Cargo or an installer."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import stat
import zipfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("android_dependency_phase", ROOT / "scripts/android-dependency-phase.py")
PHASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PHASE)


class AndroidDependencyInputTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "repo"
        self.java = self.base / "jdk"
        self.gradle = self.base / "gradle"
        self.sdk = self.base / "sdk"
        for relative in (PHASE.PINS, PHASE.GUARD, "scripts/android-dependency-phase.py",
                         "android/gradle/wrapper/gradle-wrapper.jar",
                         "android/gradle/wrapper/gradle-wrapper.properties",
                         "android/gradle/libs.versions.toml", "android/build.gradle.kts",
                         "android/settings.gradle.kts", "android/gradle.properties"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, path)
        for module in PHASE.MODULES:
            path = self.root / "android" / module / "build.gradle.kts"
            path.parent.mkdir(parents=True)
            path.write_text("// source fixture\n")
        (self.java / "bin").mkdir(parents=True)
        (self.java / "release").write_text('JAVA_VERSION="17.0.20"\n')
        (self.java / "bin/java").write_text("not executed")
        (self.java / "bin/java").chmod(0o755)
        self.launcher = self.gradle / "wrapper/dists/gradle-9.3.1-bin/fixture/gradle-9.3.1/bin/gradle"
        self.launcher.parent.mkdir(parents=True)
        self.launcher.write_text("not executed")
        self.launcher.chmod(0o755)
        (self.launcher.parents[2] / "gradle-9.3.1-bin.zip.ok").touch()
        (self.gradle / "caches/modules-2").mkdir(parents=True)
        pins = json.loads((self.root / PHASE.PINS).read_text())
        for package in pins["sdk_linux"]:
            path = self.sdk / package["package"].replace(";", "/") / "source.properties"
            path.parent.mkdir(parents=True)
            path.write_text("Pkg.Revision=fixture\n")
        self.proof = self.base / "gradle-proof.json"
        self.proof.write_text(json.dumps({"schema": 1,
            "distribution_sha256": pins["gradle"]["distribution_sha256"],
            "verifier_sha256": PHASE.digest(self.root / "scripts/android-dependency-phase.py"),
            "installation": str(self.launcher.parents[1].resolve()),
            "files": PHASE.installed_gradle_files(self.launcher.parents[1])}))
        self.metadata()

    def plan(self, phase="acquire", qualified=True, concurrent_growth_bytes=0):
        return PHASE.make_plan(self.root, self.java, self.gradle, self.sdk, phase,
                               self.proof if qualified else None, concurrent_growth_bytes)

    def metadata(self, xml=None):
        for module in PHASE.MODULES:
            (self.root / "android" / module / "gradle.lockfile").write_text("group:artifact:1=runtime\n")
        (self.root / "android/buildscript-gradle.lockfile").write_text("plugin:artifact:1=classpath\n")
        (self.root / "android/gradle.lockfile").write_text("tool:artifact:1=irisReleaseTools\n")
        (self.root / "android/settings-gradle.lockfile").write_text("empty=incomingCatalogForLibs0\n")
        self.xml = self.root / "android/gradle/verification-metadata.xml"
        scope = json.loads((self.root / PHASE.PINS).read_text())["gradle"]["release_input_scope"]
        components = '<component group="g" name="n" version="1"><artifact name="n.jar"><sha256 value="' + 'a' * 64 + '"/></artifact></component>'
        for coordinates in scope["tools"].values():
            for coordinate in coordinates:
                group, module, version, *classifier = coordinate.split(":")
                filename = f"{module}-{version}" + ("-" + classifier[0] if classifier else "") + ".jar"
                components += (f'<component group="{group}" name="{module}" version="{version}">'
                               f'<artifact name="{filename}"><sha256 value="' + 'a' * 64 + '"/></artifact></component>')
        self.xml.write_text(xml or (
            '<verification-metadata xmlns="https://schema.gradle.org/dependency-verification">'
            '<configuration><verify-metadata>true</verify-metadata></configuration>'
            '<components>' + components + '</components></verification-metadata>'))

    def artifact_report(self):
        scope = json.loads((self.root / PHASE.PINS).read_text())["gradle"]["release_input_scope"]
        configurations = {name: {"artifacts": [], "projects": []} for name in PHASE.expected_configurations(scope)}
        for name, result in configurations.items():
            if name in {f":app:{variant}{kind}Classpath" for variant in ("release", "selfHosted") for kind in ("Compile", "Runtime")}:
                result["artifacts"] = [{"group": "g", "module": "n", "version": "1", "file": "n.jar", "bytes": 100, "sha256": "a" * 64}]
                result["projects"] = [{"path": ":fips-ble", "variants": [{"com.android.build.api.attributes.BuildTypeAttr": "release"}]}]
            if name[1:] in scope["tools"]:
                result["attributes"] = {"org.gradle.usage": "java-runtime", "org.gradle.category": "library",
                                        "org.gradle.libraryelements": "jar", "org.gradle.dependency.bundling": "external",
                                        "org.gradle.jvm.environment": "standard-jvm"}
                for coordinate in scope["tools"][name[1:]]:
                    group, module, version, *classifier = coordinate.split(":")
                    filename = f"{module}-{version}" + ("-" + classifier[0] if classifier else "") + ".jar"
                    result["artifacts"].append({"group": group, "module": module, "version": version,
                                               "file": filename, "bytes": 100, "sha256": "a" * 64})
        return {"schema": 1, "phase": "acquire", "complete": True, "failures": {}, "releaseTasks": scope["release_tasks"], "configurations": configurations}

    def test_fixed_wrapper_and_cargo_ndk_pins_match_reviewed_evidence(self):
        pins = json.loads((ROOT / PHASE.PINS).read_text())
        self.assertEqual(pins["gradle"]["distribution_sha256"],
                         "b266d5ff6b90eada6dc3b20cb090e3731302e553a27c5d3e4df1f0d76beaff06")
        self.assertEqual(PHASE.digest(ROOT / "android/gradle/wrapper/gradle-wrapper.jar"),
                         pins["gradle"]["wrapper_jar_sha256"])
        self.assertEqual(pins["cargo_ndk"]["sha256"],
                         "903cc87cda6ab7a2ff82a74065e1c2ae5baa869546b32eb4aacabcc6ed5a670f")
        self.assertEqual(pins["cargo_ndk"]["bundled_lock"]["sha256"],
                         "94c8fd9b0269546b838bc75bc62308d8cdc51e8b8dbfc652b12c84615a54984e")
        self.assertFalse(pins["cargo_ndk"]["installer_executed"])
        self.assertEqual(sum(p["bytes"] for p in pins["sdk_linux"]), 851877003)
        self.assertTrue(all(len(p["sha256"]) == 64 for p in pins["sdk_linux"]))
        self.assertIn("Pinned SHA-256", pins["sdk_status"])

    def test_acquire_plan_is_strict_without_checksum_writes_or_application_tasks(self):
        with patch.object(PHASE.subprocess, "Popen", side_effect=AssertionError("No process admitted")):
            plan = self.plan()
        command = plan["command"]
        self.assertEqual(command[1], ":irisReleaseInputs")
        self.assertNotIn(":dependencies", command)
        for option in ("--no-daemon", "--max-workers=1", "--no-parallel",
                       "--no-build-cache", "--no-configuration-cache", "--write-locks"):
            self.assertIn(option, command)
        self.assertNotIn("--write-verification-metadata", command)
        self.assertNotIn("--offline", command)
        self.assertEqual(command[command.index("--dependency-verification") + 1], "strict")
        self.assertTrue(plan["network_allowed"])
        self.assertFalse(plan["checksum_writes_allowed"])
        self.assertNotIn("gradlew", command[0])  # Cannot bootstrap a missing distribution.
        self.assertFalse(plan["execution_admitted"])
        self.assertIn(PHASE.GUARD, plan["source_inputs"])
        self.assertIn(str((self.java / "bin/java").resolve()), plan["host_inputs"])

    def test_init_guard_rejects_every_other_task_before_execution(self):
        text = (ROOT / PHASE.GUARD).read_text()
        self.assertIn("gradle.startParameter.offline", text)
        self.assertIn("gradle.taskGraph.whenReady", text)
        self.assertIn("graph.allTasks.size() != 1 || graph.allTasks[0] != task.get()", text)
        self.assertIn("lenient = false", text)
        self.assertIn("componentFilter { it instanceof ModuleComponentIdentifier }", text)
        self.assertIn("throw new GradleException('Unresolved release inputs", text)
        scope = json.loads((ROOT / PHASE.PINS).read_text())["gradle"]["release_input_scope"]
        self.assertEqual(scope["release_tasks"], [":app:bundleRelease", ":app:assembleSelfHosted"])
        self.assertNotIn(":test-signer", scope["configurations"])
        self.assertFalse(any("Test" in name or "debug" in name or "Jacoco" in name
                             for names in scope["configurations"].values() for name in names))
        gradle = (ROOT / "android/build.gradle.kts").read_text()
        self.assertIn("lockAllConfigurations()", gradle)
        self.assertIn("lockMode.set(LockMode.STRICT)", gradle)
        self.assertLess(gradle.index('getByName("classpath")'), gradle.index("plugins {"))
        self.assertIn("buildscript.configurations.configureEach", gradle)

    def test_wrong_jdk_missing_cache_or_modified_wrapper_stops_before_execution(self):
        with patch.object(PHASE.subprocess, "Popen", side_effect=AssertionError("No process admitted")):
            (self.java / "release").write_text('JAVA_VERSION="21.0.1"\n')
            with self.assertRaisesRegex(ValueError, "Java major"):
                self.plan()
            (self.java / "release").write_text('JAVA_VERSION="17.0.20"\n')
            self.launcher.unlink()
            with self.assertRaisesRegex(ValueError, "warm Gradle"):
                self.plan()
            (self.root / "android/gradle/wrapper/gradle-wrapper.jar").write_bytes(b"wrong")
            with self.assertRaisesRegex(ValueError, "JAR checksum"):
                self.plan()

    def test_external_initialization_and_buildsrc_need_separate_review(self):
        for relative in ("init.gradle", "init.d/injected.gradle.kts", "gradle.properties"):
            with self.subTest(path=relative):
                path = self.gradle / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("unreviewed")
                with self.assertRaisesRegex(ValueError, "separate review"):
                    self.plan()
                path.unlink()
        (self.root / "android/buildSrc").mkdir()
        with self.assertRaisesRegex(ValueError, "buildSrc"):
            self.plan()

    def test_frozen_plan_detects_source_drift_before_starting_or_writing(self):
        plan = self.base / "plan.json"
        plan.write_text(json.dumps(self.plan()))
        (self.root / "android/app/build.gradle.kts").write_text("changed")
        with patch.object(PHASE.subprocess, "Popen", side_effect=AssertionError("No process admitted")):
            with self.assertRaisesRegex(ValueError, "inputs changed"):
                PHASE.execute_plan(plan, PHASE.digest(plan), self.base / "output")
        self.assertFalse((self.base / "output").exists())

    def test_verify_phase_requires_metadata_and_freezes_its_reviewed_bytes(self):
        (self.root / "android/app/gradle.lockfile").unlink()
        with self.assertRaisesRegex(ValueError, "Missing dependency lock"):
            self.plan("verify")
        self.metadata()
        plan = self.plan("verify")
        self.assertNotIn("--write-locks", plan["command"])
        self.assertNotIn("--write-verification-metadata", plan["command"])
        self.assertIn("--offline", plan["command"])
        self.assertFalse(plan["network_allowed"])
        self.assertIn("android/gradle/verification-metadata.xml", plan["source_inputs"])
        self.assertEqual(PHASE.check_metadata(self.root), {"artifacts": 3, "files": 7})

    def test_acquisition_requires_reviewed_tool_pins_but_can_create_new_locks(self):
        (self.root / "android/gradle.lockfile").unlink()
        self.assertTrue(self.plan()["network_allowed"])
        with self.assertRaisesRegex(ValueError, "tool/settings lock"):
            self.plan("verify")
        self.xml.write_text(self.xml.read_text().replace('name="aapt2-9.1.0-14792394-linux.jar"', 'name="wrong.jar"'))
        with patch.object(PHASE.subprocess, "Popen", side_effect=AssertionError("No process admitted")):
            with self.assertRaisesRegex(ValueError, "reviewed release tool checksum"):
                self.plan()

    def test_release_report_requires_complete_artifact_bytes_and_release_peer_variant(self):
        self.metadata()
        path = self.base / "release-artifacts.json"
        report = self.artifact_report()
        path.write_text(json.dumps(report))
        self.assertEqual(PHASE.check_release_report(self.root, path)["artifacts"], 3)
        mutations = [
            lambda value: value.update(complete=False),
            lambda value: value["failures"].update({":app:releaseRuntimeClasspath": "missing artifact"}),
            lambda value: value["configurations"].pop(":app:releaseRuntimeClasspath"),
            lambda value: value["configurations"][":app:releaseRuntimeClasspath"]["artifacts"].clear(),
            lambda value: value["configurations"][":app:releaseRuntimeClasspath"]["artifacts"][0].update(sha256="b" * 64),
            lambda value: value["configurations"][":app:releaseRuntimeClasspath"]["projects"][0]["variants"][0].update({"com.android.build.api.attributes.BuildTypeAttr": "debug"}),
            lambda value: value["configurations"][":app:releaseRuntimeClasspath"]["projects"].append({"path": ":unreviewed", "variants": []}),
            lambda value: value["configurations"][":irisReleaseAapt2Linux"]["artifacts"].clear(),
            lambda value: value["configurations"][":irisReleaseLint"]["attributes"].update({"org.gradle.jvm.environment": "android"}),
        ]
        for mutate in mutations:
            value = json.loads(json.dumps(report)); mutate(value); path.write_text(json.dumps(value))
            with self.subTest(mutation=mutate), self.assertRaises(ValueError):
                PHASE.check_release_report(self.root, path)

    def test_settings_and_tool_locks_are_required_and_bound(self):
        self.metadata()
        frozen = self.plan("verify")
        for relative in ("android/settings-gradle.lockfile", "android/gradle.lockfile"):
            self.assertIn(relative, frozen["source_inputs"])
            path = self.root / relative; saved = path.read_bytes(); path.unlink()
            with self.assertRaisesRegex(ValueError, "tool/settings lock"):
                PHASE.check_metadata(self.root)
            path.write_bytes(saved)

    def test_repeated_variant_artifact_requires_identical_bytes_and_checksum(self):
        path = self.base / "release-artifacts.json"
        report = self.artifact_report()
        artifacts = report["configurations"][":app:releaseRuntimeClasspath"]["artifacts"]
        artifacts.append(dict(artifacts[0]))
        path.write_text(json.dumps(report))
        self.assertEqual(PHASE.check_release_report(self.root, path)["artifacts"], 3)
        for changed in ({"bytes": artifacts[0]["bytes"] + 1}, {"sha256": "b" * 64}):
            artifacts[1] = {**artifacts[0], **changed}
            path.write_text(json.dumps(report))
            with self.subTest(changed=changed), self.assertRaisesRegex(ValueError, "Conflicting resolved artifact"):
                PHASE.check_release_report(self.root, path)
        # Identical duplicate claims still need the reviewed XML checksum.
        artifacts[0]["sha256"] = "b" * 64; artifacts[1] = dict(artifacts[0])
        path.write_text(json.dumps(report))
        with self.assertRaisesRegex(ValueError, "exact checksum"):
            PHASE.check_release_report(self.root, path)

    def test_entry_requires_eight_half_gib_floor_plus_half_gib_cushion(self):
        plan = self.base / "plan.json"
        frozen = self.plan()
        self.assertEqual(frozen["limits"]["protected_free_floor_bytes"], 17 * 1024**3 // 2)
        self.assertEqual(frozen["limits"]["entry_cushion_bytes"], 512 * 1024**2)
        plan.write_text(json.dumps(frozen))
        with patch.object(PHASE.shutil, "disk_usage", return_value=SimpleNamespace(free=9 * 1024**3 - 1)), \
                patch.object(PHASE.subprocess, "Popen") as start:
            with self.assertRaisesRegex(ValueError, "Insufficient free space"):
                PHASE.execute_plan(plan, PHASE.digest(plan), self.base / "output")
            start.assert_not_called()
        self.assertFalse((self.base / "output").exists())

    def test_unqualified_warm_gradle_cannot_start(self):
        plan = self.base / "plan.json"
        plan.write_text(json.dumps(self.plan(qualified=False)))
        with patch.object(PHASE.subprocess, "Popen") as start:
            with self.assertRaisesRegex(ValueError, "archive qualification"):
                PHASE.execute_plan(plan, PHASE.digest(plan), self.base / "output")
            start.assert_not_called()
        self.assertFalse((self.base / "output").exists())

    def test_gradle_archive_certificate_requires_matching_bytes_modes_and_no_extra_files(self):
        archive = self.base / "gradle.zip"
        with zipfile.ZipFile(archive, "w") as zip:
            member = zipfile.ZipInfo("gradle-9.3.1/bin/gradle")
            member.external_attr = (stat.S_IFREG | 0o755) << 16
            zip.writestr(member, self.launcher.read_bytes())
        pins_path = self.root / PHASE.PINS
        pins = json.loads(pins_path.read_text())
        pins["gradle"]["distribution_sha256"] = PHASE.digest(archive)
        pins_path.write_text(json.dumps(pins))
        receipt = self.base / "new-proof.json"
        installation = self.launcher.parents[1]
        PHASE.certify_gradle(self.root, archive, installation, receipt)
        PHASE.check_gradle_receipt(self.root, receipt, installation, PHASE.digest(archive))
        for change in ("mode", "content", "extra"):
            with self.subTest(change=change):
                self.launcher.write_text("changed" if change == "content" else "not executed")
                self.launcher.chmod(0o644 if change == "mode" else 0o755)
                if change == "extra":
                    (installation / "unreviewed.jar").touch()
                with self.assertRaisesRegex(ValueError, "differ from"):
                    PHASE.certify_gradle(self.root, archive, installation, self.base / (change + ".json"))
        archive.write_bytes(b"wrong archive")
        with patch.object(PHASE.zipfile, "ZipFile") as parse:
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                PHASE.certify_gradle(self.root, archive, installation, self.base / "bad.json")
            parse.assert_not_called()

    def test_metadata_cannot_disable_verification_or_accept_arbitrary_bytes(self):
        self.metadata()
        good = self.xml.read_text()
        invalid = [good.replace("true", "false"), good.replace("sha256", "sha1"),
                   good.replace("a" * 64, "not-a-checksum"),
                   good.replace("</configuration>", '<verify-signatures>true</verify-signatures></configuration>'),
                   good.replace("</configuration>", '<trusted-artifacts><trust group=".*" regex="true"/></trusted-artifacts></configuration>'),
                   good.replace('/></artifact>', '><also-trust value="' + "b" * 64 + '"/></sha256></artifact>')]
        for text in invalid:
            with self.subTest(xml=text):
                self.xml.write_text(text)
                with self.assertRaises(ValueError):
                    PHASE.check_metadata(self.root)

    def test_dependency_reports_cannot_hide_failed_resolution_with_zero_exit(self):
        plan = self.base / "plan.json"
        plan.write_text(json.dumps(self.plan()))
        output = self.base / "run"
        def fake_process(*args, **kwargs):
            self.assertEqual(args[0], self.plan()["command"])
            kwargs["stdout"].write(b"+--- missing:dependency:1 FAILED\n")
            kwargs["stdout"].flush()
            return SimpleNamespace(poll=lambda: 0)
        with patch.object(PHASE.subprocess, "Popen", side_effect=fake_process), \
                patch.object(PHASE, "stop_owned_process"), \
                patch.object(PHASE.shutil, "disk_usage", return_value=SimpleNamespace(free=100 * 1024**3)):
            with self.assertRaisesRegex(ValueError, "unresolved inputs"):
                PHASE.execute_plan(plan, PHASE.digest(plan), output)
        receipt = json.loads((output / "receipt.json").read_text())
        self.assertFalse(receipt["release_input_acceptance"])
        self.assertNotIn("phase_complete", receipt)

    def test_monitor_applies_explicit_combined_disk_allowance_and_time_limits(self):
        log = self.base / "gradle.log"
        log.touch()
        limits = self.plan()["limits"]
        process = SimpleNamespace(poll=lambda: None)
        baseline = 20 * 1024**3
        with self.assertRaisesRegex(ValueError, "disk-loss"):
            PHASE.monitor(process, log, {str(self.root): baseline}, limits,
                          free=lambda _: baseline - limits["maximum_combined_free_space_loss_bytes"] - 1)
        ticks = iter([0, limits["timeout_seconds"] + 1])
        with self.assertRaisesRegex(ValueError, "time limit"):
            PHASE.monitor(process, log, {str(self.root): baseline}, limits,
                          clock=lambda: next(ticks), free=lambda _: baseline)
        with self.assertRaisesRegex(ValueError, "floor"):
            PHASE.monitor(process, log, {str(self.root): baseline}, limits, free=lambda _: 1)

    def test_concurrent_allowance_allows_other_writes_but_preserves_floor_and_cap(self):
        log = self.base / "gradle.log"; log.touch()
        baseline = 20 * 1024**3
        limits = self.plan(concurrent_growth_bytes=1024**3)["limits"]
        finished = SimpleNamespace(poll=lambda: 0)
        # More than this phase's 512 MiB reservation is not falsely attributed
        # to it when a separately declared 1 GiB writer is concurrent.
        self.assertEqual(PHASE.monitor(finished, log, {str(self.root): baseline}, limits,
                         free=lambda _: baseline - 1024**3), 0)
        for available, message in ((baseline - 1536 * 1024**2 - 1, "Combined"),
                                   (int(8.5 * 1024**3) - 1, "floor")):
            with self.subTest(available=available), self.assertRaisesRegex(ValueError, message):
                PHASE.monitor(finished, log, {str(self.root): baseline}, limits,
                              free=lambda _: available)
        for invalid in (-1, True, 1.5):
            with self.subTest(value=invalid), self.assertRaisesRegex(ValueError, "nonnegative"):
                self.plan(concurrent_growth_bytes=invalid)

    def test_entry_reserves_combined_allowance_before_starting(self):
        frozen = self.plan(concurrent_growth_bytes=1024**3)
        plan = self.base / "plan.json"; plan.write_text(json.dumps(frozen))
        with patch.object(PHASE.shutil, "disk_usage", return_value=SimpleNamespace(free=10 * 1024**3 - 1)), \
                patch.object(PHASE.subprocess, "Popen") as start:
            with self.assertRaisesRegex(ValueError, "Insufficient free space"):
                PHASE.execute_plan(plan, PHASE.digest(plan), self.base / "output")
            start.assert_not_called()

    def test_abort_targets_only_its_owned_process_group(self):
        process = SimpleNamespace(pid=12345, wait=lambda timeout: 0)
        with patch.object(PHASE.os, "killpg") as kill:
            PHASE.stop_owned_process(process)
        self.assertEqual([call.args[0] for call in kill.call_args_list], [12345, 12345, 12345])


if __name__ == "__main__":
    unittest.main()
