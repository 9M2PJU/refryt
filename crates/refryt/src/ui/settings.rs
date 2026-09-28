//! Sign-in.
//!
//! The primary path is importing cookies from a browser you are already signed into. It is not a
//! convenience: a cookie copied from the DevTools console *cannot* work, because `document.cookie`
//! is specified not to expose `HttpOnly` cookies and the ones that authenticate a YouTube session
//! — `HSID`, `SSID`, `LOGIN_INFO`, `__Secure-1PSID`, `__Secure-3PSID` — are all HttpOnly. Pasting
//! from the console produces a cookie that `ytmapi-rs` accepts and YouTube ignores.

use egui::RichText;

use crate::app::App;
use crate::auth::AuthState;
use crate::browser_cookies;
#[cfg(not(target_os = "windows"))]
use crate::settings::Opacity;
use crate::settings::{Quality, Theme};
use crate::theme;
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    // The title scrolls away with the page rather than staying pinned over it.
    crate::ui::page_scroll(app, "settings").show(ui, |ui| {
        crate::ui::page_title(ui, &app.palette, "Settings");
        // Capped, and held clear of the card's right edge: with the queue or lyrics open the
        // card is narrower than the cap, and the switches sat on its edge, against the panel.
        ui.set_max_width(COLUMN_MAX_W.min(ui.available_width() - RIGHT_GUTTER));
        appearance(app, ui);
        ui.add_space(28.0);
        playback(app, ui);
        ui.add_space(28.0);
        system(app, ui);
        ui.add_space(28.0);
        integrations(app, ui);
        ui.add_space(28.0);
        local_music(app, ui);
        ui.add_space(28.0);
        heading(app, ui, "Account");
        match &app.auth {
            AuthState::SignedIn { .. } => signed_in(app, ui),
            AuthState::Checking => {
                ui.label(
                    RichText::new("Checking your saved session…")
                        .font(theme::regular(13.0))
                        .color(app.palette.secondary),
                );
            }
            _ => sign_in(app, ui),
        }
        ui.add_space(28.0);
        heading(app, ui, "Help");
        if widgets::pill_button(ui, &app.palette, "Keyboard Shortcuts").clicked() {
            app.help_open = true;
        }
        ui.add_space(28.0);
        branding(app, ui);
    });
}

/// An open-source app's own about-ish footer: icon, name, version, licence. Last thing on the
/// page rather than the header — the sidebar item that got you here is already labelled
/// "Settings", so the top of the page saying it again is the redundant one.
fn branding(app: &mut App, ui: &mut egui::Ui) {
    if app.logo.is_none() {
        app.logo = crate::logo_texture(ui.ctx());
    }
    ui.horizontal(|ui| {
        if let Some(logo) = &app.logo {
            ui.add(egui::Image::new((logo.id(), egui::Vec2::splat(32.0))));
            ui.add_space(8.0);
        }
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(
                RichText::new("refryt")
                    .font(theme::semibold(14.0))
                    .color(app.palette.text),
            );
            ui.label(
                RichText::new(format!("v{} · GPL-3.0-or-later", env!("CARGO_PKG_VERSION")))
                    .font(theme::regular(11.0))
                    .color(app.palette.dim),
            );
        });
    });
}

/// Why the last sign-in attempt failed, shown wherever the user is.
fn sign_in_error(app: &App, ui: &mut egui::Ui) {
    let Some(msg) = &app.sign_in_error else {
        return;
    };
    ui.label(
        RichText::new(msg)
            .font(theme::regular(12.0))
            .color(app.palette.danger),
    );
    ui.add_space(10.0);
}

