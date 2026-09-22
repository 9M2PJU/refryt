//! New releases, behind Explore's first button: new albums & singles.
//!
//! The same shelf shape as every other feed, so shelves render through the carousel —
//! sideways scrolling is what makes this page the reference's, not the wrapped grids
//! everywhere else uses.

use crate::app::App;
use crate::theme;
use crate::ui::{failed, shelf_carousel};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("New releases")
            .font(theme::semibold(18.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);

    if app.releases_loading && app.releases.is_none() {
        // Always albums & singles — card shelves, never a track-list one, unlike Home/Explore.
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
        return;
    }
    if let Some(err) = app.releases_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }

    crate::ui::page_scroll(app, "releases").show(ui, |ui| {
        // One snapshot for the frame: the carousel needs `&mut App`, which rules out
        // borrowing the feed out of `app` across the call.
        let Some(shelves) = app.releases.clone() else {
            return;
        };
        for s in shelves.iter() {
            shelf_carousel(app, ui, "releases", s, None);
        }
    });
}
