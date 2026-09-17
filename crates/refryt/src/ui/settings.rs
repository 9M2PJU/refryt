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
use crate::settings::{Opacity, Quality, Theme};
use crate::theme::{self, Scheme, Variant};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.label(
        RichText::new("Settings")
            .font(theme::semibold(18.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);

    crate::ui::page_scroll(app, "settings").show(ui, |ui| {
        appearance(app, ui);
        ui.add_space(22.0);
        playback(app, ui);
        ui.add_space(22.0);
        system(app, ui);
        ui.add_space(22.0);
        local_music(app, ui);
        ui.add_space(22.0);
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
        ui.add_space(22.0);
        heading(app, ui, "Help");
        if widgets::pill_button(ui, &app.palette, "Keyboard shortcuts").clicked() {
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
            .font(theme::semibold(14.0))
            .color(app.palette.text),
    );
    ui.add_space(8.0);
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
fn labeled_row(
    palette: &theme::Palette,
    ui: &mut egui::Ui,
    label: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 32.0;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(label)
                    .font(theme::regular(12.0))
                    .color(palette.secondary),
            );
            ui.add_space(8.0);
            contents(ui);
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
    labeled_row(&app.palette, ui, "Theme", |ui| {
        for option in Theme::ALL {
            if widgets::chip(ui, &app.palette, option.label(), theme_choice == option).clicked() {
                theme_choice = option;
            }
        }
    });
    ui.add_space(8.0);

    // The polarity on screen right now decides which flavours each family offers, and what a
    // row's swatch promises — a flavour is what it is; the Theme only chooses between them.
    let dark = app.palette.dark;
    let mut scheme_choice = app.settings.scheme;
    let mut flavour_choice = app.settings.scheme_variant.clone();
    ui.label(
        RichText::new("Scheme")
            .font(theme::regular(12.0))
            .color(app.palette.secondary),
    );
    ui.add_space(6.0);
    let scheme_options: Vec<(egui::Color32, &'static str)> = Scheme::ALL
        .iter()
        .map(|s| (s.default_variant(dark).swatch(), s.label()))
        .collect();
    let resolved = app.settings.variant(dark);
    if let Some(i) = widgets::dropdown(
        ui,
        &app.palette,
        ui.id().with("scheme-dropdown"),
        resolved.swatch(),
        scheme_choice.label(),
        &scheme_options,
    ) {
        scheme_choice = Scheme::ALL[i];
    }
    ui.add_space(8.0);

    // A second dropdown, and only when the family actually offers a choice in this polarity —
    // nine families would be eight rows of noise otherwise. The rows are the family's own
    // flavours with their own accents, so picking Macchiato is one click, not family-then-flavour.
    let flavours: Vec<&Variant> = scheme_choice
        .variants()
        .iter()
        .filter(|v| v.dark == dark)
        .collect();
    if flavours.len() > 1 {
        ui.label(
            RichText::new("Flavour")
                .font(theme::regular(12.0))
                .color(app.palette.secondary),
        );
        ui.add_space(6.0);
        let flavour_options: Vec<(egui::Color32, &'static str)> =
            flavours.iter().map(|v| (v.swatch(), v.label)).collect();
        let current = scheme_choice.resolve(dark, flavour_choice.as_deref());
        if let Some(i) = widgets::dropdown(
            ui,
            &app.palette,
            ui.id().with("flavour-dropdown"),
            current.swatch(),
            current.label,
            &flavour_options,
        ) {
            flavour_choice = Some(flavours[i].label.to_owned());
        }
        ui.add_space(4.0);
    }
    ui.add_space(4.0);
    // Nord ships no light theme; the note says so rather than letting Snow Storm quietly
    // disappear into a set of derived greys that look like a different, unnamed choice.
    if scheme_choice == Scheme::Nord && !dark {
        note(app, ui, "Snow Storm is derived; Nord ships no light theme");
    }
    ui.add_space(8.0);

    // A slider, not chips: transparency is a continuum, and four names only ever made the
    // setting look like a choice of four windows. The label centres on the slider's own row.
    let mut sheer = app.settings.opacity.sheer();
    labeled_row(&app.palette, ui, "Transparency", |ui| {
        // `--decorated` gives the window back to the OS, which paints its own opaque
        // background — the slider would do nothing, so it is dimmed and inert rather than
        // silently pretending.
        ui.add_enabled_ui(!app.decorated, |ui| {
            widgets::slider(ui, &app.palette, &mut sheer, 0.0..=Opacity::MAX, 220.0);
            ui.add_space(6.0);
            // The number the slider only implies: a percentage is what "how transparent is it"
            // is actually asked and answered in, and the label's x is fixed by the slider's own
            // width, so the changing digits move nothing else. A percentage of the slider's
            // whole range, which reaches past the old maximum — not of the composite alpha.
            ui.label(
                RichText::new(format!("{:.0}%", sheer / Opacity::MAX * 100.0))
                    .font(theme::regular(12.0))
                    .color(app.palette.text),
            );
        });
    });
    ui.add_space(4.0);
    if app.decorated {
        note(
            app,
            ui,
            "The acrylic mode needs the frameless window; `--decorated` hands it back to the OS.",
        );
    } else {
        note(app, ui, "How much of the desktop shows through the shell.");
    }
    ui.add_space(8.0);

    let blur_available = app.blur.available();
    let blur_useful = sheer > 0.0;
    let mut blur = app.settings.blur;
    widgets::setting_row(
        ui,
        &app.palette,
        "Blur what shows through",
        if !blur_available {
            "Your compositor does not offer window blur; the window is simply translucent."
        } else if !blur_useful {
            "Nothing shows through at the current transparency."
        } else {
            "Real blur behind the window, on compositors that offer it."
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

    let mut adaptive_art_color = app.settings.adaptive_art_color;
    toggle_row(
        app,
        ui,
        "setting-adaptive-art-color",
        "Match header colour to the cover art",
        "Tints the album/playlist header with a colour pulled from its cover, instead of the theme's fixed accent.",
        &mut adaptive_art_color,
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        s.theme = theme_choice;
        s.scheme = scheme_choice;
        s.scheme_variant = flavour_choice;
        s.opacity = Opacity::new(sheer);
        s.blur = blur;
        s.adaptive_art_color = adaptive_art_color;
    });
}

fn playback(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "Playback");

    let mut quality = app.settings.quality;
    labeled_row(&app.palette, ui, "Audio quality", |ui| {
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
        "Keeps playing related songs once the queue ends: YouTube's own radio.",
        &mut autoplay,
    );

    let mut normalise = app.settings.normalise_volume;
    toggle_row(
        app,
        ui,
        "setting-normalise",
        "Even out the volume between tracks",
        "Uses YouTube's own loudness measurement, so tracks do not jump in level.",
        &mut normalise,
    );

    let mut scrobble = app.settings.scrobble;
    toggle_row(
        app,
        ui,
        "setting-scrobble",
        "Count plays on your YouTube account",
        "Reports what you play to your watch history. Only while signed in; anonymous listening stays anonymous.",
        &mut scrobble,
    );

    let mut double_click = app.settings.double_click_to_play;
    toggle_row(
        app,
        ui,
        "setting-double-click",
        "Double-click to play",
        "A single click selects a song, a double click plays it — the way Spotify works.",
        &mut double_click,
    );

    let mut crossfade = app.settings.crossfade;
    toggle_row(
        app,
        ui,
        "setting-crossfade",
        "Crossfade between tracks",
        "Dips the volume down at the end of a track and back up as the next begins.",
        &mut crossfade,
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        s.quality = quality;
        s.autoplay = autoplay;
        s.normalise_volume = normalise;
        s.scrobble = scrobble;
        s.double_click_to_play = double_click;
        s.crossfade = crossfade;
    });
}

fn system(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "System");

    // No autostart mechanism is wired up for any target but Linux, so the row itself doesn't
    // exist elsewhere rather than offering a switch that would do nothing.
    #[cfg(target_os = "linux")]
    let mut start_on_boot = app.settings.start_on_boot;
    #[cfg(target_os = "linux")]
    toggle_row(
        app,
        ui,
        "setting-start-on-boot",
        "Start on boot",
        "Launches Refryt automatically when you log in.",
        &mut start_on_boot,
    );

    let mut hide_tray = app.settings.hide_tray_on_close;
    // Wayland has no way to hide a window and show it again from code — only a real minimize,
    // and only in the hide direction (`app::hide_command`'s own doc comment has the specifics)
    // — so the promise made here has to be the one Wayland can actually keep, or the toggle
    // reads as broken the first time someone clicks the tray's Show and nothing happens.
    let hide_tray_detail = if crate::app::is_wayland() {
        "Closing the window minimizes it instead of quitting: Wayland has no way to hide and \
         reopen a window from an app, so use your taskbar to bring it back. The tray menu quits \
         for real."
    } else {
        "Closing the window hides it to a tray icon instead of quitting. The tray menu brings it back, or quits for real."
    };
    toggle_row(
        app,
        ui,
        "setting-hide-tray",
        "Hide tray on close",
        hide_tray_detail,
        &mut hide_tray,
    );

    let ctx = ui.ctx().clone();
    app.update_settings(&ctx, |s| {
        #[cfg(target_os = "linux")]
        {
            s.start_on_boot = start_on_boot;
        }
        s.hide_tray_on_close = hide_tray;
    });
}

/// Folders scanned for local audio files (mp3, wav, flac, and more), shown in the Library's
/// Local tab. Adding or removing a folder here is what triggers the rescan — see
/// `App::update_settings`'s own diff branch for `local_folders`.
fn local_music(app: &mut App, ui: &mut egui::Ui) {
    heading(app, ui, "Local Music");
    note(
        app,
        ui,
        "Folders scanned for audio files, shown in the Library's Local tab.",
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
        RichText::new("Search and playback work without an account; signing in adds your own content and your personalised home feed.")
            .font(theme::regular(12.0))
            .color(app.palette.dim),
    );
    ui.add_space(14.0);

    browser_import(app, ui);
}

/// The path that works: read the cookies out of a browser that is already signed in.
fn browser_import(app: &mut App, ui: &mut egui::Ui) {
    ui.label(
        RichText::new("Import from a browser")
            .font(theme::semibold(13.0))
            .color(app.palette.text),
    );
    ui.add_space(4.0);

    let browsers = app
        .detected_browsers
        .get_or_insert_with(browser_cookies::detected);
    if browsers.is_empty() {
        ui.label(
            RichText::new(
                "No browser profile found. Sign in to music.youtube.com in Firefox, Chrome, \
                 Chromium, Brave, Vivaldi or Edge.",
            )
            .font(theme::regular(12.0))
            .color(app.palette.dim),
        );
        return;
    }

    ui.label(
        RichText::new(
            "Reads the cookies from a browser you are already signed into. Nothing is launched \
             and no page is loaded; only the browser's cookie store is read.",
        )
        .font(theme::regular(12.0))
        .color(app.palette.dim),
    );
    // Windows locks a running Chromium browser's cookie database far more strictly than
    // Linux does; yt-dlp's own docs call closing the browser first the fix for exactly this,
    // and it is the single most common reason this import fails there. Worth saying up front
    // rather than only after a failed attempt, since the error it produces (a locked-file
    // message from deep inside yt-dlp) does not make the fix obvious on its own.
    #[cfg(windows)]
    ui.label(
        RichText::new("On Windows, close the browser first if the import below fails.")
            .font(theme::regular(12.0))
            .color(app.palette.dim),
    );
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
        app.auth = AuthState::Checking;
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
    ui.label(
        RichText::new(if account.is_empty() {
            "Signed in".to_owned()
        } else {
            format!("Signed in as {account}")
        })
        .font(theme::medium(13.0))
        .color(app.palette.text),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new("Your library is available from the sidebar, and Home is personalised.")
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
    note(
        app,
        ui,
        "Each channel has its own library and playlists. Switching reloads them.",
    );

    if let Some(page_id) = chosen {
        let ctx = ui.ctx().clone();
        app.update_settings(&ctx, |s| s.account_page_id = page_id);
        app.forget_account_data();
    }
}