fn heading(app: &App, ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(theme::bold(17.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);
}

/// What needs saying under the Scheme row: the painting theme's warnings, a chosen theme that
/// is gone, the built-in stand-in when the folder has nothing of this mode, and files that
/// failed to read.
fn theme_notes(app: &App, ui: &mut egui::Ui, dark: bool) {
    let themes = &app.custom_themes;
    let mode = if dark { "dark" } else { "light" };
    let chosen = if dark {
        &app.settings.dark_theme
    } else {
        &app.settings.light_theme
    };
    match app.settings.painting_theme(dark, themes) {
        Some(t) => {
            if t.name != *chosen {
                problem(
                    app,
                    ui,
                    &format!("“{chosen}” is missing, using “{}”.", t.name),
                );
            }
            for warning in &t.warnings {
                note(app, ui, warning);
            }
        }
        None => problem(
            app,
            ui,
            &format!("No {mode} themes in the folder, using Classic."),
        ),
    }
    for (file, error) in &themes.errors {
        problem(app, ui, &format!("{file}: {error}"));
    }
}

/// A [`note`] that reports something wrong.
fn problem(app: &App, ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(theme::regular(11.0))
            .color(app.palette.danger),
    );
}

fn note(app: &App, ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(theme::regular(11.0))
            .color(app.palette.dim),
    );
}

/// A label followed by controls on one line, centred together. `ui.horizontal` sizes its row
/// from `interact_size.y` (18pt) and centres the label on that band, so a 32pt chip hangs below
/// it; giving the row the chip's own height first keeps the two level.
/// The label column of a [`labeled_row`].
const LABEL_W: f32 = 104.0;

/// How wide the settings column gets. Past this the switches, right-aligned, drifted a window's
/// width away from the words that name them.
const COLUMN_MAX_W: f32 = 720.0;
/// Room kept between the settings column and the card's right edge when the card is narrower
/// than [`COLUMN_MAX_W`].
const RIGHT_GUTTER: f32 = 40.0;

fn labeled_row(
    palette: &theme::Palette,
    ui: &mut egui::Ui,
    label: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 32.0;
        ui.horizontal(|ui| {
            // A fixed label column, so the chips and dropdowns of consecutive rows start on one
            // line instead of wherever their own label happened to end.
            ui.allocate_ui_with_layout(
                egui::vec2(LABEL_W, 32.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(LABEL_W);
                    ui.label(
                        RichText::new(label)
                            .font(theme::regular(12.0))
                            .color(palette.secondary),
                    );
                },
            );
            // Wrapping, within what is left of the column: the Theme row's dropdown and folder
            // button are wider than a narrow card (queue open), and a row that overflows widens
            // the column itself — which pushed every switch below it out against the panel.
            // Wrapped, the button drops under the dropdown instead.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 32.0),
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                contents,
            );
        });
    });
}

/// A titled row with a persisted on/off switch. The Playback section is three of these, so the
/// wiring lives here once.
fn toggle_row(
    app: &App,
    ui: &mut egui::Ui,
    id: &'static str,
    title: &str,
    detail: &str,
    value: &mut bool,
) {
    widgets::setting_row(ui, &app.palette, title, detail, |ui| {
        widgets::toggle(ui, &app.palette, ui.id().with(id), value);
    });
}

