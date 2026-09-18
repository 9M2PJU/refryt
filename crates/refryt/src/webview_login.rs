//! Sign in through a browser window this app owns.
//!
//! # Why this exists
//!
//! On Windows there is no other way left. The browser import (`crate::browser_cookies`) shells
//! out to `yt-dlp`, which cannot read a modern Chromium cookie store: since Chromium 127 the
//! values are sealed with App-Bound Encryption and only the browser's own process can unwrap
//! them (`yt-dlp` #10927), and Brave keeps an exclusive lock on the database while it runs on
//! top of that. OAuth is not a fallback either — Google stopped accepting Bearer tokens on the
//! YouTube Music InnerTube surface (sigma67/ytmusicapi#676, yt-dlp/yt-dlp#11462), and the TV
//! client that still accepts them answers in a shape none of this app's parsers read.
//!
//! Asking a browser we control sidesteps both walls. WebView2 is a real Edge browser the OS
//! already ships, and its own cookie API hands back *decrypted* values, HttpOnly included,
//! because the answer comes from inside the WebView2 process. The user sees an ordinary Google
//! sign-in page; nothing is scraped, pasted, or exported.
//!
//! # Why it runs in its own process
//!
//! The window's event loop is tao's, and creating one in a process where eframe's winit loop is
//! already running faults natively — an exception out of a Win32 callback
//! (`STATUS_FATAL_USER_CALLBACK_EXCEPTION`), not a Rust panic, and not something the app can
//! catch. Running it as a fixed extension of this same binary is what a child process is for:
//! `--sign-in-window <file>` is that half, `capture` below spawns it and reads what it wrote.
//! This costs nothing visible — the binary is GUI-subsystem, so the child has no console window
//! — and it means a crash in the sign-in window cannot take the music down with it.
//!
//! Windows-only: on Linux the desktop keyring path already works, and carrying WebKitGTK (a
//! full web engine) for a second route into the same cookies would cost the AppImage far more
//! than it buys.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder};

/// The value stored as the session's `source` so a stale cookie can be refreshed by reopening
/// this same window — see `Api::refresh_from_source`.
pub const SOURCE: &str = "window";

/// The argument that puts the binary into sign-in-window mode, followed by the file to write the
/// captured cookie to.
pub const ARG: &str = "--sign-in-window";

/// The sign-in page, pointed back at YouTube Music when it finishes.
///
/// Signing in on `accounts.google.com` alone leaves the Google cookies but not the `.youtube.com`
/// session; the `continue` target is what makes the browser come back through YouTube and set
/// the cookies this app actually needs before the window is polled.
const SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube\
&continue=https%3A%2F%2Fmusic.youtube.com%2F";

/// How often the cookie jar is looked at while the window is open. Cheap: the API returns the
/// store's contents without a network round trip, and a poll only ever runs while the window is.
const POLL: Duration = Duration::from_secs(1);

/// Open the sign-in window and return the session it captures.
///
/// Runs the window as a child of this same binary — see the module doc for why it cannot be a
/// thread here — and waits for it, so the cookie only comes back once the user is done (or the
/// window is closed, which is an error with the child's own explanation).
pub async fn capture() -> Result<String> {
    let jar = temp_cookie_path();
    // The file holds a live session: gone whether this returns, fails, or the future is dropped.
    let _guard = Cleanup(jar.clone());

    let exe = std::env::current_exe().context("could not find refryt's own executable")?;
    let mut command = tokio::process::Command::new(exe);
    command
        .arg(ARG)
        .arg(&jar)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        // The window is a GUI-subsystem binary, so nothing would appear without this anyway;
        // it is here for the same reason `browser_cookies` sets it — a console must never flash.
        .creation_flags(0x0800_0000)
        // Quitting the app must not leave the window behind.
        .kill_on_drop(true);

    let output = command
        .output()
        .await
        .context("could not start the sign-in window")?;

    if output.status.success() {
        let cookie = std::fs::read_to_string(&jar)
            .context("the sign-in window said it succeeded but left no session behind")?;
        let cookie = cookie.trim().to_owned();
        if cookie.is_empty() {
            bail!("the sign-in window returned an empty session");
        }
        return Ok(cookie);
    }

    // The child's stderr is its explanation — the last line it printed, which is the specific
    // reason rather than a protocol trace.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let why = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or_default();
    if why.is_empty() {
        bail!("the sign-in window exited unexpectedly ({})", output.status);
    }
    bail!("{why}")
}

