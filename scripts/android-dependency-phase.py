#!/usr/bin/env python3
"""Acquire pinned release dependencies, then verify them in a separate offline run.

This reuses an existing Gradle/JDK/SDK installation. It does not download or
verify those installed payloads. Both phases require reviewed checksums; neither
can generate or change them. Acquire may fetch dependencies and write locks.
Gradle may compile its Kotlin build scripts while configuring projects; the
init script forbids application, test, Cargo and other execution tasks.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PINS = "scripts/android-release-inputs.json"
GUARD = "scripts/android-dependency-phase.init.gradle"
MODULES = ("app", "fips-ble", "test-signer")
RELEASE_MODULES = ("app", "fips-ble")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def properties(path):
    result = {}
    for line in Path(path).read_text().splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            name, value = line.split("=", 1)
            result[name.strip()] = value.strip().replace(r"\:", ":").strip('"')
    return result


def metadata_paths(root):
    paths = list((root / "android").glob("**/*gradle.lockfile"))
    xml = root / "android/gradle/verification-metadata.xml"
    if xml.exists():
        paths.append(xml)
    return sorted(path for path in paths if "build" not in path.relative_to(root).parts)


def source_bindings(root):
    paths = [root / PINS, root / GUARD, root / "scripts/android-dependency-phase.py"]
    android = root / "android"
    for folder in [android, *(android / name for name in MODULES)]:
        paths.extend(path for path in folder.glob("*.gradle*") if path.is_file())
        paths.extend(folder.glob("gradle.properties"))
    paths.extend(android / name for name in (
        "gradle/libs.versions.toml", "gradle/wrapper/gradle-wrapper.jar",
        "gradle/wrapper/gradle-wrapper.properties"))
    paths.extend(metadata_paths(root))
    for path in (android / "local.properties", root / "release.env"):
        if path.exists():
            paths.append(path)  # Only hashes are retained; never configuration secrets.
    return {str(path.relative_to(root)): digest(path) for path in sorted(set(paths))}


def check_metadata(root, require_locks=True):
    if require_locks:
        for module in RELEASE_MODULES:
            lock = root / "android" / module / "gradle.lockfile"
            require(lock.is_file() and lock.stat().st_size > 0, f"Missing dependency lock: {module}")
        require((root / "android/buildscript-gradle.lockfile").is_file(),
                "Missing root plugin classpath lock")
        for name in ("gradle.lockfile", "settings-gradle.lockfile"):
            require((root / "android" / name).is_file(), "Missing release tool/settings lock: " + name)
    path = root / "android/gradle/verification-metadata.xml"
    require(path.is_file(), "Missing dependency verification metadata")
    tree = ET.fromstring(path.read_bytes())
    for node in tree.iter():
        node.tag = node.tag.rsplit("}", 1)[-1]
    require(tree.findtext("configuration/verify-metadata") == "true",
            "Dependency metadata itself must be verified")
    require(tree.findtext("configuration/verify-signatures") in (None, "false"),
            "This phase requires artifact checksums rather than signature-only trust")
    require(not tree.findall(".//trusted-artifacts/trust"), "Unreviewed verification bypass")
    artifacts = tree.findall("components/component/artifact")
    require(bool(artifacts), "Empty dependency verification metadata")
    for artifact in artifacts:
        hashes = artifact.findall("sha256")
        require(len(hashes) == 1 and re.fullmatch("[0-9a-f]{64}", hashes[0].get("value", "")),
                "Each artifact requires one reviewed SHA-256")
        require(not hashes[0].findall("also-trust"), "Multiple artifact checksums require review")
    entries = {(component.get("group"), component.get("name"), component.get("version"), artifact.get("name"))
               for component in tree.findall("components/component") for artifact in component.findall("artifact")}
    scope = json.loads((root / PINS).read_text())["gradle"]["release_input_scope"]
    for coordinates in scope["tools"].values():
        for coordinate in coordinates:
            group, module, version, *classifier = coordinate.split(":")
            filename = f"{module}-{version}" + ("-" + classifier[0] if classifier else "") + ".jar"
            require((group, module, version, filename) in entries,
                    "Missing reviewed release tool checksum: " + coordinate)
    return {"artifacts": len(artifacts), "files": len(metadata_paths(root))}


def expected_configurations(scope):
    return ({f"{project}:buildscript:classpath" for project in scope["buildscript_projects"]} |
            {f"{project}:{name}" for project, names in scope["configurations"].items() for name in names} |
            {":" + name for name in scope["tools"]})


def check_release_report(root, report_path):
    """Require actual artifact access, not merely successful graph reporting."""
    require(report_path.is_file() and report_path.stat().st_size <= 16 * 1024**2,
            "Missing or oversized release artifact report")
    report = json.loads(report_path.read_text())
    scope = json.loads((root / PINS).read_text())["gradle"]["release_input_scope"]
    require(report.get("schema") == 1 and report.get("complete") is True and report.get("failures") == {},
            "Release artifact resolution is incomplete")
    require(report.get("releaseTasks") == scope["release_tasks"] and
            set(report.get("configurations", {})) == expected_configurations(scope),
            "Release artifact configuration coverage changed")
    tree = ET.parse(root / "android/gradle/verification-metadata.xml").getroot()
    for node in tree.iter():
        node.tag = node.tag.rsplit("}", 1)[-1]
    verified = {}
    for component in tree.findall("components/component"):
        for artifact in component.findall("artifact"):
            key = (component.get("group"), component.get("name"), component.get("version"), artifact.get("name"))
            require(key not in verified, "Duplicate artifact verification entry")
            verified[key] = artifact.find("sha256").get("value")
    observed = set()
    for name, result in report["configurations"].items():
        require(isinstance(result.get("artifacts"), list) and isinstance(result.get("projects"), list),
                "Invalid release artifact result")
        per_configuration = {}
        for artifact in result["artifacts"]:
            key = tuple(artifact.get(field) for field in ("group", "module", "version", "file"))
            require(all(isinstance(value, str) and value for value in key), "Invalid resolved artifact")
            require(type(artifact.get("bytes")) is int and artifact["bytes"] > 0 and
                    re.fullmatch("[0-9a-f]{64}", artifact.get("sha256", "")), "Invalid resolved artifact payload")
            payload = (artifact["bytes"], artifact["sha256"])
            # Different selected variants can expose the same immutable file.
            # Repeated observations must agree in both content and length.
            require(key not in per_configuration or per_configuration[key] == payload,
                    "Conflicting resolved artifact observations: " + str(key))
            require(verified.get(key) == artifact["sha256"], "Resolved release artifact is missing its exact checksum: " + str(key))
            per_configuration[key] = payload; observed.add(key)
        if name in {f":app:{variant}{kind}Classpath" for variant in ("release", "selfHosted") for kind in ("Compile", "Runtime")}:
            require(bool(per_configuration), "Empty release application classpath")
            peers = [project for project in result["projects"] if project["path"] != ":app"]
            require(len(peers) == 1 and peers[0]["path"] == ":fips-ble" and
                    any(variant.get("com.android.build.api.attributes.BuildTypeAttr") == "release"
                        for variant in peers[0]["variants"]), "Release must select the fips-ble release variant")
        if name[1:] in scope["tools"]:
            require(all(result.get("attributes", {}).get(key) == value for key, value in {
                "org.gradle.usage": "java-runtime", "org.gradle.category": "library",
                "org.gradle.libraryelements": "jar", "org.gradle.dependency.bundling": "external",
                "org.gradle.jvm.environment": "standard-jvm"}.items()),
                "Release tools must select standard JVM runtime artifacts")
            for coordinate in scope["tools"][name[1:]]:
                group, module, version, *classifier = coordinate.split(":")
                filename = f"{module}-{version}" + ("-" + classifier[0] if classifier else "") + ".jar"
                require((group, module, version, filename) in per_configuration,
                        "Required release tool artifact was not resolved: " + coordinate)
    return {"configurations": len(report["configurations"]), "artifacts": len(observed),
            "report_sha256": digest(report_path)}


def installed_gradle_files(installation):
    result = {}
    for path in sorted(installation.rglob("*")):
        require(not path.is_symlink(), "Gradle installation contains an unreviewed symlink")
        if path.is_file():
            result[str(path.relative_to(installation))] = {
                "sha256": digest(path), "bytes": path.stat().st_size,
                "executable": stat.S_IMODE(path.stat().st_mode) & 0o111}
    return result


def certify_gradle(root, archive_path, installation, receipt_path):
    """Read an already obtained archive and installed files; never install or execute."""
    pins = json.loads((root / PINS).read_text())["gradle"]
    require(digest(archive_path) == pins["distribution_sha256"], "Gradle archive checksum mismatch")
    expected = {}
    prefix = "gradle-" + pins["version"] + "/"
    with zipfile.ZipFile(archive_path) as archive:
        for member in archive.infolist():
            name = member.filename
            require(name.startswith(prefix) and ".." not in name.split("/") and "\\" not in name,
                    "Unexpected Gradle archive path")
            if member.is_dir():
                continue
            relative = name[len(prefix):]
            require(relative and relative not in expected and
                    stat.S_IFMT(member.external_attr >> 16) in (0, stat.S_IFREG),
                    "Unexpected Gradle file metadata")
            value = hashlib.sha256()
            with archive.open(member) as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    value.update(block)
            expected[relative] = {"sha256": value.hexdigest(), "bytes": member.file_size,
                                  "executable": (member.external_attr >> 16) & 0o111}
    installation = Path(installation).resolve()
    require(expected and installed_gradle_files(installation) == expected,
            "Installed Gradle files or executable modes differ from the pinned archive")
    receipt = {"schema": 1, "distribution_sha256": pins["distribution_sha256"],
               "verifier_sha256": digest(root / "scripts/android-dependency-phase.py"),
               "installation": str(installation), "files": expected}
    with Path(receipt_path).open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")


def check_gradle_receipt(root, receipt_path, installation, distribution_sha256):
    receipt = json.loads(Path(receipt_path).read_text())
    require(receipt.get("schema") == 1 and receipt["distribution_sha256"] == distribution_sha256 and
            receipt["verifier_sha256"] == digest(root / "scripts/android-dependency-phase.py") and
            receipt["installation"] == str(installation.resolve()) and
            receipt["files"] == installed_gradle_files(installation),
            "Gradle archive qualification receipt or installed payload changed")


def make_plan(root, java_home, gradle_home, sdk, phase, gradle_receipt=None, concurrent_growth_bytes=0):
    root, java_home, gradle_home, sdk = (Path(value).resolve() for value in
                                       (root, java_home, gradle_home, sdk))
    require(phase in ("acquire", "verify"), "Unknown metadata phase")
    pins = json.loads((root / PINS).read_text())
    gradle = pins["gradle"]
    require(gradle["tasks"] == [":irisReleaseInputs"] and
            re.search(r'^agp\s*=\s*"' + re.escape(gradle["release_input_scope"]["agp_version"]) + r'"\s*$',
                      (root / "android/gradle/libs.versions.toml").read_text(), re.MULTILINE),
            "Release input scope must match the reviewed AGP version")
    wrapper = properties(root / "android/gradle/wrapper/gradle-wrapper.properties")
    require(wrapper.get("distributionUrl") == gradle["distribution_url"] and
            wrapper.get("distributionSha256Sum") == gradle["distribution_sha256"],
            "Gradle wrapper distribution differs from reviewed pin")
    require(digest(root / "android/gradle/wrapper/gradle-wrapper.jar") ==
            gradle["wrapper_jar_sha256"], "Gradle wrapper JAR checksum mismatch")
    require(properties(java_home / "release").get("JAVA_VERSION", "").split(".")[0] ==
            str(gradle["java_major"]), "The metadata phase requires the reviewed Java major")
    java = java_home / "bin/java"
    require(java.is_file() and os.access(java, os.X_OK), "Java executable is unavailable")
    candidates = list((gradle_home / "wrapper/dists" / f"gradle-{gradle['version']}-bin").glob(
        f"*/gradle-{gradle['version']}/bin/gradle"))
    require(len(candidates) == 1, "Expected exactly one existing warm Gradle installation")
    launcher = candidates[0]
    require(os.access(launcher, os.X_OK) and
            (launcher.parents[2] / f"gradle-{gradle['version']}-bin.zip.ok").is_file(),
            "Warm Gradle installation is incomplete; no distribution download is admitted")
    require((gradle_home / "caches/modules-2").is_dir(), "Warm dependency cache is missing")
    for folder in (gradle_home, launcher.parents[1]):
        init = list(folder.glob("init.gradle*")) + list((folder / "init.d").glob("*.gradle*"))
        require(not init, "External Gradle init scripts require separate review")
    require(not (gradle_home / "gradle.properties").exists(),
            "Global Gradle properties require separate review")
    require(not (root / "android/buildSrc").exists(), "buildSrc requires separate phase review")
    settings = (root / "android/settings.gradle.kts").read_text()
    require("includeBuild" not in settings, "Included builds require separate phase review")
    local = root / "android/local.properties"
    if local.exists() and "sdk.dir" in properties(local):
        require(Path(properties(local)["sdk.dir"]).resolve() == sdk, "Local SDK override differs")
    host_files = [java, java_home / "release", launcher]
    if gradle_receipt:
        gradle_receipt = Path(gradle_receipt).resolve()
        check_gradle_receipt(root, gradle_receipt, launcher.parents[1], gradle["distribution_sha256"])
        host_files.append(gradle_receipt)
    for package in pins["sdk_linux"]:
        path = sdk / package["package"].replace(";", "/") / "source.properties"
        require(path.is_file(), f"Required installed SDK metadata is missing: {package['package']}")
        host_files.append(path)
    check_metadata(root, require_locks=phase == "verify")
    require(type(concurrent_growth_bytes) is int and concurrent_growth_bytes >= 0,
            "Concurrent growth allowance must be a nonnegative byte count")
    limits = dict(pins["dependency_phase_limits"])
    limits["concurrent_growth_bytes"] = concurrent_growth_bytes
    limits["maximum_combined_free_space_loss_bytes"] = limits["reserved_growth_bytes"] + concurrent_growth_bytes
    command = [str(launcher), *gradle["tasks"], "--no-daemon",
               f"--max-workers={limits['workers']}", "--no-parallel", "--no-configuration-cache",
               "--no-build-cache", "--no-watch-fs", "--console=plain", "--init-script", str(root / GUARD),
               f"-Dorg.gradle.jvmargs=-Xmx{limits['heap_mib']}m -Dfile.encoding=UTF-8",
               "-Dorg.gradle.java.installations.auto-download=false",
               "-Pandroid.builder.sdkDownload=false", "--dependency-verification", "strict"]
    command.append("--write-locks" if phase == "acquire" else "--offline")
    return {"schema": 1, "phase": phase, "root": str(root), "java_home": str(java_home),
            "gradle_home": str(gradle_home), "sdk": str(sdk), "command": command, "limits": limits,
            "source_inputs": source_bindings(root),
            "host_inputs": {str(path): digest(path) for path in host_files},
            "gradle_receipt": str(gradle_receipt) if gradle_receipt else None,
            "gradle_payload_verified": bool(gradle_receipt),
            "network_allowed": phase == "acquire",
            "cache_completeness": "Unknown; missing checksums stop acquisition; offline verification never retries online.",
            "installed_payload_verification": "Not established by this metadata-only phase.",
            "execution_admitted": False, "checksum_writes_allowed": False}


def stop_owned_process(process):
    # The launcher/Gradle single-use daemon inherit this task's new process group.
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            return
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            continue
        # Kill remaining descendants as well if the launcher exits first.


def monitor(process, log, volumes, limits, clock=time.monotonic, sleep=time.sleep,
            free=lambda path: shutil.disk_usage(path).free):
    started = clock()
    while True:
        for path, baseline in volumes.items():
            available = free(path)
            require(available >= limits["protected_free_floor_bytes"], "Protected free-space floor reached")
            require(baseline - available <= limits["maximum_combined_free_space_loss_bytes"],
                    "Combined concurrent disk-loss allowance reached")
        require(log.stat().st_size <= 32 * 1024 * 1024, "Metadata phase log-size limit reached")
        require(clock() - started <= limits["timeout_seconds"], "Metadata phase time limit reached")
        result = process.poll()
        if result is not None:
            return result
        sleep(limits["sample_seconds"])


def execute_plan(plan_path, expected_sha256, output):
    require(digest(plan_path) == expected_sha256, "Frozen plan checksum mismatch")
    plan = json.loads(Path(plan_path).read_text())
    require(plan.get("gradle_payload_verified") is True and plan.get("gradle_receipt"),
            "Pinned Gradle archive qualification is required before execution")
    current = make_plan(plan["root"], plan["java_home"], plan["gradle_home"], plan["sdk"], plan["phase"], plan["gradle_receipt"],
                        plan["limits"]["concurrent_growth_bytes"])
    require(current == plan, "Frozen plan inputs changed; inspect and freeze a new plan")
    output = Path(output)
    require(output.is_absolute() and not output.exists() and output.parent.is_dir(),
            "Output must be a new absolute directory with an existing parent")
    root = Path(plan["root"])
    volumes = {}
    devices = set()
    for path in (root, Path(plan["gradle_home"]), output.parent):
        if path.stat().st_dev not in devices:
            devices.add(path.stat().st_dev)
            volumes[str(path)] = shutil.disk_usage(path).free
    entry_floor = (plan["limits"]["protected_free_floor_bytes"] +
                   max(plan["limits"]["entry_cushion_bytes"],
                       plan["limits"]["maximum_combined_free_space_loss_bytes"]))
    require(all(value >= entry_floor for value in volumes.values()),
            "Insufficient free space before metadata phase")
    output.mkdir()
    (output / "tmp").mkdir()
    environment = {"HOME": str(Path.home()), "PATH": str(Path(plan["java_home"]) / "bin") + os.pathsep + os.defpath,
                   "JAVA_HOME": plan["java_home"], "GRADLE_USER_HOME": plan["gradle_home"],
                   "ANDROID_HOME": plan["sdk"], "ANDROID_SDK_ROOT": plan["sdk"],
                   "TMPDIR": str(output / "tmp"), "LANG": "en_US.UTF-8",
                   "IRIS_ANDROID_DEPENDENCY_PHASE": plan["phase"],
                   "IRIS_ANDROID_DEPENDENCY_REPORT": str(output / "release-artifacts.json")}
    log = output / "gradle.log"
    receipt = {"phase": plan["phase"], "plan_sha256": expected_sha256,
               "release_input_acceptance": False, "exit_code": None,
               "disk_accounting": "Explicit combined concurrent allowance; not attributed phase growth",
               "limits": plan["limits"]}
    process = None
    try:
        with log.open("wb") as stream:
            process = subprocess.Popen(plan["command"], cwd=root / "android", env=environment,
                                       stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            receipt["exit_code"] = monitor(process, log, volumes, plan["limits"])
        require(receipt["exit_code"] == 0, "Gradle metadata phase failed; see gradle.log")
        require(not re.search(r"\bFAILED\b|Could not resolve|Could not find", log.read_text(errors="replace")),
                "Dependency report contains unresolved inputs despite a zero Gradle exit status")
        receipt["metadata"] = check_metadata(root)
        receipt["release_artifacts"] = check_release_report(root, output / "release-artifacts.json")
        report = json.loads((output / "release-artifacts.json").read_text())
        require(report["phase"] == plan["phase"], "Release artifact phase changed")
        metadata_name = "android/gradle/verification-metadata.xml"
        require(digest(root / metadata_name) == plan["source_inputs"][metadata_name], "Reviewed checksums changed during dependency phase")
        if plan["phase"] == "verify":
            require(source_bindings(root) == plan["source_inputs"], "Offline verification changed source or dependency locks")
        receipt["checksum_writes_allowed"] = False
        receipt["lock_review_required"] = plan["phase"] == "acquire"
        receipt["phase_complete"] = True
    except BaseException as error:
        receipt["error"] = str(error)
        raise
    finally:
        if process is not None:
            stop_owned_process(process)
        receipt["metadata_files"] = {str(path.relative_to(root)): digest(path) for path in metadata_paths(root)}
        receipt["free_bytes_after"] = {path: shutil.disk_usage(path).free for path in volumes}
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="operation", required=True)
    plan = sub.add_parser("plan", help="Read files and print a frozen local plan; does not run Java or Gradle")
    plan.add_argument("--phase", choices=("acquire", "verify"), default="verify")
    plan.add_argument("--java-home", required=True)
    plan.add_argument("--gradle-home", default=str(Path.home() / ".gradle"))
    plan.add_argument("--sdk", required=True)
    plan.add_argument("--gradle-receipt", help="Receipt from certify-gradle; required before execution")
    plan.add_argument("--concurrent-growth-bytes", type=int, default=0,
                      help="Explicit other concurrent disk reservations; added to this phase's reservation")
    certify = sub.add_parser("certify-gradle", help="Compare a pinned archive to installed files; no execution")
    certify.add_argument("--archive", type=Path, required=True)
    certify.add_argument("--installation", type=Path, required=True)
    certify.add_argument("--receipt", type=Path, required=True)
    run = sub.add_parser("run", help="Execute only after release-owner resource admission and plan review")
    run.add_argument("--execute", action="store_true", required=True)
    run.add_argument("--plan", required=True)
    run.add_argument("--plan-sha256", required=True)
    run.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.operation == "plan":
        print(json.dumps(make_plan(ROOT, args.java_home, args.gradle_home, args.sdk, args.phase, args.gradle_receipt, args.concurrent_growth_bytes), indent=2))
    elif args.operation == "run":
        execute_plan(args.plan, args.plan_sha256, args.output)
    else:
        certify_gradle(ROOT, args.archive, args.installation, args.receipt)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        sys.exit(str(error))