fn appearance(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "Appearance");

    let mut theme_choice = app.settings.theme;
    labeled_row(&app.palette, ui, "Mode", |ui| {
        for option in Theme::ALL {
            if widgets::chip(ui, &app.palette, option.label(), theme_choice == option).clicked() {
                theme_choice = option;
            }
        }
    });
    ui.add_space(8.0);

    // The mode on screen decides which themes are offered: a theme file says whether it is
    // dark or light, and picking one sets the theme for that mode only, so switching modes
    // (or following the desktop) swaps between two remembered choices.
    let dark = app.palette.dark;
    let mut dark_choice = app.settings.dark_theme.clone();
    let mut light_choice = app.settings.light_theme.clone();
    let offered: Vec<&crate::theme::custom::CustomTheme> = app
        .custom_themes
        .themes
        .iter()
        .filter(|t| t.dark == dark)
        .collect();
    let options: Vec<(egui::Color32, &str)> = offered
        .iter()
        .map(|t| (t.palette().accent, t.name.as_str()))
        .collect();
    let painting = app.settings.painting_theme(dark, &app.custom_themes);
    let (current_swatch, current_label) = match painting {
        Some(t) => (t.palette().accent, t.name.as_str()),
        None => (app.palette.accent, "Classic (built-in)"),
    };
    labeled_row(&app.palette, ui, "Theme", |ui| {
        if let Some(i) = widgets::dropdown(
            ui,
            &app.palette,
            ui.id().with("scheme-dropdown"),
            current_swatch,
            current_label,
            &options,
        ) {
            let name = offered[i].name.clone();
            if dark {
                dark_choice = name;
            } else {
                light_choice = name;
            }
        }
        ui.add_space(8.0);
        if widgets::outline_button(ui, &app.palette, "Open Themes Folder").clicked() {
            match crate::theme::custom::dir() {
                Some(dir) => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        tracing::warn!(%e, "could not create the themes folder");
                    }
                    crate::open_url(&dir.to_string_lossy());
                }
                None => tracing::warn!("no config directory for the themes folder"),
            }
        }
    });
    theme_notes(app, ui, dark);
    ui.add_space(8.0);

    // Windows gets no transparency and no blur row at all. Its per-pixel-alpha route composites
    // the shell over a flat DWM material — a washed-out white, not the desktop — so the slider
    // promised see-through that could never look right, and the blur behind that material was
    // invisible. Absent beats a switch that lies; the window is opaque there (`app::App::new`
    // forces the stored values off, in case a settings file came from Linux).
    #[cfg(not(target_os = "windows"))]
    {
        // A slider, not chips: transparency is a continuum, and four names only ever made the
        // setting look like a choice of four windows. The label centres on the slider's own row.
        let mut sheer = app.settings.opacity.sheer();
        // The same shape as the switches below it — title, a line under it, the control at the
        // right — rather than a grey label-column row with its note floating a line lower: in a
        // narrow card (queue or lyrics open) the two styles side by side, with the slider's end
        // short of the switches' edge, read as a page that had come apart.
        let decorated = app.decorated;
        let percent = format!("{:.0}%", sheer / Opacity::MAX * 100.0);
        widgets::setting_row(
            ui,
            &app.palette,
            "Transparency",
            if decorated {
                "Not available with `--decorated`."
            } else {
                "How much of the desktop shows through."
            },
            |ui| {
                // `--decorated` gives the window back to the OS, which paints its own opaque
                // background — the slider would do nothing, so it is dimmed and inert rather
                // than silently pretending.
                ui.add_enabled_ui(!decorated, |ui| {
                    // The number the slider only implies, in a fixed slot at the row's right
                    // edge so the changing digits move nothing: a percentage of the slider's
                    // whole range, which reaches past the old maximum — not of the composite
                    // alpha.
                    ui.allocate_ui_with_layout(
                        egui::vec2(34.0, 20.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new(&percent)
                                    .font(theme::regular(12.0))
                                    .color(app.palette.text),
                            );
                        },
                    );
                    ui.add_space(4.0);
                    widgets::slider(ui, &app.palette, &mut sheer, 0.0..=Opacity::MAX, 150.0);
                });
            },
        );

        let blur_available = app.blur.available();
        let blur_useful = sheer > 0.0;
        let mut blur = app.settings.blur;
        widgets::setting_row(
            ui,
            &app.palette,
            "Background Blur",
            if !blur_available {
                "Your compositor doesn't support it."
            } else if !blur_useful {
                "Turn up transparency to use it."
            } else {
                "Blurs the desktop behind the window."
            },
            |ui| {
                // Dimmed and inert rather than clickable-but-pointless when there is nothing to
                // turn on — no compositor support, or nothing set to show through it yet.
                widgets::toggle_enabled(
                    ui,
                    &app.palette,
                    ui.id().with("setting-blur"),
                    &mut blur,
                    blur_available && blur_useful,
                );
            },
        );

        let ctx = ui.ctx().clone();
        app.update_settings(&ctx, |s| {
            s.opacity = Opacity::new(sheer);
            s.blur = blur;
        });
    }

    let mut adaptive_art_color = app.settings.adaptive_art_color;
    toggle_row(
        app,
        ui,
        "setting-adaptive-art-color",
        "Adaptive Header Colour",
        "Tints album and playlist headers to match the cover.",
        &mut adaptive_art_color,
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        s.theme = theme_choice;
        s.dark_theme = dark_choice;
        s.light_theme = light_choice;
        s.adaptive_art_color = adaptive_art_color;
    });
}

