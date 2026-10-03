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
Electron, and it talks to YouTube the same way the website does. With the home feed loaded it
uses around 100 MB of memory.

## Disclaimer

> [!IMPORTANT]
> **refryt is not a piracy tool.** It contains no music and does not host, share or unlock any
> content. Every song is streamed from YouTube under the account you sign in with.

**YouTube Premium.** refryt is intended for use with a YouTube Premium or YouTube Music Premium
subscription. It plays without ads and in the background, which YouTube offers as Premium
features. Downloads are available only when the signed-in account holds Premium: the app
checks the account's Paid memberships page and shows the result in Settings. Downloaded files
are for personal offline listening and should not be redistributed.

**Your account.** refryt is a front end for your own YouTube Music account. Likes, playlist
edits and listening history are saved to that account, so they appear on the website and in
the mobile apps as well.

**Not affiliated.** refryt is an unofficial client and is not affiliated with, endorsed by or
sponsored by YouTube or Google. "YouTube" and "YouTube Music" are trademarks of Google LLC. The
app uses YouTube's undocumented internal API, and third-party clients fall outside YouTube's
Terms of Service. Use it at your own discretion.

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
- **Home in your order.** Drag Home's sections into the order you want. YouTube decides what
  exists and sends it in batches, so a late section lands below what's showing. Settings has a
  Strict Home Order switch, off by default, that waits a few seconds longer for your top seven
  sections before it shows Home.
- **Liked songs everywhere.** A liked song shows its thumb in search, on Home and on every
  list. Each artist page has a tile with the songs you've liked by them.
- **Local files.** Point Settings at a folder and browse it like a file explorer. Any folder
  can become a playlist.
- **Downloads, with Premium.** Any song or whole page can be saved for offline listening.
  The files land in `~/Music/refryt` with their tags and cover art embedded, and a downloaded
  song plays from disk even when you're online.
- **Offline doesn't fall over.** No connection gets you a banner and a shortcut to your
  downloads. When the connection comes back, the page reloads by itself.
- **Playback.** mpv underneath, with an audio quality setting, volume normalisation from
  YouTube's own loudness figures, and autoplay that keeps going when your queue runs out.
- **Brand accounts.** If your Google account has more than one channel, the account menu at the
  top lets you browse as whichever one.
- **Desktop things.** Tray icon, start on boot, media keys, the OS media controls (MPRIS on
  Linux, SMTC on Windows, Now Playing on macOS) and Discord Rich Presence, which is off until you turn it on.
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
one and the app repaints when you come back to its window or open Settings. Delete one and it's gone. Drop in your own and it
turns up in the list. Headers can also take their colour from the album art, which is on by
default.

The window can be see-through with real compositor blur behind it. That only works where the
system offers blur, which today means KDE on Wayland and macOS. Windows hides both settings,
because what shows through there is a washed-out grey, not your desktop. Chips, the search box
and the highlights stay see-through with the window, and the cards get a thin edge so they don't
melt into each other; Settings has a switch for that edge, solid window or not.

## Signing in

Either sign in from the app's own window (Linux, Windows and macOS), or let it borrow the session
from a browser you're already signed into: Chrome, Firefox, Brave, Vivaldi or Edge. There are
no cookies to paste and no codes to type on another device.

Signed out works too, as it does on the website. You just don't get a library.

## Where your stuff lives

Everything stays on your machine:

- `~/.config/refryt` (`~/Library/Application Support/refryt` on macOS) has settings, themes, the saved session and a cache of your liked songs.
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
- **macOS is new.** It runs on Apple Silicon. Home, playback, the tray, the in-app sign-in window,
  transparency and blur have been tried by hand. Media keys, Now Playing and the browser cookie
  import are untested there, and it hasn't had the use Linux has. There's no Intel build yet.

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

**Linux, Flatpak.** The releases page also has a `.flatpak` file. It isn't on Flathub, so install
the file itself:

```sh
flatpak install --user refryt-<version>-x86_64.flatpak
flatpak run io.github.gridoe.refryt
```

It needs the GNOME 49 runtime, which Flatpak offers to fetch from Flathub the first time. mpv
and yt-dlp are inside it. The sandbox can't see your browsers' profile folders, so "Import from
a Browser" is hidden and you sign in through the app's own window. It can't write a login item
for your desktop either, so "Launch on Startup" is hidden too. `./scripts/build-flatpak.sh`
builds it from source.

**macOS.** Build the app bundle yourself (needs Homebrew's `mpv` and `dylibbundler`):

```sh
./scripts/build-macos.sh    # dist/Refryt.app, plus a zip of it
```

It's ad-hoc signed, so on another Mac macOS will refuse it until you clear the quarantine flag
with `xattr -cr Refryt.app`. libmpv and yt-dlp are bundled inside.

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

## Credits

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
