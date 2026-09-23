//! One mood's playlists, reached from the home page's Moods & genres grid.

use crate::app::{App, Route};
use crate::ui::{art_texture, card_is_visible, failed, section};
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

    crate::ui::page_title(ui, &app.palette, &mood.title);

    crate::ui::page_scroll(app, "mood").show(ui, |ui| {
        section(ui, &app.palette, "Playlists");
        let mut clicked = None;
        let mut played = None;
        ui.horizontal_wrapped(|ui| {
            let card_w = crate::ui::grid_card_w(ui);
            for (i, pl) in app
                .mood_playlists
                .as_ref()
                .map(|m| m.playlists.as_slice())
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let art = card_is_visible(ui, card_w, widgets::ArtShape::Square)
                    .then(|| {
                        art_texture(
                            &mut app.images,
                            &app.backend,
                            ui.ctx(),
                            pl.thumbnail.as_ref(),
                            card_w - 16.0,
                        )
                    })
                    .flatten();
                let card = widgets::art_card(
                    ui,
                    &app.palette,
                    card_w,
                    art,
                    &pl.title,
                    &pl.author,
                    widgets::ArtShape::Square,
                    None,
                    false,
                    false,
                );
                // The corner play button wins over the card click, the same contract every
                // other card grid follows: play the playlist, don't open it.
                if card.play {
                    played = Some(i);
                } else if card.response.clicked() {
                    clicked = Some(i);
                }
            }
        });
        if let Some(i) = played {
            let id = app
                .mood_playlists
                .as_ref()
                .and_then(|m| m.playlists.get(i))
                .map(|pl| pl.id.clone());
            if let Some(id) = id {
                app.play_playlist(id);
            }
        }
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
