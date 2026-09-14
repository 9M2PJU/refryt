//! What you played, in the dated shelves YouTube serves — Today, This week, and so on.
//!
//! Rendered as plain rows regardless of entry kind, never the card grid the generic shelf
//! renderer would otherwise pick for a shelf that isn't all songs — a whole album or single you
//! played shows up as a shelf item here too, and History reads better as one consistent list of
//! rows than as a row/card mix that changes shape by what happened to be played that day.

use crate::app::{App, Route};
use crate::auth::AuthState;
use crate::theme;
use crate::ui::{centered, entry_rows, failed, section};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("History")
            .font(theme::semibold(18.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);

    match &app.auth {
        AuthState::SignedIn { .. } => content(app, ui),
        AuthState::Checking => {
            widgets::skeleton_shelf(ui, &app.palette, widgets::SkeletonShelfShape::Rows, 4);
            widgets::skeleton_shelf(ui, &app.palette, widgets::SkeletonShelfShape::Rows, 4);
        }
        _ => {
            ui.label(
                egui::RichText::new("Sign in to see what you played.")
                    .font(theme::regular(13.0))
                    .color(app.palette.secondary),
            );
            ui.add_space(12.0);
            if widgets::pill_button(ui, &app.palette, "Sign in").clicked() {
                app.navigate(Route::Settings);
            }
        }
    }
}

fn content(app: &mut App, ui: &mut egui::Ui) {
    if app.history_loading && app.history_shelves.is_none() {
        // Dated shelves (Today, Yesterday, ...) are always track lists — several titled row
        // blocks, not one flat 8-row list with no shelf boundaries.
        widgets::skeleton_shelf(ui, &app.palette, widgets::SkeletonShelfShape::Rows, 4);
        widgets::skeleton_shelf(ui, &app.palette, widgets::SkeletonShelfShape::Rows, 4);
        return;
    }
    if let Some(err) = app.history_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }

    // One snapshot for the frame: `shelf` needs `&mut App`, which rules out borrowing the
    // shelves out of `app` across the call.
    let Some(shelves) = app.history_shelves.clone() else {
        return;
    };
    // Legitimately empty — a fresh account, paused watch history, or just nothing played yet
    // — is not an error (see `Api::history`'s own doc comment), so it gets a plain message
    // rather than nothing at all.
    if shelves.is_empty() {
        centered(app, ui, "Nothing played yet.");
        return;
    }
    crate::ui::page_scroll(app, "history").show(ui, |ui| {
        for s in shelves.iter() {
            if s.items.is_empty() {
                continue;
            }
            section(ui, &app.palette, &s.title);
            entry_rows(app, ui, &s.items);
        }
    });
}
