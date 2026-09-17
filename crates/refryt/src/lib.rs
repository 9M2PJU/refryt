//! A fast, native YouTube Music client.
//!
//! The application is a library with a thin binary on top, so the diagnostics in `src/bin/` can
//! exercise the *same* code paths the app uses. That matters most for `api-check`: search now runs
//! through our own parser (see [`innertube`]), and a canary that tested `ytmapi-rs` directly would
//! be watching the fallback rather than the path anyone actually takes.

pub mod api;
pub mod app;
pub mod auth;
/// "Start on boot" — Linux only, see the module's own doc comment.
#[cfg(target_os = "linux")]
pub mod autostart;
pub mod backend;
pub mod blur;
pub mod browser_cookies;
pub mod chrome;
pub mod crash;
pub mod downloads;
pub mod emoji;
pub mod icons;
pub mod images;
pub mod innertube;
pub mod liked;
pub mod local;
pub mod local_playlists;
pub mod media;
pub mod model;
pub mod persist;
pub mod playback;
pub mod player;
pub mod settings;
pub mod stats;
pub mod theme;
pub mod tray;
pub mod ui;
pub mod video;
pub mod widgets;

use std::sync::Arc;

use eframe::egui;

/// Set as early as possible so `--startup-timing` measures from process entry, not from the
/// point eframe hands us a context.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

pub fn mark_start() {
    let _ = START.set(std::time::Instant::now());
}

/// Install the tracing subscriber with a `RUST_LOG`-driven target filter.
///
/// This uses `filter::Targets` rather than `EnvFilter` on purpose: `EnvFilter`'s dynamic
/// directive matching pulls in `regex-automata` + `regex-syntax` (~14 MB of rlib), which was
/// the only regex anywhere in the default build. `Targets` parses the same
/// `warn,refryt=info`-style syntax without it. An unparseable `RUST_LOG` falls back to `default`.
pub fn init_tracing(default: &str) {
    use tracing_subscriber::{filter::Targets, fmt, prelude::*};

    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            default
                .parse::<Targets>()
                .expect("valid default log filter")
        });

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}

pub fn elapsed_since_start() -> std::time::Duration {
    START.get().map(|t| t.elapsed()).unwrap_or_default()
}

/// Hand a URL to the desktop's own handler.
///
/// eframe opens links through its `links` feature, which is part of its default set — and this
/// build turns the defaults off to keep the dependency tree small, so hyperlinks silently did
/// nothing. Spawning the platform opener is the same thing that feature does on Linux, costs no
/// dependency, and matches how the app already shells out to `yt-dlp`.
pub fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let (program, args): (&str, &[&str]) = ("xdg-open", &[]);
    #[cfg(target_os = "macos")]
    let (program, args): (&str, &[&str]) = ("open", &[]);
    #[cfg(target_os = "windows")]
    let (program, args): (&str, &[&str]) = ("cmd", &["/C", "start", ""]);

    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // `cmd` is a console-subsystem binary; without this, spawning it from this GUI-subsystem
    // app flashes a new console window open for the length of the call.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.spawn() {
        Ok(_) => {}
        Err(e) => tracing::warn!(%url, error = %e, "could not open a link"),
    }
}

/// The app's own artwork: the window/taskbar/launcher icon, and (as an egui texture instead of
/// an OS-level `IconData`) the branding shown in Settings. One embed, two decodes, so a change to
/// the master PNG only has one place to update.
const LOGO_PNG: &[u8] = include_bytes!("../../../assets/icons/app/refryt-256.png");

/// The window / taskbar / launcher icon, decoded from the same preprocessed artwork that
/// `just install` places into hicolor. `None` when the bytes cannot be decoded — the app still
/// runs, just without an icon.
pub fn window_icon() -> Option<Arc<egui::IconData>> {
    let img = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(Arc::new(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }))
}

/// The same artwork as a texture, for the app's own UI (currently just the Settings page)
/// rather than the OS chrome `window_icon` is for. Loaded once and cached by the caller — a
/// fresh `egui::Context::load_texture` call is cheap to repeat, but there is no reason to.
pub fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let img = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], img.as_raw());
    Some(ctx.load_texture("refryt-logo", image, egui::TextureOptions::LINEAR))
}
