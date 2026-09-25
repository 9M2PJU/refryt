//! Binary entry point. Everything of substance lives in the library — see `lib.rs`.

// Windows-only, ignored elsewhere: without it a GUI build still launches with a console window
// behind it, since Windows otherwise defaults every binary to the console subsystem.
#![windows_subsystem = "windows"]

use eframe::egui;
use refryt::{app, mark_start, window_icon};

fn main() -> eframe::Result {
    mark_start();

    refryt::init_tracing("warn,refryt=info");
    // Record a panic before the abort so the next launch can offer it.
    refryt::crash::install_hook();

    // Frameless is the default look, but it is the first thing to fail on an unusual setup:
    // X11 without a compositor paints the rounded corners black, and some tiling WMs ignore
    // client-side decorations. `--decorated` hands the window back to the OS.
    let decorated = std::env::args().any(|a| a == "--decorated");
    // Print the time to first painted frame and exit. This is the only honest way to measure
    // cold start: everything before the first frame is invisible to the user, and everything
    // after it is not startup.
    let startup_timing = std::env::args().any(|a| a == "--startup-timing");

    // Launch straight into a search, and optionally start playing its first hit. Handy on its
    // own for a keyboard-first app, and the only way to exercise the whole chain without a mouse.
    let args: Vec<String> = std::env::args().collect();
    let search = args
        .iter()
        .position(|a| a == "--search")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let autoplay = args.iter().any(|a| a == "--autoplay");

    // Live counters. `--stats` shows the overlay (F2 toggles it at runtime); `--stats-after N`
    // prints one machine-readable line after N seconds and exits, which is how the perf script
    // measures an app that has actually loaded something.
    // Start on a named page. `--page search` is a page that fetches nothing, which is how the
    // perf script measures the app's floor now that the default launch loads the home feed.
    let page = args
        .iter()
        .position(|a| a == "--page")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| app::Route::parse(p));

    let stats = args.iter().any(|a| a == "--stats");
    let autoscroll = args.iter().any(|a| a == "--autoscroll");
    let queue = args.iter().any(|a| a == "--queue");
    let lyrics = args.iter().any(|a| a == "--lyrics");

    // Open at a given desktop coordinate, without taking focus. For testing on a second screen
    // while someone is working on the first.
    let position = args
        .iter()
        .position(|a| a == "--position")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.split_once(','))
        .and_then(|(x, y)| Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?)));
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(|p| {
            let value = |flag: &str| {
                args.iter()
                    .position(|a| a == flag)
                    .and_then(|i| args.get(i + 1))
                    .cloned()
            };
            let when = match value("--screenshot-on").as_deref() {
                Some("ring") => app::ShotWhen::Ring,
                Some("skeleton") => app::ShotWhen::Skeleton,
                _ => app::ShotWhen::After(std::time::Duration::from_secs_f64(
                    value("--screenshot-after")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(8.0),
                )),
            };
            (std::path::PathBuf::from(p), when)
        });
    let stats_after = args
        .iter()
        .position(|a| a == "--stats-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<f64>().ok())
        .map(std::time::Duration::from_secs_f64);

    // Override the saved theme and opacity for this run only — never written to
    // `settings.json` — so the whole theme × opacity matrix can be screenshotted without
    // clicking through Settings for each one. `--scheme` takes a theme's name however it is
    // typed (`catppuccin-frappe`); it is matched once the themes folder has been read.
    let scheme = args
        .iter()
        .position(|a| a == "--scheme")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let opacity = args
        .iter()
        .position(|a| a == "--opacity")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| refryt::settings::Opacity::parse(s));
    // A safety hatch, not a test aid: `blur` persists to disk, and a Wayland protocol error in
    // the blur binding is fatal to the whole connection — a session that dies on it needs a way
    // back in that does not depend on reaching the Settings page.
    let no_blur = args.iter().any(|a| a == "--no-blur");
    // Autoplay off for one run, without touching `settings.json`: the switch is in Settings, and
    // the A/B of a queue with and without its tail should not need a click to set up.
    let no_autoplay = args.iter().any(|a| a == "--no-autoplay");
    // Drive the tray without a mouse: hide after N seconds, show three seconds later, quit three
    // seconds after that — each injected through the same queue-and-wake path a real tray click
    // takes. A test aid, like `autoscroll`: a tray icon cannot be clicked from outside the
    // process, and this is what makes quit latency measurable from a script.
    let tray_test = args
        .iter()
        .position(|a| a == "--tray-test")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<f64>().ok())
        .map(std::time::Duration::from_secs_f64);
    // Passed by the autostart entry (`autostart::ARG`): login launched this, so once the window
    // is up it should ask the compositor to bring it forward — Wayland starts it behind whatever
    // window already has focus, and nothing can raise a window that has not asked.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let autostart = args.iter().any(|a| a == refryt::autostart::ARG);
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let autostart = false;

    // The child half of the in-app sign-in window: `webview_login::capture` starts this same
    // binary with `--sign-in-window <file>`, because tao's event loop cannot be created beside
    // eframe's winit one in the same process (it faults natively on Windows). Handled here —
    // before any of the app is set up — so the child is nothing but the sign-in window.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Some(at) = args.iter().position(|a| a == refryt::webview_login::ARG) {
        let path = args.get(at + 1).cloned().unwrap_or_default();
        return match refryt::webview_login::capture_here(std::path::Path::new(&path)) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        };
    }

    // One instance per profile: a plain second launch — the desktop entry, the AppImage file —
    // tells the running instance to show its window and exits instead of starting a second
    // player. A launch with any other argument is scripted (`--page`, `--search`, a test aid),
    // which is not a user clicking the app again, so it runs unguarded. See `instance`'s own
    // module doc.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let plain = args.iter().skip(1).all(|a| a == refryt::autostart::ARG);
        if plain
            && matches!(
                refryt::instance::claim(),
                refryt::instance::Claim::Secondary
            )
        {
            return Ok(());
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Refryt")
        .with_app_id("refryt")
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([760.0, 480.0]);

    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }

    if let Some((x, y)) = position {
        viewport = viewport
            .with_position([x, y])
            // Do not raise or focus: the point is to leave the other screen alone.
            .with_active(false);
    }

    if !decorated {
        viewport = viewport.with_decorations(false);
        // Windows does not get asked at all: the shell is opaque there (see `app::App::new`),
        // and a transparent request there only ever put the window in front of DWM's flat
        // material. Every other compositor is asked for the see-through shell it can blend.
        #[cfg(not(target_os = "windows"))]
        {
            viewport = viewport.with_transparent(true);
        }
    }

    let result = eframe::run_native(
        "refryt",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(
                app::App::new(
                    cc,
                    app::Options {
                        decorated,
                        startup_timing,
                        search: search.clone(),
                        autoplay,
                        stats,
                        stats_after,
                        screenshot: screenshot.clone(),
                        page: page.clone(),
                        autoscroll,
                        queue,
                        lyrics,
                        scheme,
                        opacity,
                        no_blur,
                        no_autoplay,
                        tray_test,
                        autostart,
                    },
                )
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("{e:#}").into()
                })?,
            ))
        }),
    );
    // The app itself was dropped inside `run_native` (eframe destroys it on `LoopExiting`), so
    // this line is the log's proof that teardown finished: a slow quit that never reaches it is
    // stuck in one of the drops `App::drop` starts.
    tracing::info!("event loop exited");
    result
}
