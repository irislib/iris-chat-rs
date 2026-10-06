#!/usr/bin/env python3
"""Benchmark the production desktop video module without building the whole app.

Run with --build-only before reserving a quiet machine for timings. --check runs
the conversion parity and existing video encoder/decoder tests instead. The
default compares allocation+conversion and conversion+encoding in release mode.
All generated sources, dependencies and build outputs stay in core/target/.
"""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess


def rust_path(path: Path) -> str:
    return '"' + str(path).replace("\\", "\\\\").replace('"', '\\"') + '"'


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--build-only", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    workspace = root / "core/target/video-conversion-bench"
    workspace.mkdir(parents=True, exist_ok=True)
    video = root / "core/src/desktop_call/video.rs"
    converter = video.with_name("video_rgb.rs")
    source = video.read_text()
    call = "super::video_rgb::convert(rgb, width as usize, height as usize)"
    if source.count(call) != 1:
        raise RuntimeError("Production conversion call changed; update the benchmark baseline")
    (workspace / "baseline_video.rs").write_text(
        source.replace(call, call.replace("video_rgb", "baseline_rgb"))
    )
    dependencies = (root / "core/Cargo.toml").read_text()
    manifest = '[package]\nname = "iris-video-conversion-bench"\nversion = "0.0.0"\nedition = "2021"\n'
    manifest += '[workspace]\n[lib]\npath = "lib.rs"\n[dependencies]\n'
    for name in ["openh264", "openh264-sys2"]:
        version = re.search(
            r'(?m)^' + re.escape(name) + r'\s*=\s*\{\s*version\s*=\s*"(=[0-9.]+)"',
            dependencies,
        )
        if version is None:
            raise RuntimeError(f"Expected an exact {name} version in core/Cargo.toml")
        manifest += f'{name} = "{version.group(1)}"\n'
    (workspace / "Cargo.toml").write_text(manifest)
    # Preserve the app's resolved transitive versions; Cargo only needs to add
    # this tiny harness package and discard unrelated dependencies from the copy.
    shutil.copyfile(root / "core/Cargo.lock", workspace / "Cargo.lock")
    (workspace / "lib.rs").write_text(f'''#![allow(dead_code)]
enum DesktopCallEvent {{
    Video {{ local: bool, width: u32, height: u32, rgba: Vec<u8> }},
    RequestKeyFrame,
}}
#[path = {rust_path(video)}] mod video;
#[path = {rust_path(converter)}] mod video_rgb;
mod baseline_video;
mod baseline_rgb {{
    pub(super) fn convert(rgb: &[u8], width: usize, height: usize) -> openh264::formats::YUVBuffer {{
        openh264::formats::YUVBuffer::from_rgb8_source(
            openh264::formats::RgbSliceU8::new(rgb, (width, height)))
    }}
}}
#[cfg(test)]
#[path = {rust_path(root / "scripts/video_conversion_bench.rs")}] mod bench;
''')
    command = ["cargo", "test", "--manifest-path", str(workspace / "Cargo.toml"), "--release"]
    if args.build_only:
        command += ["--no-run"]
    elif args.check:
        command += ["--", "--test-threads=1"]
    else:
        command += ["bench::timings", "--", "--ignored", "--nocapture", "--test-threads=1"]
    environment = dict(os.environ, CARGO_TARGET_DIR=str(workspace / "target"))
    environment.setdefault("CARGO_BUILD_JOBS", "4")
    subprocess.run(command, cwd=root, env=environment, check=True)


if __name__ == "__main__":
    main()