/// Forget the Google session the sign-in window accumulated.
///
/// Signing out has to reach this, or the next Log in finds Google still signed in inside the
/// window's own profile and silently re-captures the same account: no account to choose and no
/// way to sign in as anyone else. Best effort — the profile is only a cache of credentials, and
/// a failed delete just means the old behaviour.
pub fn forget() {
    if let Ok(dir) = profile_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The child half: open the window, and on success write the cookie to `path`.
///
/// Called from `main` when [`ARG`] is present. A failure is printed by `main` and exits non-zero,
/// which is what tells [`capture`] the missing file is a failed sign-in rather than a bug.
pub fn capture_here(path: &Path) -> Result<()> {
    let cookie = run_window()?;
    std::fs::write(path, cookie)
        .with_context(|| format!("could not write the session to {}", path.display()))
}

/// Where the sign-in window keeps its WebView2 profile.
///
/// Under refryt's config directory so Google's own session outlives the process: reopening the
/// window later — including for a refresh — usually captures a fresh cookie without the user
/// typing anything.
fn profile_dir() -> Result<PathBuf> {
    let path = crate::auth::cookie_path()?;
    let parent = path
        .parent()
        .context("the cookie path has no directory to put the profile in")?;
    Ok(parent.join("sign-in-window"))
}

/// The window and its whole event loop. Blocks until the session is captured or the window is
/// closed, and only returns a cookie once `auth::validate_cookie`'s names are all present.
fn run_window() -> Result<String> {
    let dir = profile_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;

    // This process is the one that exists for this window, so the loop may as well own its
    // thread — but tao still requires the opt-in when that thread is not the process's first.
    let mut builder = EventLoopBuilder::new();
    builder.with_any_thread(true);
    let mut event_loop = builder.build();
    let window = WindowBuilder::new()
        .with_title("Sign in to YouTube Music")
        .with_inner_size(LogicalSize::new(560.0, 780.0))
        .with_min_inner_size(LogicalSize::new(420.0, 560.0))
        .build(&event_loop)
        .context("could not create the sign-in window")?;

    let mut context = WebContext::new(Some(dir));
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(SIGN_IN_URL)
        .build(&window)
        .context(
            "could not open the sign-in window. It needs the Microsoft Edge WebView2 runtime, \
             which Windows 10 and 11 ship; if it was removed, install the Evergreen runtime \
             from Microsoft and try again",
        )?;

    let mut outcome: Option<Result<String>> = None;
    let mut deadline = Instant::now() + POLL;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(deadline);
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                match session_cookie(&webview) {
                    Ok(Some(cookie)) => {
                        outcome = Some(Ok(cookie));
                        *control_flow = ControlFlow::Exit;
                    }
                    Ok(None) => deadline = Instant::now() + POLL,
                    Err(e) => {
                        outcome = Some(Err(e));
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                outcome = Some(Err(anyhow!(
                    "the sign-in window was closed before it finished signing in"
                )));
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });

    outcome.unwrap_or_else(|| bail!("the sign-in window closed without a result"))
}

/// The `Cookie` header for the session the window currently holds, if it is signed in.
///
/// A signed-out visitor to `music.youtube.com` still gets cookies, but not `SAPISID` — the name
/// `auth::validate_cookie` treats as the marker of a real session. Waiting for those names here
/// keeps a half-finished login from being handed to `install_cookie` as if it were one.
fn session_cookie(webview: &wry::WebView) -> Result<Option<String>> {
    let cookies = webview
        .cookies_for_url("https://music.youtube.com")
        .context("could not read the sign-in window's cookies")?;

    let named = |name: &str| cookies.iter().any(|c| c.name() == name);
    if !named("SAPISID") || !(named("__Secure-1PSID") || named("__Secure-3PSID")) {
        return Ok(None);
    }

    let header = cookies
        .iter()
        .filter(|c| !c.name().is_empty())
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(Some(header))
}

/// Where the child writes the session for the parent to read. Unique per run, so two windows
/// opened back to back cannot read each other's file.
fn temp_cookie_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "refryt-sign-in-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ))
}

/// Delete the session file as soon as we are done with it: it is a credential.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
