#!/usr/bin/env python3
"""Draws the app icon from vector shapes and writes packaging/icons/: app.icns, app.ico and
app-<size>.png. The artwork is original: a cube on a rounded square. Needs Pillow and, for the
.icns, macOS's iconutil. The outputs are committed, so only a change to the artwork reruns it.

    python3 scripts/make-icons.py
"""
import math
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "packaging" / "icons"
BASE = 2048  # drawn at this size, then scaled down for smooth edges


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def draw():
    img = Image.new("RGBA", (BASE, BASE), (0, 0, 0, 0))
    # Rounded square with a vertical gradient.
    margin = int(BASE * 0.06)
    top, bottom = (58, 130, 246), (23, 52, 120)
    grad = Image.new("RGBA", (BASE, BASE))
    gd = ImageDraw.Draw(grad)
    for y in range(BASE):
        gd.line([(0, y), (BASE, y)], fill=lerp(top, bottom, y / BASE) + (255,))
    mask = Image.new("L", (BASE, BASE), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [margin, margin, BASE - margin, BASE - margin], radius=int(BASE * 0.2), fill=255
    )
    img.paste(grad, (0, 0), mask)

    # An isometric cube: three faces of one hexagon.
    d = ImageDraw.Draw(img)
    cx, cy = BASE / 2, BASE / 2
    r = BASE * 0.30
    pts = [
        (cx + r * math.cos(math.radians(a)), cy + r * math.sin(math.radians(a)))
        for a in (-90, -30, 30, 90, 150, 210)
    ]
    center = (cx, cy)
    d.polygon([pts[0], pts[1], center, pts[5]], fill=(255, 255, 255, 255))  # top
    d.polygon([pts[5], center, pts[3], pts[4]], fill=(205, 222, 250, 255))  # left
    d.polygon([center, pts[1], pts[2], pts[3]], fill=(150, 183, 240, 255))  # right
    edge = (23, 52, 120, 255)
    w = int(BASE * 0.012)
    for a, b in [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 0)]:
        d.line([pts[a], pts[b]], fill=edge, width=w)
    for p in (pts[1], pts[3], pts[5]):
        d.line([center, p], fill=edge, width=w)
    return img


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    big = draw()
    sizes = [16, 32, 48, 64, 128, 256, 512, 1024]
    for s in sizes:
        big.resize((s, s), Image.LANCZOS).save(OUT / f"app-{s}.png")
    big.resize((256, 256), Image.LANCZOS).save(
        OUT / "app.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )
    if shutil.which("iconutil"):
        with tempfile.TemporaryDirectory() as tmp:
            iconset = Path(tmp) / "app.iconset"
            iconset.mkdir()
            for base in (16, 32, 128, 256, 512):
                big.resize((base, base), Image.LANCZOS).save(iconset / f"icon_{base}x{base}.png")
                big.resize((base * 2, base * 2), Image.LANCZOS).save(
                    iconset / f"icon_{base}x{base}@2x.png"
                )
            subprocess.run(
                ["iconutil", "-c", "icns", str(iconset), "-o", str(OUT / "app.icns")], check=True
            )
    else:
        print("iconutil not found: app.icns not written", file=sys.stderr)


main()
