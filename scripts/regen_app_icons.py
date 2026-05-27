#!/usr/bin/env python3
"""Regenerate Iris Chat app icons and logo assets.

The icon is a native-style sweep-gradient iris ring with a small lower-left
dent in the center hole. The script writes a vector copy to assets/ and uses
that same geometry to generate platform PNG/ICO/ICNS assets.
"""
from __future__ import annotations

import math
import shutil
import subprocess
import sys
import tempfile
from functools import lru_cache
from pathlib import Path
from PIL import Image

REPO = Path(__file__).resolve().parent.parent
SVG_SOURCE = REPO / "assets/iris-chat-logo.svg"
BG = (0, 0, 0, 255)
LOGO_FRACTION = 0.75
ANDROID_FOREGROUND_FRACTION = 0.68
ANDROID_SPLASH_FRACTION = ANDROID_FOREGROUND_FRACTION

VIEWBOX = 512
CENTER = VIEWBOX / 2
OUTER_RADIUS = 253
INNER_RADIUS = 107
DENT_DIRECTION_DEGREES = 135
DENT_HALF_ANGLE_DEGREES = 15
DENT_TIP_RADIUS = INNER_RADIUS + (OUTER_RADIUS - INNER_RADIUS) / 2
SWEEP_SEGMENTS = 720
SWEEP_SEGMENT_OVERLAP_DEGREES = 0.2
SWEEP_COLORS = [
    "#312E81",
    "#4F46E5",
    "#7C3AED",
    "#A855F7",
    "#D946EF",
    "#EC4899",
    "#BE185D",
    "#7C3AED",
    "#312E81",
]

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