fn playback(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "Playback");

    let mut quality = app.settings.quality;
    labeled_row(&app.palette, ui, "Audio Quality", |ui| {
        for option in Quality::ALL {
            if widgets::chip(ui, &app.palette, option.label(), quality == option).clicked() {
                quality = option;
            }
        }
    });
    note(app, ui, quality.detail());
    ui.add_space(8.0);

    let mut autoplay = app.settings.autoplay;
    toggle_row(
        app,
        ui,
        "setting-autoplay",
        "Autoplay",
        "Plays similar songs when your queue ends.",
        &mut autoplay,
    );

    let mut normalise = app.settings.normalise_volume;
    toggle_row(
        app,
        ui,
        "setting-normalise",
        "Audio Normaliser",
        "Keeps the volume consistent between tracks.",
        &mut normalise,
    );

    let mut scrobble = app.settings.scrobble;
    toggle_row(
        app,
        ui,
        "setting-scrobble",
        "Save Listening History",
        "Adds what you play to your YouTube history.",
        &mut scrobble,
    );

    let mut double_click = app.settings.double_click_to_play;
    toggle_row(
        app,
        ui,
        "setting-double-click",
        "Double-Click to Play",
        "One click selects, two clicks play.",
        &mut double_click,
    );

    ui.add_space(8.0);
    let mut playlist_sort = app.settings.playlist_default_sort;
    labeled_row(&app.palette, ui, "Playlist Order", |ui| {
        for option in crate::app::PlaylistSort::ALL {
            let label = match option {
                // Named for what it shows in a playlist you add to: the latest songs on top.
                crate::app::PlaylistSort::Default => "As saved",
                crate::app::PlaylistSort::Reverse => "Newest added first",
                other => other.label(),
            };
            if widgets::chip(ui, &app.palette, label, playlist_sort == option).clicked() {
                playlist_sort = option;
            }
        }
    });
    note(
        app,
        ui,
        "How playlists are ordered when you open them. The sort button on a playlist still changes it for that visit.",
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        s.playlist_default_sort = playlist_sort;
        s.quality = quality;
        s.autoplay = autoplay;
        s.normalise_volume = normalise;
        s.scrobble = scrobble;
        s.double_click_to_play = double_click;
    });
}

fn system(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "System");

    // Both platforms this ships to have a mechanism — an XDG autostart entry on Linux, the
    // per-user `Run` key on Windows (`autostart`'s own doc comment) — so the row exists
    // wherever one does, and nowhere else.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let mut start_on_boot = app.settings.start_on_boot;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    toggle_row(
        app,
        ui,
        "setting-start-on-boot",
        "Launch on Startup",
        "Opens refryt when you log in.",
        &mut start_on_boot,
    );

    let mut hide_tray = app.settings.hide_tray_on_close;
    // Wayland has no way to hide a window and show it again from code — only a real minimize,
    // and only in the hide direction (`app::hide_command`'s own doc comment has the specifics)
    // — so the promise made here has to be the one Wayland can actually keep, or the toggle
    // reads as broken the first time someone clicks the tray's Show and nothing happens.
    let hide_tray_detail = if crate::app::is_wayland() {
        "Closing minimises instead of quitting. Quit from the tray."
    } else {
        "Closing hides to the tray instead of quitting."
    };
    toggle_row(
        app,
        ui,
        "setting-hide-tray",
        "Close to Tray",
        hide_tray_detail,
        &mut hide_tray,
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            s.start_on_boot = start_on_boot;
        }
        s.hide_tray_on_close = hide_tray;
    });
}

