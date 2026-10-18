#!/usr/bin/env python3
"""Build-independent UI tests; callers bind the separately qualified app at run time."""
import hashlib
import json
import pathlib
import subprocess
import sys
import tarfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "work/macos-ui-runner"
SPEC = """name: IrisChatMacUIRunner
options:
  deploymentTarget:
    macOS: '14.0'
settings:
  base:
    PRODUCT_BUNDLE_IDENTIFIER: fi.siriusbusiness.irischat
    SWIFT_VERSION: '5.0'
targets:
  IrisChatMacUITests:
    type: bundle.ui-testing
    platform: macOS
    sources:
      - path: ../../ios/UITests
        excludes:
          - Fixtures
      - path: ../../ios/UITests/Fixtures
        buildPhase: resources
        type: folder
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: fi.siriusbusiness.irischat.macos.uitests
        PRODUCT_NAME: '$(TARGET_NAME)'
        PRODUCT_MODULE_NAME: '$(TARGET_NAME)'
        GENERATE_INFOPLIST_FILE: YES
schemes:
  IrisChatMacUITests:
    build:
      targets:
        IrisChatMacUITests: all
    test:
      config: Debug
      targets:
        - IrisChatMacUITests
"""


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    if sys.argv[1:] == []:
        OUT.mkdir(parents=True, exist_ok=False)
        (OUT / "project.yml").write_text(SPEC)
        return
    if sys.argv[1:] != ["--receipt"]:
        raise SystemExit("usage: prepare_macos_ui_runner.py [--receipt]")
    products = OUT / "DerivedData/Build/Products"
    runner = products / "Debug/IrisChatMacUITests-Runner.app"
    if not runner.is_dir() or (products / "Debug/Iris Chat.app").exists():
        raise SystemExit("Expected only the UI runner, without an application build")
    archive = OUT / "macos-ui-runner.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(products, arcname="Products")
    proof = {
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "uiSourceTree": subprocess.check_output(["git", "rev-parse", "HEAD:ios/UITests"], cwd=ROOT, text=True).strip(),
        "platform": "macos", "architecture": "arm64", "configuration": "Debug",
        "signed": False, "testsExecuted": False, "releasePackage": False,
        "appBuilt": False, "rustBuilt": False,
        "archive": archive.name, "archiveBytes": archive.stat().st_size,
        "archiveSHA256": digest(archive), "projectSHA256": digest(OUT / "project.yml"),
        "xcode": subprocess.check_output(["xcodebuild", "-version"], text=True).strip(),
    }
    (OUT / "build-receipt.json").write_text(json.dumps(proof, indent=2) + "\n")


if __name__ == "__main__":
    main()
