<p align="center">
  <img src="assets/icons/app/refryt-256.png" width="96" alt="refryt icon">
</p>

<h1 align="center">refryt</h1>

<p align="center">A fast, native YouTube Music client — no Electron, no browser tab, no ads.</p>

## What this is

YouTube Music's web app is fine, but it's a browser tab pretending to be an app: slow to open,
heavy at idle, and it comes with everything else a browser drags along. refryt is a real desktop
app instead — one binary, no Electron. It talks to YouTube Music the same way the web app does —
there's no official API for this, so it's reverse-engineered and can break when YouTube changes
something. It usually doesn't take long to fix when it does.

This is **not affiliated with YouTube or Google** in any way. Sign-in works by importing your
existing browser session's cookies, locally, on your own machine — no credentials ever pass
through anything but your own browser and YouTube itself.

## What it does

- **Search that actually searches** — the mixed, ranked response YouTube hands back can bury a
  real match behind a handful of vaguely-related songs; narrowing to Artists/Albums/Songs/etc.
  asks YouTube for that category specifically instead of just filtering the same short list.
- **Playlists** you can make, rename, delete, and reorder by dragging — and **local playlists**,
  too: point Settings at a folder of your own files and browse them like a file explorer, right
  down to turning any folder into a real playlist without ever leaving the app.
- **Radio/autoplay**, ratings, your library, listening history, moods & genres, charts — the
  usual shape of YouTube Music, without the rest of the browser around it.
- **A translucent, blurred shell** with real desktop compositor blur where it's available, nine
  colour schemes, and light/dark variants for each.
- **A system tray and autostart** on Linux, and the keyboard shortcuts you'd expect from
  Spotify's own desktop app (space to play/pause, arrows to skip, and so on).
- **Sign-in that's just importing a browser session** — Chrome, Firefox, Brave, Vivaldi, Edge,
  whatever you're already signed into. No pasting cookies, no device codes.

## Tech stack

- **Rust**, one binary — [`egui`/`eframe`](https://github.com/emilk/egui) for the UI
- **[`mpv`](https://mpv.io/)** (via `libmpv2`) for playback
- **[`yt-dlp`](https://github.com/yt-dlp/yt-dlp)** for stream resolution
- **[`ytmapi-rs`](https://github.com/nick42d/youtui)** for YouTube Music's InnerTube protocol
- **`tokio`** + **`reqwest`**/`rustls` for async networking
- **`glow`** (OpenGL) on Linux, **`wgpu`** (DirectComposition) on Windows, for rendering
- **`lofty`** + **`walkdir`** + **`rfd`** for local files and folders

## Installing

**Linux** — grab the AppImage from
[Releases](https://github.com/gridoe/refryt/releases), or build it yourself:

```sh
./scripts/build-appimage.sh
./scripts/appimage-install.sh install   # ~/.local/bin, with a desktop launcher entry
```

**Windows** — a real installer and a portable zip both exist
(`./scripts/build-windows.sh`), but neither has had real testing on actual Windows yet. See
[`docs/BUILDING.md`](docs/BUILDING.md) if you want to try one or help shake it out.

## Building from source

See [`docs/BUILDING.md`](docs/BUILDING.md) for the full setup, the dev-loop commands (there's a
`Justfile` for all of it), and how the Windows cross-build works.

## License

[GPL-3.0-or-later](LICENSE).