ANDROID_SPLASH_OUTPUTS = {
    "drawable-mdpi/ic_splash_icon.png": 108,
    "drawable-hdpi/ic_splash_icon.png": 162,
    "drawable-xhdpi/ic_splash_icon.png": 216,
    "drawable-xxhdpi/ic_splash_icon.png": 324,
    "drawable-xxxhdpi/ic_splash_icon.png": 432,
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


def hex_to_rgb(color: str) -> tuple[int, int, int]:
    color = color.removeprefix("#")
    return int(color[0:2], 16), int(color[2:4], 16), int(color[4:6], 16)


def interpolate_color(angle_degrees: float) -> str:
    angle = angle_degrees % 360
    position = angle / 360 * (len(SWEEP_COLORS) - 1)
    index = math.floor(position)
    fraction = position - index
    c1 = hex_to_rgb(SWEEP_COLORS[index])
    c2 = hex_to_rgb(SWEEP_COLORS[min(index + 1, len(SWEEP_COLORS) - 1)])
    channels = [
        round(c1[channel] + (c2[channel] - c1[channel]) * fraction)
        for channel in range(3)
    ]
    return f"#{channels[0]:02X}{channels[1]:02X}{channels[2]:02X}"


def point(angle_degrees: float, radius: float, scale: float = 1.0) -> tuple[float, float]:
    radians = math.radians(angle_degrees)
    return (
        (CENTER + math.cos(radians) * radius) * scale,
        (CENTER + math.sin(radians) * radius) * scale,
    )


def dent_points(scale: float = 1.0) -> list[tuple[float, float]]:
    return [
        point(DENT_DIRECTION_DEGREES - DENT_HALF_ANGLE_DEGREES, INNER_RADIUS, scale),
        point(DENT_DIRECTION_DEGREES, DENT_TIP_RADIUS, scale),
        point(DENT_DIRECTION_DEGREES + DENT_HALF_ANGLE_DEGREES, INNER_RADIUS, scale),
    ]


def svg_sector_path(start_degrees: float, end_degrees: float) -> str:
    radius = OUTER_RADIUS + 8
    x0, y0 = point(start_degrees, radius)
    x1, y1 = point(end_degrees, radius)
    return (
        f"M {CENTER:.3f} {CENTER:.3f} "
        f"L {x0:.3f} {y0:.3f} "
        f"A {radius:.3f} {radius:.3f} 0 0 1 {x1:.3f} {y1:.3f} Z"
    )


def svg_source() -> str:
    sectors = []
    for index in range(SWEEP_SEGMENTS):
        start = index * 360 / SWEEP_SEGMENTS
        end = (index + 1) * 360 / SWEEP_SEGMENTS
        color = interpolate_color((start + end) / 2)
        path = svg_sector_path(
            start - SWEEP_SEGMENT_OVERLAP_DEGREES,
            end + SWEEP_SEGMENT_OVERLAP_DEGREES,
        )
        sectors.append(f'    <path d="{path}" fill="{color}"/>')

    dent = " ".join(f"{x:.3f},{y:.3f}" for x, y in dent_points())
    sector_markup = "\n".join(sectors)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {VIEWBOX} {VIEWBOX}">
  <!-- Generated by scripts/regen_app_icons.py. -->
  <defs>
    <mask id="iris-ring-mask" maskUnits="userSpaceOnUse" x="0" y="0" width="{VIEWBOX}" height="{VIEWBOX}">
      <rect width="{VIEWBOX}" height="{VIEWBOX}" fill="#000000"/>
      <circle cx="{CENTER:.3f}" cy="{CENTER:.3f}" r="{OUTER_RADIUS:.3f}" fill="#ffffff"/>
      <circle cx="{CENTER:.3f}" cy="{CENTER:.3f}" r="{INNER_RADIUS:.3f}" fill="#000000"/>
      <polygon points="{dent}" fill="#000000"/>
    </mask>
  </defs>
  <g mask="url(#iris-ring-mask)">
{sector_markup}
  </g>
</svg>
"""


def write_svg_source() -> None:
    SVG_SOURCE.parent.mkdir(parents=True, exist_ok=True)
    SVG_SOURCE.write_text(svg_source(), encoding="utf-8")
    print(f"SVG source: {SVG_SOURCE.relative_to(REPO)}")


def rasterize_logo(size: int) -> Image.Image:
    resvg = shutil.which("resvg")
    if not resvg:
        print("resvg not found. Install it with: cargo install resvg", file=sys.stderr)
        sys.exit(1)

    with tempfile.NamedTemporaryFile(suffix=".png") as tmp:
        subprocess.run(
            [
                resvg,
                "--width",
                str(size),
                "--height",
                str(size),
                str(SVG_SOURCE),
                tmp.name,
            ],
            check=True,
        )
        return Image.open(tmp.name).convert("RGBA")


@lru_cache(maxsize=None)
def render_logo(size: int) -> Image.Image:
    return rasterize_logo(size)


def render_icon(size: int, rgb: bool = False) -> Image.Image:
    icon = Image.new("RGBA", (size, size), BG)
    logo_size = round(size * LOGO_FRACTION)
    resized = render_logo(logo_size).copy()
    x = (size - resized.width) // 2
    y = (size - resized.height) // 2
    icon.alpha_composite(resized, (x, y))
    return icon.convert("RGB") if rgb else icon


def render_foreground(size: int) -> Image.Image:
    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    logo_size = round(size * ANDROID_FOREGROUND_FRACTION)
    resized = render_logo(logo_size).copy()
    x = (size - resized.width) // 2
    y = (size - resized.height) // 2
    icon.alpha_composite(resized, (x, y))
    return icon


def render_splash(size: int) -> Image.Image:
    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    logo_size = round(size * ANDROID_SPLASH_FRACTION)
    resized = render_logo(logo_size).copy()
    x = (size - resized.width) // 2
    y = (size - resized.height) // 2
    icon.alpha_composite(resized, (x, y))
    return icon


def render_transparent_logo(size: int) -> Image.Image:
    return render_logo(size).copy()


def write_outputs(out_dir: Path, outputs: dict[str, int], rgb: bool) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    for filename, size in outputs.items():
        img = render_icon(size, rgb=rgb)
        path = out_dir / filename
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_android() -> None:
    for filename, size in ANDROID_LEGACY_OUTPUTS.items():
        path = ANDROID_RES_DIR / filename
        img = render_icon(size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")

    for filename, size in ANDROID_FOREGROUND_OUTPUTS.items():
        path = ANDROID_RES_DIR / filename
        img = render_foreground(size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")

    for filename, size in ANDROID_SPLASH_OUTPUTS.items():
        path = ANDROID_RES_DIR / filename
        img = render_splash(size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_linux() -> None:
    for filename, size in LINUX_OUTPUTS.items():
        path = LINUX_RES_DIR / filename
        img = render_icon(size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_transparent_logos() -> None:
    for filename, size in TRANSPARENT_LOGO_OUTPUTS.items():
        path = REPO / filename
        img = render_transparent_logo(size)
        img.save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(REPO)} ({size}x{size})")


def write_icns() -> None:
    """Build a macOS .icns bundle alongside the appiconset PNGs."""
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
            img = render_icon(size, rgb=True)
            img.save(iconset_dir / name, "PNG", optimize=True)

        out = MAC_DIR / "app_icon.icns"
        subprocess.run(
            ["iconutil", "-c", "icns", "-o", str(out), str(iconset_dir)],
            check=True,
        )
    print(f"  {out.relative_to(REPO)}")


def write_windows_ico() -> None:
    frames = [render_icon(size[0]) for size in WINDOWS_ICO_SIZES]
    out = WINDOWS_RES_DIR / "IrisChat.ico"
    frames[-1].save(
        out,
        format="ICO",
        sizes=WINDOWS_ICO_SIZES,
        append_images=frames[:-1],
    )
    print(f"  {out.relative_to(REPO)}")


def main() -> None:
    write_svg_source()
    print("iOS:")
    write_outputs(IOS_DIR, IOS_OUTPUTS, rgb=True)
    print("macOS:")
    write_outputs(MAC_DIR, MAC_OUTPUTS, rgb=True)
    write_icns()
    print("Android:")
    write_android()
    print("Linux:")
    write_linux()
    print("Windows:")
    write_windows_ico()
    print("transparent logos:")
    write_transparent_logos()


if __name__ == "__main__":
    main()
