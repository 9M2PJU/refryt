<p align="center">
  <img src="assets/icons/app/refryt-256.png" width="96" alt="refryt icon">
</p>

<h1 align="center">refryt</h1>

<p align="center">A native YouTube Music client. One binary, no browser inside it.</p>

<p align="center">
  <a href="assets/screenshots/home-classic.png"><img src="assets/screenshots/home-classic.png" width="49%" alt="Home in the classic dark theme"></a>
  <a href="assets/screenshots/home-catppuccin-frappe-blur.png"><img src="assets/screenshots/home-catppuccin-frappe-blur.png" width="49%" alt="Home with the Catppuccin Frappe scheme at 75% opacity and compositor blur"></a>
</p>

I have YouTube Music on most of the day, and the web app is a browser tab dressed up as an
app. It takes a while to open, it's heavy while it does nothing, and closing the wrong tab
stops the music. So this is a real desktop app instead. It's written in Rust, there's no
Electron, and it talks to YouTube the same way the website does.

## What this is, and what it isn't

It's a different front end for your own YouTube Music account. You sign in as yourself, and
it shows your home feed, your library and your playlists, fetched from YouTube the way the
website fetches them. Likes, playlist edits and listening history go back to your account, so
the website and your phone see them too.

It isn't pirate software. There's no music in this repo or in the builds, nothing is hosted
or shared, and nothing is unlocked for anyone else's account. Every song plays from YouTube,
as you. Downloads are files on your own disk for listening offline, and they're yours to keep
off the internet.

It also isn't official. It's not affiliated with or endorsed by YouTube or Google, and
"YouTube Music" is their trademark. None of the API it uses is documented, and third-party
clients sit outside YouTube's terms of service. I use it every day anyway, but that's a call
you make for your own account. If you like an artist, pay them in whatever way they actually
get paid.

## Numbers

Measured by `scripts/perf.sh` on a release build, with the home feed loaded and its covers on
screen:

| | |
|---|---|
| its own memory (private dirty) | ~96 MB |
| its share of the system's (PSS) | ~141 MB |
| drawing a frame | under 1 ms |
| AppImage | 154 MB, most of which is mpv and yt-dlp |

The script fails if any of those drift past their limits, so they don't quietly creep up.

## What it does

All the usual YouTube Music things: Home, Explore, search, your library, radio, charts,
moods, history, lyrics, an editable queue, and music videos when a song has one. Some parts
are there on purpose:

- **Search that searches.** YouTube's mixed results can bury what you typed under a pile of
  loosely related songs. Picking Songs, Albums or Artists asks YouTube for that category
  directly, instead of filtering the same short list.
- **Playlists that behave.** Create, rename, drag to reorder, and get asked when a song is
  already in one instead of it quietly going in twice. Your own playlists get YouTube's
  Suggestions underneath, with a + on each. Settings can open every playlist newest-first.
- **Liked songs everywhere.** A liked song shows its thumb in search, on Home and on every
  list. Each artist page has a tile with the songs you've liked by them.
- **Local files.** Point Settings at a folder and browse it like a file explorer. Any folder
  can become a playlist.
- **Downloads.** Any song or whole page can be saved for offline listening. The files land in
  `~/Music/refryt` with their tags and cover art embedded, and a downloaded song plays from
  disk even when you're online.
- **Offline doesn't fall over.** No connection gets you a banner and a shortcut to your
  downloads. When the connection comes back, the page reloads by itself.
- **Playback.** mpv underneath, with an audio quality setting, volume normalisation from
  YouTube's own loudness figures, and autoplay that keeps going when your queue runs out.
- **Brand accounts.** If your Google account has more than one channel, Settings lets you
  browse as whichever one.
- **Desktop things.** Tray icon, start on boot, media keys, the OS media controls (MPRIS on
  Linux, SMTC on Windows) and Discord Rich Presence, which is off until you turn it on.
- **Sidebars you can drag.** Both sides resize within sensible limits and remember where you
  left them.

## Keys

These are Spotify's desktop shortcuts, because my fingers already knew them.

| key | what it does |
|---|---|
| space | play / pause |
| up / down | previous / next |
| m | mute |
| ctrl + s | shuffle |
| ctrl + r | cycle repeat |
| ctrl + k or ctrl + l | jump to search |
| ? or F1 | the full list |

Songs select on one click and play on two, also like Spotify. If you'd rather one click just
plays, that's a toggle in Settings. Mouse back and forward buttons walk the history like a
browser.

## Themes

22 of them, and each one is a [plain CSS file](docs/THEMES.md) in your config folder. Edit
one and the app repaints when you save. Delete one and it's gone. Drop in your own and it
turns up in the list. Headers can also take their colour from the album art, which is on by
default.

The window can be see-through with real compositor blur behind it. That only works where the
compositor offers blur, which today means KDE on Wayland. Windows hides both settings,
because what shows through there is a washed-out grey, not your desktop.

## Signing in

Either sign in from the app's own window (Linux and Windows), or let it borrow the session
from a browser you're already signed into: Chrome, Firefox, Brave, Vivaldi or Edge. There are
no cookies to paste and no codes to type on another device.

Signed out works too, as it does on the website. You just don't get a library.

## Where your stuff lives

Everything stays on your machine:

- `~/.config/refryt` has settings, themes, the saved session and a cache of your liked songs.
  The session is a cookie, stored with owner-only permissions, because it's a password in
  all but name.
- `~/Music/refryt` has downloads.

There's no telemetry. The app talks to YouTube, to [LRCLIB](https://lrclib.net) for synced
lyrics, and to Discord's local client if you turned that on. That's it.

If it crashes, it writes a report to the config folder and offers it on the next launch.
You can copy it or open a prefilled GitHub issue. Nothing is sent by itself.

## Status

Pre-1.0, and it moves quickly. Worth knowing:

- **Linux is where it's used and tested.** The Windows installer and zip build from the same
  source, but haven't spent much time on real Windows hardware.
- **YouTube breaks it now and then.** They don't send notice when something changes. Fixes
  usually land within a few days.
- **No podcasts.** Episodes aren't supported at all.
- **No macOS build.** Most of it would carry over, but nobody's done it yet.

## Install

**Linux.** Grab the AppImage from [releases](https://github.com/gridoe/refryt/releases), or
build it:

```sh
./scripts/build-appimage.sh
./scripts/appimage-install.sh install     # into ~/.local/bin, with a launcher entry
./scripts/appimage-install.sh uninstall   # and back out again
```

Uninstalling leaves `~/.config/refryt` and your downloads alone. Delete those yourself if
you want everything gone.

**Windows.** There's an installer and a portable zip on the same releases page. Both come out
of `./scripts/build-windows.sh`, which cross-builds from Linux.

## Build

It's a Cargo workspace with a `Justfile` on top. `just` builds and runs it, `just release`
does the optimised build, and `just check` runs formatting, clippy and the tests.

You'll need Rust and libmpv. yt-dlp is optional but worth having, because it's what playback
falls back on when YouTube won't hand a stream over directly. [docs/BUILDING.md](docs/BUILDING.md)
has the full setup and the Windows cross-build.

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
- **[youtui](https://github.com/nick42d/youtui)**: `ytmapi-rs` for the InnerTube API, and its
  maintained `rusty_ytdl` fork behind the optional native resolver.
- **[ytmdesktop](https://github.com/ytmdesktop/ytmdesktop)**: what happens when the queue
  runs out.
- **Spotify's desktop app**: the shortcuts, select-then-play clicks, and the shape of the
  album, playlist and artist pages.
- **[LRCLIB](https://lrclib.net)**: the synced lyrics.
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
