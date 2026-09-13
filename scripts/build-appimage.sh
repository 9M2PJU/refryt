#!/usr/bin/env bash
# Build dist/refryt-<version>-x86_64.AppImage: the release binary, the logo as the icon, and
# every shared library short of glibc — libmpv and its whole tail — bundled inside, so the
# image runs on any machine with the same architecture and a close-enough glibc.
#
# Needs python3 with PIL (for the icon sizes) and curl (for the two tool AppImages, cached
# under .appimage-tools). No root anywhere; both tools run --appimage-extract-and-run.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version="$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "refryt"))')"
out="dist/refryt-$version-x86_64.AppImage"
tools=".appimage-tools"

fetch() { # fetch <name> <url>
    local file="$tools/$1"
    if [ ! -x "$file" ]; then
        mkdir -p "$tools"
        echo "fetching $1"
        curl -fsSL -o "$file" "$2"
        chmod +x "$file"
    fi
}

fetch linuxdeploy \
    https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch appimagetool \
    https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage

echo "building release binary"
cargo build --release -p refryt --bin refryt

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
appdir="$work/AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications"

echo "sizing the logo into the icon set"
python3 - "$root" "$appdir" << 'EOF'
import os, sys
from PIL import Image
root, appdir = sys.argv[1], sys.argv[2]
# The preprocessed master, not `logov1.png`: the raw source has no alpha and an opaque black
# margin outside the rounded square, which is exactly the black corner this once shipped.
src = Image.open(os.path.join(root, "assets/icons/app/refryt-master.png")).convert("RGBA")
base = os.path.join(appdir, "usr/share/icons/hicolor")
for s in (16, 32, 48, 64, 128, 256, 512):
    d = os.path.join(base, f"{s}x{s}/apps")
    os.makedirs(d, exist_ok=True)
    src.resize((s, s), Image.LANCZOS).save(os.path.join(d, "refryt.png"))
src.resize((256, 256), Image.LANCZOS).save(os.path.join(appdir, "refryt.png"))
EOF

cp target/release/refryt "$appdir/usr/bin/"
cp assets/refryt.desktop "$appdir/usr/share/applications/"

# yt-dlp is the resolver that exists specifically to survive YouTube changing something — it is
# not optional, playback does not work at all without it — so it ships inside the image rather
# than assuming the target machine already has it. Always freshly fetched, unlike `fetch()`'s
# cached tools above: an old pinned copy would defeat the whole point of using it. The standalone
# build is self-contained (no system Python needed).
echo "fetching yt-dlp"
curl -fsSL -o "$appdir/usr/bin/yt-dlp" \
    https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux
chmod +x "$appdir/usr/bin/yt-dlp"

echo "bundling libraries"
"$tools/linuxdeploy" --appimage-extract-and-run \
    --appdir "$appdir" \
    -e "$appdir/usr/bin/refryt" \
    -d "$appdir/usr/share/applications/refryt.desktop" \
    -i "$appdir/refryt.png" > /dev/null

mkdir -p dist
echo "packing $out"
ARCH=x86_64 "$tools/appimagetool" --appimage-extract-and-run \
    "$appdir" "$root/$out" > /dev/null
echo "built $out ($(du -h "$out" | cut -f1))"
