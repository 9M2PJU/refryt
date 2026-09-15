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
    let stats_after = args
        .iter()
        .position(|a| a == "--stats-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<f64>().ok())
        .map(std::time::Duration::from_secs_f64);

    // Override the saved scheme and opacity for this run only — never written to
    // `settings.json` — so the whole scheme/flavour × opacity matrix can be screenshotted
    // without clicking through Settings for each one.
    let scheme = args
        .iter()
        .position(|a| a == "--scheme")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| refryt::theme::Scheme::parse_with_flavour(s));
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
        viewport = viewport.with_decorations(false).with_transparent(true);
    }

    eframe::run_native(
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
                        page: page.clone(),
                        autoscroll,
                        queue,
                        lyrics,
                        scheme,
                        opacity,
                        no_blur,
                        no_autoplay,
                    },
                )
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("{e:#}").into()
                })?,
            ))
        }),
    )
}
