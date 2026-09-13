# Building refryt

## Prerequisites

- Rust 1.90+ (see `rust-version` in `Cargo.toml`)
- [`mpv`](https://mpv.io/) development libraries (`libmpv`) — refryt plays audio through it
- [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) — resolves a video id to a real audio stream.
  Not needed to build, only to actually play anything.
- [`just`](https://github.com/casey/just) (optional) — the `Justfile` has a command for
  everything below; without it, the plain `cargo`/`./scripts/...` equivalent is noted alongside.

## Everyday development

```sh
just              # cargo run, dev profile
just search "some song"
just release      # optimised run, for judging real performance
just check        # fmt --check + clippy (both feature sets) + tests — what CI runs
```

Without `just`, the equivalents are `cargo run -p refryt --bin refryt`, and the four commands
listed in `just check`'s own recipe in the `Justfile`.

## Linux: the AppImage

```sh
just appimage          # or: ./scripts/build-appimage.sh
just appimage-install  # installs it to ~/.local/bin with a desktop launcher entry
```

This bundles the release binary, every shared library short of glibc (`libmpv` included), a
freshly-fetched `yt-dlp`, and the app icon into a single `dist/refryt-<version>-x86_64.AppImage`.
Needs `python3` with Pillow and `curl`; no root required anywhere.

## Windows: cross-compiled from Linux

`./scripts/build-windows.sh` produces both a portable, no-install zip and a real installer, the
same "unzip and run, or install like anything else" choice most desktop apps offer. One-time
setup on the build machine:

```sh
sudo pacman -S mingw-w64-gcc mingw-w64-crt mingw-w64-headers mingw-w64-winpthreads mingw-w64-binutils
rustup target add x86_64-pc-windows-gnu
```

(If your system `cargo` doesn't come from `rustup` — e.g. Arch's `rust` package, which has no
cross-compile targets — install `rustup` separately rather than replacing it; the two can
coexist as long as `rustup`'s shims don't end up ahead of the system toolchain on `PATH`. The
build script finds `rustup`'s `cargo`/`rustc` explicitly either way, so it does not depend on
which one your shell resolves to by default.)

The installer needs [Inno Setup](https://jrsoftware.org/isinfo.php) too — there's no Linux
build of it, so it runs for real under Wine:

```sh
sudo pacman -S wine
wine <path-to-innosetup-6.x.exe> /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

(installs to `~/.wine/drive_c/Program Files (x86)/Inno Setup 6/ISCC.exe`, which is where the
build script looks for it). Skipping this step is fine — the script still builds the portable
zip on its own and just notes that it's skipping the installer.

```sh
./scripts/build-windows.sh
```

The script fetches the current [mpv Windows dev
package](https://sourceforge.net/projects/mpv-player-windows/files/libmpv/) (cached under
`.windows-tools/`, since — unlike yt-dlp's GitHub releases — there's no stable "latest" URL for
it) to link against, cross-builds with `mingw-w64-gcc`, fetches a fresh `yt-dlp.exe`, zips
`refryt.exe` + `libmpv-2.dll` + `yt-dlp.exe` into
`dist/refryt-<version>-windows-x86_64.zip`, and — when Wine + Inno Setup are set up — compiles
`scripts/windows-installer.iss` (via `ISCC.exe` under Wine) into
`dist/refryt-<version>-windows-x86_64-setup.exe`: Program Files, a Start Menu entry, an optional
desktop shortcut, and a real uninstaller.

Every other Windows dependency (`tray-icon`, `rfd`, window blur via DWM) resolves to DLLs that
ship with Windows itself — `libmpv-2.dll` is the only one that has to be bundled by hand.

The `.exe` carries its own icon (`assets/icons/app/refryt.ico`, generated from the same master
PNG the Linux icon set comes from) via `build.rs` + `embed-resource` — Windows-only, and gated
on the actual cross-compile *target*, not the host running the build, which is what lets it
stay a no-op on every other platform without a `cfg` that would just check the wrong thing.

The installer is unsigned — nobody's bought a code-signing certificate for this project — so
Windows' SmartScreen will warn on first run. That's expected, not a bug in the build.

## Packaged size

`just size` builds the release binary and prints its stripped size plus a section breakdown —
tracked so a dependency added later doesn't quietly undo the effort that went into keeping it
small.
