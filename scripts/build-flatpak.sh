#!/usr/bin/env bash
# Build the Flatpak bundle: dist/refryt-<version>-x86_64.flatpak.
#
# Needs flatpak and flatpak-builder (org.flatpak.Builder from Flathub works), the GNOME 49
# Platform and Sdk, and org.freedesktop.Sdk.Extension.rust-stable//25.08. Needs the network.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
dir="packaging/flatpak"
id="io.github.gridoe.refryt"

version="$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "refryt"))')"
out="dist/refryt-$version-x86_64.flatpak"

if command -v flatpak-builder > /dev/null; then
    builder=(flatpak-builder)
else
    builder=(flatpak run --command=flatpak-builder org.flatpak.Builder)
fi

# Kept out of git: flathub's shared recipes (the tray library), and the newest yt-dlp, fetched
# fresh like the AppImage's — an old pinned copy would defeat the point of having it.
[ -d "$dir/shared-modules" ] \
    || git clone -q --depth 1 https://github.com/flathub/shared-modules.git "$dir/shared-modules"
echo "fetching yt-dlp"
curl -fsSL -o "$dir/yt-dlp" \
    https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux

mkdir -p dist
"${builder[@]}" --force-clean --user --disable-rofiles-fuse \
    --state-dir="$dir/.flatpak-builder" \
    --repo="$dir/repo" "$dir/build-dir" "$dir/$id.yml"
flatpak build-bundle "$dir/repo" "$out" "$id"
echo "built $out ($(du -h "$out" | cut -f1))"
