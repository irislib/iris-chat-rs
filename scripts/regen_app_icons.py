#!/usr/bin/env python3
"""Regenerate Iris app icons and logo assets.

The PWA icon in ~/src/iris-chat is the visual source of truth: a transparent
Iris mark centered on a black background at 75% of the icon canvas. App icons
use that same composition. Welcome/logo assets stay transparent.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path
from PIL import Image

REPO = Path(__file__).resolve().parent.parent
SRC = Path(os.environ.get(
    "IRIS_LOGO_SRC",
    Path.home() / "src/iris-chat/public/iris-logo.png",
)).expanduser()
BG = (0, 0, 0, 255)
LOGO_FRACTION = 0.75

IOS_DIR = REPO / "ios/Assets.xcassets/AppIcon.appiconset"
MAC_DIR = REPO / "macos/Assets.xcassets/AppIcon.appiconset"
ANDROID_RES_DIR = REPO / "android/app/src/main/res"
LINUX_RES_DIR = REPO / "linux/resources"
WINDOWS_RES_DIR = REPO / "windows/IrisChat/Resources"

IOS_OUTPUTS = {
    "Icon-App-20x20@1x.png": 20,
    "Icon-App-20x20@2x.png": 40,
    "Icon-App-20x20@3x.png": 60,
    "Icon-App-29x29@1x.png": 29,
    "Icon-App-29x29@2x.png": 58,
    "Icon-App-29x29@3x.png": 87,
    "Icon-App-40x40@1x.png": 40,
    "Icon-App-40x40@2x.png": 80,
    "Icon-App-40x40@3x.png": 120,
    "Icon-App-50x50@1x.png": 50,
    "Icon-App-50x50@2x.png": 100,
    "Icon-App-57x57@1x.png": 57,
    "Icon-App-57x57@2x.png": 114,
    "Icon-App-60x60@2x.png": 120,
    "Icon-App-60x60@3x.png": 180,
    "Icon-App-72x72@1x.png": 72,
    "Icon-App-72x72@2x.png": 144,
    "Icon-App-76x76@1x.png": 76,
    "Icon-App-76x76@2x.png": 152,
    "Icon-App-83.5x83.5@2x.png": 167,
    "Icon-App-1024x1024@1x.png": 1024,
}

MAC_OUTPUTS = {
    "app_icon_16.png": 16,
    "app_icon_32.png": 32,
    "app_icon_64.png": 64,
    "app_icon_128.png": 128,
    "app_icon_256.png": 256,
    "app_icon_512.png": 512,
    "app_icon_1024.png": 1024,
}

ANDROID_LEGACY_OUTPUTS = {
    "mipmap-mdpi/ic_launcher.png": 48,
    "mipmap-hdpi/ic_launcher.png": 72,
    "mipmap-xhdpi/ic_launcher.png": 96,
    "mipmap-xxhdpi/ic_launcher.png": 144,
    "mipmap-xxxhdpi/ic_launcher.png": 192,
}

ANDROID_FOREGROUND_OUTPUTS = {
    "drawable-mdpi/ic_launcher_foreground.png": 108,
    "drawable-hdpi/ic_launcher_foreground.png": 162,
    "drawable-xhdpi/ic_launcher_foreground.png": 216,
    "drawable-xxhdpi/ic_launcher_foreground.png": 324,
    "drawable-xxxhdpi/ic_launcher_foreground.png": 432,
}

LINUX_OUTPUTS = {
    "iris-chat-16.png": 16,
    "iris-chat-22.png": 22,
    "iris-chat-24.png": 24,
    "iris-chat-32.png": 32,
    "iris-chat-48.png": 48,
    "iris-chat-64.png": 64,
    "iris-chat-128.png": 128,
    "iris-chat-256.png": 256,
    "iris-chat-512.png": 512,
}

TRANSPARENT_LOGO_OUTPUTS = {
    "ios/Assets.xcassets/IrisLogo.imageset/iris-logo.png": 512,
    "macos/Assets.xcassets/IrisLogo.imageset/iris-logo.png": 512,
    "android/app/src/main/res/drawable-nodpi/iris_logo.png": 512,
    "linux/resources/iris-chat-logo.png": 512,
    "windows/IrisChat/Resources/IrisLogo.png": 512,
}

WINDOWS_ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def load_logo() -> Image.Image:
    return Image.open(SRC).convert("RGBA")


def resize_logo(logo: Image.Image, max_size: int) -> Image.Image:
    scale = min(max_size / logo.width, max_size / logo.height)
    size = (round(logo.width * scale), round(logo.height * scale))
    return logo.resize(size, Image.Resampling.LANCZOS)


def render_icon(logo: Image.Image, size: int, rgb: bool = False) -> Image.Image:
    icon = Image.new("RGBA", (size, size), BG)
    logo_size = round(size * LOGO_FRACTION)
    resized = resize_logo(logo, logo_size)
    x = (size - resized.width) // 2
    y = (size - resized.height) // 2
    icon.alpha_composite(resized, (x, y))
    return icon.convert("RGB") if rgb else icon


def render_foreground(logo: Image.Image, size: int) -> Image.Image:
    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    logo_size = round(size * LOGO_FRACTION)
    resized = resize_logo(logo, logo_size)
    x = (size - resized.width) // 2
    y = (size - resized.height) // 2
    icon.alpha_composite(resized, (x, y))
    return icon


def render_transparent_logo(logo: Image.Image, size: int) -> Image.Image:
    return resize_logo(logo, size)


def write_outputs(out_dir: Path, outputs: dict[str, int], logo: Image.Image, rgb: bool) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    for filename, size in outputs.items():
        img = render_icon(logo, size, rgb=rgb)
        path = out_dir / filename
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_android(logo: Image.Image) -> None:
    for filename, size in ANDROID_LEGACY_OUTPUTS.items():
        path = ANDROID_RES_DIR / filename
        img = render_icon(logo, size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")

    for filename, size in ANDROID_FOREGROUND_OUTPUTS.items():
        path = ANDROID_RES_DIR / filename
        img = render_foreground(logo, size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_linux(logo: Image.Image) -> None:
    for filename, size in LINUX_OUTPUTS.items():
        path = LINUX_RES_DIR / filename
        img = render_icon(logo, size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_transparent_logos(logo: Image.Image) -> None:
    for filename, size in TRANSPARENT_LOGO_OUTPUTS.items():
        path = REPO / filename
        img = render_transparent_logo(logo, size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_icns(logo: Image.Image) -> None:
    """Build a macOS .icns bundle alongside the appiconset PNGs."""
    import subprocess
    import tempfile

    pairs = [
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ]

    with tempfile.TemporaryDirectory(dir=MAC_DIR) as tmp:
        iconset_dir = Path(tmp) / "app_icon.iconset"
        iconset_dir.mkdir()
        for size, name in pairs:
            img = render_icon(logo, size, rgb=True)
            img.save(iconset_dir / name, "PNG", optimize=True)

        out = MAC_DIR / "app_icon.icns"
        subprocess.run(
            ["iconutil", "-c", "icns", "-o", str(out), str(iconset_dir)],
            check=True,
        )
    print(f"  {out.relative_to(REPO)}")


def write_windows_ico(logo: Image.Image) -> None:
    frames = [render_icon(logo, size[0]) for size in WINDOWS_ICO_SIZES]
    out = WINDOWS_RES_DIR / "IrisChat.ico"
    frames[-1].save(
        out,
        format="ICO",
        sizes=WINDOWS_ICO_SIZES,
        append_images=frames[:-1],
    )
    print(f"  {out.relative_to(REPO)}")


def main() -> None:
    if not SRC.exists():
        print(f"source not found: {SRC}", file=sys.stderr)
        sys.exit(1)
    logo = load_logo()
    print(f"source: {SRC}")
    print("iOS:")
    write_outputs(IOS_DIR, IOS_OUTPUTS, logo, rgb=True)
    print("macOS:")
    write_outputs(MAC_DIR, MAC_OUTPUTS, logo, rgb=True)
    write_icns(logo)
    print("Android:")
    write_android(logo)
    print("Linux:")
    write_linux(logo)
    print("Windows:")
    write_windows_ico(logo)
    print("transparent logos:")
    write_transparent_logos(logo)


if __name__ == "__main__":
    main()