/// Discord Rich Presence. Nothing to configure: the app connects as its own Discord application
/// (the id is fixed in `crate::discord`), so the toggle is the whole setting.
fn integrations(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "Integrations");

    let mut enabled = app.settings.discord_rich_presence;
    widgets::setting_row(
        ui,
        &app.palette,
        "Discord Rich Presence",
        "Shows what you're playing on Discord.",
        |ui| {
            widgets::toggle(
                ui,
                &app.palette,
                ui.id().with("setting-discord"),
                &mut enabled,
            );
        },
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        s.discord_rich_presence = enabled;
    });
}

/// Folders scanned for local audio files (mp3, wav, flac, and more), shown in the Library's
/// Local tab. Adding or removing a folder here is what triggers the rescan — see
/// `App::update_settings`'s own diff branch for `local_folders`.
fn local_music(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "Local Files");
    note(
        app,
        ui,
        "Folders to scan for music. They show up in Library → Local.",
    );
    ui.add_space(8.0);

    let mut folders = app.settings.local_folders.clone();
    let mut removed = None;
    for (i, folder) in folders.iter().enumerate() {
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, &app.palette, crate::icons::Icon::X, 12.0, false).clicked()
            {
                removed = Some(i);
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(folder.as_str())
                    .font(theme::regular(12.0))
                    .color(app.palette.secondary),
            );
        });
        ui.add_space(2.0);
    }
    if let Some(i) = removed {
        folders.remove(i);
    }
    ui.add_space(4.0);

    if widgets::pill_button(ui, &app.palette, "Add folder").clicked()
        && let Some(dir) = rfd::FileDialog::new().pick_folder()
    {
        let path = dir.to_string_lossy().into_owned();
        if !folders.contains(&path) {
            folders.push(path);
        }
    }

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        s.local_folders = folders;
    });
}

fn sign_in(app: &mut App, ui: &mut egui::Ui) {
    sign_in_error(app, ui);
    if let AuthState::Failed(msg) = &app.auth {
        ui.label(
            RichText::new(msg)
                .font(theme::regular(13.0))
                .color(app.palette.danger),
        );
        ui.add_space(10.0);
    }

    ui.label(
        RichText::new("Sign in to see your library and playlists.")
            .font(theme::regular(13.0))
            .color(app.palette.secondary),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new("Search and playback work without one.")
            .font(theme::regular(12.0))
            .color(app.palette.dim),
    );
    ui.add_space(14.0);

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        window_sign_in(app, ui);
        ui.add_space(16.0);
    }
    browser_import(app, ui);
}

/// The path that needs no browser already signed in: the app's own window owns the session it
/// creates. On Windows that is also the only path that works at all — a modern Chromium's cookie
/// store is sealed with App-Bound Encryption, so the only process that can read it is the browser
/// itself, and this window is one. See [`crate::webview_login`].
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn window_sign_in(app: &mut App, ui: &mut egui::Ui) {
    ui.label(
        RichText::new("Sign-In Window")
            .font(theme::semibold(13.0))
            .color(app.palette.text),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new("Signs in through Google in a small window of its own.")
            .font(theme::regular(12.0))
            .color(app.palette.dim),
    );
    ui.add_space(8.0);

    if widgets::pill_button(ui, &app.palette, "Sign In").clicked() {
        // Deliberately not `auth = Checking`: that state means "verifying a stored session at
        // startup", and `AuthChanged` reads it to tell an optimistically personalised feed from
        // an anonymous one a fresh sign-in has just invalidated. The state changes when the
        // attempt actually lands.
        app.backend.sign_in_from_window();
    }
}

