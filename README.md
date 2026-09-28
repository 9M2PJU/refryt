<p align="center">
  <img src="assets/icons/app/refryt-256.png" width="96" alt="refryt icon">
</p>

<h1 align="center">refryt</h1>

<p align="center">A native YouTube Music client. One binary, no browser inside it.</p>

> [!WARNING]
> Unofficial, and not affiliated with YouTube or Google. "YouTube Music" is a trademark of
> Google LLC. It talks to YouTube's internal API, which can change on any given Tuesday, so
> things break now and then. Use it at your own risk.

<p align="center">
  <a href="assets/screenshots/home-classic.png"><img src="assets/screenshots/home-classic.png" width="49%" alt="Home in the classic dark theme"></a>
  <a href="assets/screenshots/home-catppuccin-frappe-blur.png"><img src="assets/screenshots/home-catppuccin-frappe-blur.png" width="49%" alt="Home with the Catppuccin Frappe scheme at 75% opacity and compositor blur"></a>
</p>

I listen to YouTube Music most of the day, and the web app is a browser tab dressed up as an
app. It's slow to open, heavy when it's doing nothing, and closing the wrong tab stops the
music. So this is a real desktop app instead. Rust, no Electron, and it talks to YouTube the
same way the website does.

With the home feed loaded and its covers on screen it sits at about 96 MB of its own memory,
and a frame costs well under a millisecond to draw. `scripts/perf.sh` measures both, and it
fails if either creeps up.

## What it does

The usual YouTube Music stuff: Home, Explore, search, your library, radio, charts, moods,
lyrics, a queue you can edit, and music videos when a song has one. A few things it does on
purpose:

- **Search actually searches.** YouTube's mixed results can bury the thing you typed under
  a pile of vaguely related songs. Picking Songs or Albums asks YouTube for that category
  directly, instead of filtering the same short list.
- **Your playlists behave.** Create, rename, reorder, and get told when a song is already in
  one instead of quietly adding it twice. Your own playlists get YouTube's Suggestions
  underneath, with a + to add them.
- **Local files too.** Point Settings at a folder and browse it like a file explorer. Any
  folder can become a playlist. Downloads for offline listening live next to it.
- **Offline doesn't fall over.** No connection gets you a banner and a way to your downloads.
  When it comes back, the page reloads by itself.
- **It fits on the desktop.** Tray icon, start on boot, media keys, the OS media controls
  (MPRIS on Linux, SMTC on Windows) and Discord Rich Presence.

## Keys

They're Spotify's desktop shortcuts, because my fingers already knew them.

| key | what it does |
|---|---|
| space | play / pause |
| up / down | previous / next |
| m | mute |
| ctrl + s | shuffle |
| ctrl + r | cycle repeat |
| ctrl + k or ctrl + l | jump to search |
| ? or F1 | the full list |

Songs select on one click and play on two, like Spotify. If you'd rather one click plays,
that's a toggle in Settings.

## Themes

22 of them, and each one is a [plain CSS file](docs/THEMES.md) in your config folder. Edit
one and the app repaints when you save. Delete one and it's gone. Drop in your own and it
shows up in the list.

The shell can be see-through with real compositor blur behind it. That only works where the
compositor offers blur, which today means KDE on Wayland. Windows hides both settings,
because what shows through there is a washed-out grey, not your desktop.

## Signing in

Either sign in from the app's own window (Linux and Windows), or let it borrow the session
from a browser you're already signed into: Chrome, Firefox, Brave, Vivaldi or Edge. It all
happens on your machine. There are no cookies to paste and no codes to type on another device.

## Status

Pre-1.0, and it moves quickly. Things worth knowing:

- **Linux is where it's used and tested.** The Windows installer and zip build from the
  same source but haven't spent much time on real Windows hardware.
- **YouTube will break it occasionally.** Nothing here is documented by YouTube, and they
  don't send notice when they change something. Fixes usually land within a few days.
- **Unofficial clients are outside YouTube's terms.** I use it anyway, but I can't promise
  your account is exempt from that.
- **No podcasts.** Episodes aren't supported at all.

## Install

**Linux.** Grab the AppImage from [releases](https://github.com/gridoe/refryt/releases),
or build it:

```sh
./scripts/build-appimage.sh
./scripts/appimage-install.sh install   # into ~/.local/bin, with a launcher entry
```

**Windows.** There's an installer and a portable zip on the same releases page. Both come
out of `./scripts/build-windows.sh`, which cross-builds from Linux.

## Build

It's a Cargo workspace with a `Justfile` on top. `just` builds and runs it,
`just release` does the optimised build, and `just check` runs formatting, clippy and the
tests.

You'll need Rust and libmpv. yt-dlp is optional but worth having, because it's the fallback
when YouTube won't hand a stream over directly. [docs/BUILDING.md](docs/BUILDING.md) has the
full setup and the Windows cross-build.

What's inside:

- **Rust** with [`egui`/`eframe`](https://github.com/emilk/egui) for the UI
- **[`mpv`](https://mpv.io/)** through `libmpv2` for playback
- YouTube's own `player` endpoint for streams, with
  **[`yt-dlp`](https://github.com/yt-dlp/yt-dlp)** behind it when that says no
- **[`ytmapi-rs`](https://github.com/nick42d/youtui)** for YouTube Music's InnerTube API
- **`tokio`** and **`reqwest`**/`rustls` for networking
- **`glow`** (OpenGL) on Linux and **`wgpu`** (DX12) on Windows for drawing
- **`lofty`**, **`walkdir`** and **`rfd`** for local files

## Where ideas came from

- **[Sonora](https://github.com/sonorahq/sonora)** and its
  [`ytmusic-rs`](https://github.com/sonorahq/ytmusic-rs): asking YouTube's `player` endpoint
  as the visionOS client, which hands back direct stream URLs, and retrying with the visitor
  id a refusal issues.
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)**: the fallback resolver, and the reference
  for how YouTube gates anonymous clients.
- **[youtui](https://github.com/nick42d/youtui)**: `ytmapi-rs` for the InnerTube API, and
  its maintained `rusty_ytdl` fork behind the optional native resolver.
- **[ytmdesktop](https://github.com/ytmdesktop/ytmdesktop)**: what happens when the queue
  runs out.
- **Spotify's desktop app**: the shortcuts, select-then-play clicks, and the shape of the
  album, playlist and artist pages.
- **Theme palettes**: [Catppuccin](https://github.com/catppuccin/palette),
  [Nord](https://www.nordtheme.com/docs/colors-and-palettes),
  [Gruvbox](https://github.com/morhetz/gruvbox),
  [Tokyo Night](https://github.com/folke/tokyonight.nvim),
  [Dracula](https://spec.draculatheme.com), [Rosé Pine](https://github.com/rose-pine/palette),
  [Everforest](https://github.com/sainnhe/everforest) and
  [Solarized](https://ethanschoonover.com/solarized). Each file in `crates/refryt/themes`
  names its source.

## Licence

[GPL-3.0-or-later](LICENSE).
