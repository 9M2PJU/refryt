<p align="center">
  <img src="assets/icons/app/refryt-256.png" width="96" alt="refryt icon">
</p>

<h1 align="center">refryt</h1>

<p align="center">A fast, native YouTube Music client.</p>

> [!WARNING]
> refryt is an unofficial, experimental client, not affiliated with YouTube or Google in any
> way. "YouTube Music" is a trademark of Google LLC. It talks to YouTube's internal API, so it
> can break when YouTube changes something, and you use it at your own risk.

## What this is

YouTube Music's web app is fine, but it's a browser tab pretending to be an app: slow to open,
heavy at idle, and it comes with everything else a browser drags along. refryt is a real desktop
app instead. It's one binary, with no Electron, and it talks to YouTube Music the same way the
web app does. There is no official API for this, so the client is reverse-engineered and can
break when YouTube changes something. It usually doesn't take long to fix when that happens.

Sign-in imports the session from a browser you're already signed into. That happens locally, on
your own machine, and credentials never pass through anything but your browser and YouTube.

## What it does

- **Search that actually searches.** The mixed, ranked response YouTube returns can bury a real
  match behind a handful of vaguely related songs. Narrowing to Artists, Albums, Songs and the
  rest asks YouTube for that category specifically, instead of filtering the same short list.
- **A real library.** Create, rename, delete and reorder playlists, or point Settings at a folder
  of your own files and browse it like a file explorer, down to turning any folder into a local
  playlist. Liked songs, listening history and downloads for offline listening sit alongside.
- **Radio and autoplay**, ratings, moods and genres, and charts: the usual shape of YouTube
  Music, without the rest of the browser around it.
- **Music videos, lyrics and queue.** A track that has a music video can play it in the app, and
  a lyrics panel and an editable Up Next queue sit next to the usual transport.
- **Desktop integration.** A tray icon, start on boot, media keys and the OS media controls
  (MPRIS on Linux, SMTC on Windows), Discord Rich Presence, and the keyboard shortcuts you'd
  expect from Spotify's desktop app (space to play or pause, arrows to skip, and so on).
- **A translucent, blurred shell**, with real compositor blur where the desktop offers it (KDE
  Wayland today; Windows hides transparency and blur for now), and 22 colour themes that are
  [plain CSS files](docs/THEMES.md) in your config folder: edit one and the app repaints as
  you save, delete one and it's gone, add your own.
- **Sign-in from the app or from a browser.** The in-app sign-in window works on Linux and
  Windows, and importing the session from Chrome, Firefox, Brave, Vivaldi or Edge works
  everywhere. No pasting cookies, no device codes.

## Status

refryt is pre-1.0 and changes quickly. Expect rough edges, and grab the current build from
[Releases](https://github.com/gridoe/refryt/releases).

- **Linux is the tested platform.** The AppImage is what the project is developed and used on.
  The Windows installer and zip build from the same source, but have not had real testing on
  Windows hardware yet.
- **YouTube changes can break it.** Everything is reverse-engineered, and there is no advance
  notice when an endpoint or a token changes. A fix usually lands within a few days.
- **Unofficial clients fall outside YouTube's terms of service.** That is a risk taken
  knowingly, and no one here can promise your account is exempt from it.
- **Transparency and blur are Linux-only for now** (blur where the compositor offers it; Windows
  hides both). Podcasts and episodes are not supported at all.

## Screenshots

<p align="center">
  <a href="assets/screenshots/home-classic.png"><img src="assets/screenshots/home-classic.png" width="49%" alt="Home in the classic dark theme"></a>
  <a href="assets/screenshots/home-catppuccin-frappe-blur.png"><img src="assets/screenshots/home-catppuccin-frappe-blur.png" width="49%" alt="Home with the Catppuccin Frappe scheme at 75% opacity and compositor blur"></a>
</p>

The classic dark theme, and Catppuccin Frappe at 75% opacity with compositor blur on KDE
Wayland. Transparency and blur are Linux-only for now; Windows hides both.

## Tech stack

- **Rust**, one binary, with [`egui`/`eframe`](https://github.com/emilk/egui) for the UI
- **[`mpv`](https://mpv.io/)** (via `libmpv2`) for playback
- YouTube's own `player` endpoint for stream resolution, with **[`yt-dlp`](https://github.com/yt-dlp/yt-dlp)** as the fallback
- **[`ytmapi-rs`](https://github.com/nick42d/youtui)** for YouTube Music's InnerTube protocol
- **`tokio`** and **`reqwest`**/`rustls` for async networking
- **`glow`** (OpenGL) on Linux, **`wgpu`** (DX12) on Windows, for rendering
- **`lofty`**, **`walkdir`** and **`rfd`** for local files and folders

## Installing

**Linux.** Grab the AppImage from
[Releases](https://github.com/gridoe/refryt/releases), or build it yourself:

```sh
./scripts/build-appimage.sh
./scripts/appimage-install.sh install   # ~/.local/bin, with a desktop launcher entry
```

**Windows.** A real installer and a portable zip both exist
(`./scripts/build-windows.sh`); see [`docs/BUILDING.md`](docs/BUILDING.md) for the build setup.

## Building from source

See [`docs/BUILDING.md`](docs/BUILDING.md) for the full setup, the dev-loop commands (there's a
`Justfile` for all of it), and how the Windows cross-build works.

## License

[GPL-3.0-or-later](LICENSE).

## Inspiration

Ideas this project took from others, and where they show up:

- **[Sonora](https://github.com/sonorahq/sonora)** and its
  [`ytmusic-rs`](https://github.com/sonorahq/ytmusic-rs): playback asks YouTube's `player`
  endpoint as the visionOS client, which returns direct stream URLs, and when YouTube refuses the
  visitor it retries with the one the refusal issues.
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)**: the fallback resolver, and the reference for
  how YouTube gates anonymous clients.
- **[youtui](https://github.com/nick42d/youtui)**: `ytmapi-rs` for the InnerTube API, and its
  maintained `rusty_ytdl` fork behind the optional native resolver.
- **[ytmdesktop](https://github.com/ytmdesktop/ytmdesktop)**: what happens when the queue runs
  out.
- **Spotify's desktop app**: the keyboard shortcuts, single-click-to-select with double-click to
  play, and the shape of the album, playlist and artist pages.
- **Theme palettes**: [Catppuccin](https://github.com/catppuccin/palette),
  [Nord](https://www.nordtheme.com/docs/colors-and-palettes),
  [Gruvbox](https://github.com/morhetz/gruvbox),
  [Tokyo Night](https://github.com/folke/tokyonight.nvim),
  [Dracula](https://spec.draculatheme.com), [Rosé Pine](https://github.com/rose-pine/palette),
  [Everforest](https://github.com/sainnhe/everforest) and
  [Solarized](https://ethanschoonover.com/solarized). Each theme file in `crates/refryt/themes`
  names its source.