/// The path that works: read the cookies out of a browser that is already signed in.
fn browser_import(app: &mut App, ui: &mut egui::Ui) {
    ui.label(
        RichText::new(if cfg!(any(target_os = "linux", target_os = "windows")) {
            "Or Import from a Browser"
        } else {
            "Import from a Browser"
        })
        .font(theme::semibold(13.0))
        .color(app.palette.text),
    );
    ui.add_space(4.0);

    let browsers = app
        .detected_browsers
        .get_or_insert_with(browser_cookies::detected);
    if browsers.is_empty() {
        ui.label(
            RichText::new(if cfg!(target_os = "windows") {
                "No Firefox profile found. Sign in to music.youtube.com there, or use the \
                 window above."
            } else {
                "No signed-in browser found. Sign in to music.youtube.com in Firefox, Chrome, \
                 Chromium, Brave, Vivaldi or Edge."
            })
            .font(theme::regular(12.0))
            .color(app.palette.dim),
        );
        return;
    }

    ui.label(
        RichText::new("Uses the session from a browser you're already signed into.")
            .font(theme::regular(12.0))
            .color(app.palette.dim),
    );
    // No "close the browser first" hint any more: on Windows only Firefox is ever offered here,
    // and Firefox neither locks its store nor seals it, so there is nothing to close for. The
    // Chromium browsers that needed that advice are not shown there at all — see
    // `browser_cookies::readable_on_this_os`.
    ui.add_space(8.0);

    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for b in browsers.iter() {
            if widgets::pill_button(ui, &app.palette, b.label).clicked() {
                chosen = Some(b.key);
            }
            ui.add_space(6.0);
        }
    });
    if let Some(key) = chosen {
        // Same reason as the sign-in window button above: `Checking` belongs to the startup
        // session check, not to an interactive attempt.
        app.backend.sign_in_from_browser(key.to_owned());
    }
}

fn signed_in(app: &mut App, ui: &mut egui::Ui) {
    // A refused sign-in attempt is reported here too — the session it was meant to replace is
    // still perfectly good, and the app must not imply otherwise.
    sign_in_error(app, ui);
    let account = match &app.auth {
        AuthState::SignedIn { account } => account.clone(),
        _ => String::new(),
    };
    // Asked for once per visit; a Google account may own brand channels besides the personal one.
    if app.accounts.is_empty() {
        app.backend.fetch_accounts(app.account_epoch);
    }
    // Naming the account is the point: it is YouTube confirming the session, not us assuming it.
    // The Premium badge rides the same line: downloads follow it, so it should be seen.
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if account.is_empty() {
                "Signed in".to_owned()
            } else {
                format!("Signed in as {account}")
            })
            .font(theme::medium(13.0))
            .color(app.palette.text),
        );
        if app.premium {
            ui.add_space(6.0);
            premium_badge(ui, &app.palette);
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new(if app.premium {
            "Your library and Home feed are synced, and downloads are on."
        } else {
            "Your library and Home feed are synced. Downloads need YouTube Premium."
        })
        .font(theme::regular(12.0))
        .color(app.palette.secondary),
    );
    ui.add_space(14.0);
    channels(app, ui);
    ui.add_space(16.0);
    if widgets::pill_button(ui, &app.palette, "Sign out").clicked() {
        app.backend.sign_out();
    }
}

/// Brand accounts: one Google login can own several YouTube channels, each with its own library.
fn channels(app: &mut App, ui: &mut egui::Ui) {
    if app.accounts.len() < 2 {
        return;
    }
    ui.label(
        RichText::new("Browse as")
            .font(theme::medium(12.0))
            .color(app.palette.text),
    );
    ui.add_space(4.0);

    let current = app.settings.account_page_id.clone();
    let mut chosen: Option<Option<String>> = None;
    for account in &app.accounts {
        let selected = current == account.page_id;
        let label = if account.detail.is_empty() {
            account.name.clone()
        } else {
            format!("{}  ·  {}", account.name, account.detail)
        };
        if widgets::chip(ui, &app.palette, &label, selected).clicked() && !selected {
            chosen = Some(account.page_id.clone());
        }
    }
    note(app, ui, "Each channel has its own library.");

    if let Some(page_id) = chosen {
        let ctx = ui.ctx().clone();
        app.update_settings(&ctx, |s| s.account_page_id = page_id);
        app.forget_account_data();
    }
}

/// "YouTube Premium · Active": a small accent-tinted pill beside the account name.
fn premium_badge(ui: &mut egui::Ui, palette: &crate::theme::Palette) {
    egui::Frame::NONE
        .fill(palette.accent.gamma_multiply(0.16))
        .corner_radius(egui::CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new("YouTube Premium · Active")
                    .font(theme::semibold(11.0))
                    .color(palette.accent),
            );
        });
}
