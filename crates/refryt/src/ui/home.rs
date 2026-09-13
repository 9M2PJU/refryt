//! Home: the feed YouTube Music would actually serve.
//!
//! This used to be a moods list and a recent-plays shelf we assembled ourselves, which meant none
//! of YouTube's ranking reached the page. It is now `FEmusic_home` verbatim — Listen again, Mixed
//! for you and Quick picks when signed in — with Explore's shelves appended while the anonymous
//! feed is thin.

use crate::app::{App, Route};
use crate::auth::AuthState;
use crate::theme;
use crate::ui::{failed, shelf};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    // No "Home" heading: the sidebar already says where you are.
    ui.add_space(8.0);

    if app.home_loading && app.home.is_none() {
        // The real feed is several titled shelves, mixing grids and row-lists — never one
        // untitled block. Which shelves are coming can't be known before they load, so this is
        // a reasonable approximation of the shape, not a prediction of the exact content.
        widgets::skeleton_shelf(
            ui,
            &app.palette,
            widgets::SkeletonShelfShape::Cards(widgets::ArtShape::Square),
            4,
        );
        widgets::skeleton_shelf(
            ui,
            &app.palette,
            widgets::SkeletonShelfShape::Cards(widgets::ArtShape::Square),
            4,
        );
        widgets::skeleton_shelf(ui, &app.palette, widgets::SkeletonShelfShape::Rows, 4);
        return;
    }
    if let Some(err) = app.home_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }

    crate::ui::page_scroll(app, "home").show(ui, |ui| {
        sign_in_note(app, ui);
        filter_chips(app, ui);

        // One snapshot for the frame — a refcount bump, not a copy of every shelf. `shelf`
        // needs `&mut App` for navigation and the texture cache, which rules out borrowing
        // the feed out of `app` across the call.
        let Some(shelves) = app.home.clone() else {
            return;
        };
        for s in shelves.iter() {
            shelf(app, ui, s);
        }
    });
}

/// The mood bar: the same feed filtered through Workout, Relax and friends.
///
/// Plain buttons like the Moods & genres chips, with the selected one highlighted. Clicking
/// the selected chip clears back to the default feed, as the site does.
fn filter_chips(app: &mut App, ui: &mut egui::Ui) {
    if app.home_chips.is_empty() {
        return;
    }
    // The clicked index, acted on after the loop — the pattern M4 put everywhere else. Cloning
    // every chip's strings up front was 22 allocations a frame in the render path.
    let palette = app.palette;
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for (i, chip) in app.home_chips.iter().enumerate() {
            if widgets::chip(ui, &palette, &chip.title, chip.selected).clicked() {
                picked = Some(i);
            }
        }
    });
    if let Some(chip) = picked.and_then(|i| app.home_chips.get(i)) {
        // Clicking the selected chip clears back to the default feed, as the site does.
        let filter = (!chip.selected).then(|| chip.params.clone());
        app.set_home_filter(filter);
    }
    ui.add_space(4.0);
}

/// Said once, quietly: the anonymous feed is two shelves, and a session is what unlocks the rest.
fn sign_in_note(app: &mut App, ui: &mut egui::Ui) {
    if matches!(app.auth, AuthState::SignedIn { .. }) {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(
                "You're browsing signed out. Signing in adds your own feed: \
                 Listen again, Mixed for you and Quick picks.",
            )
            .font(theme::regular(12.0))
            .color(app.palette.dim),
        );
    });
    ui.add_space(6.0);
    if widgets::pill_button(ui, &app.palette, "Sign in").clicked() {
        app.navigate(Route::Settings);
    }
    ui.add_space(4.0);
}
