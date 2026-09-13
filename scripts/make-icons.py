#!/usr/bin/env python3
"""Turn a raw square logo into the app's icon set.

A launcher icon wants its corners transparent. This intersects the artwork's own alpha with a
rounded-rectangle mask at the standard 22.37% app-icon radius (what the first `refryt-master.png`
used: 280/1254) — a raw square with opaque corners gets them clipped, while art that is already
a transparent, inset tile passes through untouched — then emits:

    assets/icons/app/refryt-master.png      the RGBA master both AppImage scripts resize from
    assets/icons/app/refryt-{16..512}.png   the pre-sized hicolor set `just install` copies

Run after changing the source logo:  python3 scripts/make-icons.py [newlogo.png]
"""

import os
import sys

from PIL import Image, ImageChops, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets/icons/app")
RADIUS_FRAC = 0.2237  # iOS-style app-icon corner radius; matches the original master
SIZES = (16, 32, 48, 64, 128, 256, 512)
SS = 4  # mask supersampling, so 512px corners are not jagged


def rounded_mask(size: int) -> Image.Image:
    big = size * SS
    m = Image.new("L", (big, big), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=int(big * RADIUS_FRAC), fill=255
    )
    return m.resize((size, size), Image.LANCZOS)


def _masked(src: Image.Image, size: int) -> Image.Image:
    """Resize to `size` and clip the alpha to the rounded mask.

    The clip is an intersection (`darker`), not a replacement: a source that already carries
    transparency — like the current artwork, whose tile is inset with a transparent margin —
    must keep it. Replacing the alpha would make that transparent margin opaque, which shows
    up as a black frame around the tile.
    """
    out = src.resize((size, size), Image.LANCZOS)
    out.putalpha(ImageChops.darker(out.getchannel("A"), rounded_mask(size)))
    return out


def main() -> None:
    src_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "newlogo.png")
    src = Image.open(src_path).convert("RGBA")
    if src.width != src.height:
        raise SystemExit(f"{src_path}: expected a square image, got {src.size}")

    os.makedirs(OUT, exist_ok=True)
    master = _masked(src, src.width)
    master.save(os.path.join(OUT, "refryt-master.png"))

    for s in SIZES:
        _masked(src, s).save(os.path.join(OUT, f"refryt-{s}.png"))

    print(f"wrote refryt-master.png ({src.width}px) and refryt-{{{','.join(map(str, SIZES))}}}.png from {os.path.relpath(src_path, ROOT)}")


if __name__ == "__main__":
    main()
