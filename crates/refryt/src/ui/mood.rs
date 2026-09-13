//! One mood's playlists, reached from the home page's Moods & genres grid.

use crate::app::{App, Route};
use crate::theme;
use crate::ui::{CARD_ART_PT, CARD_W, art_texture, card_is_visible, failed, section};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    if app.mood_playlists_loading {
        widgets::skeleton_title(ui, &app.palette);
        widgets::skeleton_cards(ui, &app.palette, 8, widgets::ArtShape::Square);
        return;
    }
    if let Some(err) = app.mood_playlists_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }
    let Some(mood) = app.mood_playlists.as_ref() else {
        return;
    };

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(&mood.title)
            .font(theme::semibold(18.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);

    crate::ui::page_scroll(app, "mood").show(ui, |ui| {
        section(ui, &app.palette, "Playlists");
        let mut clicked = None;
        ui.horizontal_wrapped(|ui| {
            for (i, pl) in app
                .mood_playlists
                .as_ref()
                .map(|m| m.playlists.as_slice())
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let art = card_is_visible(ui, CARD_W, widgets::ArtShape::Square)
                    .then(|| {
                        art_texture(
                            &mut app.images,
                            &app.backend,
                            ui.ctx(),
                            pl.thumbnail.as_ref(),
                            CARD_ART_PT,
                        )
                    })
                    .flatten();
                if widgets::art_card(
                    ui,
                    &app.palette,
                    CARD_W,
                    art,
                    &pl.title,
                    &pl.author,
                    widgets::ArtShape::Square,
                    None,
                )
                .response
                .clicked()
                {
                    clicked = Some(i);
                }
            }
        });
        if let Some(i) = clicked {
            let id = app
                .mood_playlists
                .as_ref()
                .and_then(|m| m.playlists.get(i))
                .map(|pl| pl.id.clone());
            if let Some(id) = id {
                app.navigate(Route::Playlist(id));
            }
        }
    });
}
