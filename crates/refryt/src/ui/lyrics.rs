//! The lyrics panel: the now-playing song's words, synced to playback when we have timings.
//!
//! Shares the right panel with the queue — opening one closes the other — so it takes the same
//! width and insets the queue uses.

use crate::app::App;
use crate::theme;
use crate::widgets;

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette;
    // The card's own rect, before the header and list consume the ui: the corner repaint at
    // the end needs its rounded edges.
    let card = widgets::card_rect(ui.available_rect_before_wrap(), theme::CARD_INSET);
    widgets::clip_card_content(ui, card, theme::CARD_INSET_X, &p);
    ui.add_space(8.0);
    // The panel's own title, a hair right of the lyrics below it by eye — the same nudge the
    // queue's own header takes.
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("Lyrics")
                .font(theme::semibold(14.0))
                .color(p.text),
        );
    });
    ui.add_space(8.0);

    if app.lyrics_loading {
        crate::ui::centered(app, ui, "Loading lyrics…");
        return;
    }
    let Some(lyrics) = app.lyrics.as_ref() else {
        let message = app
            .lyrics_error
            .clone()
            .unwrap_or_else(|| "Play a song to see its lyrics".to_owned());
        crate::ui::centered(app, ui, &message);
        return;
    };

    // Which line is playing. Only synced lyrics know; plain text has no highlight.
    let position_ms = (app.player_state.position * 1000.0) as u64;
    let active = lyrics
        .synced
        .then(|| lyrics.lines.iter().rposition(|l| l.at_ms <= position_ms))
        .flatten();

    let mut area = egui::ScrollArea::vertical()
        .id_salt("lyrics")
        .auto_shrink([false, false]);
    if app.lyrics_scroll_reset {
        area = area.vertical_scroll_offset(0.0);
        app.lyrics_scroll_reset = false;
        // Land on the top of the panel, not wherever the active line happens to be — the
        // sync below then treats this frame's line as already seen, so the scroll-to-active
        // below doesn't immediately undo the reset.
        app.lyrics_active = active;
    }
    // Only scroll when the active line actually changed since last frame — not every repaint,
    // which used to snap the view back the instant a paused listener scrolled away to read
    // ahead. Since `active` only advances as `position_ms` crosses a new line, this is also
    // exactly "only while playback advances": paused, position never changes, so neither does
    // `active`.
    let scroll = active != app.lyrics_active;
    app.lyrics_active = active;

    area.show(ui, |ui| {
        for (i, line) in lyrics.lines.iter().enumerate() {
            let is_active = Some(i) == active;
            let text = egui::RichText::new(if line.text.is_empty() {
                "♪"
            } else {
                &line.text
            })
            .font(if is_active {
                theme::semibold(15.0)
            } else {
                theme::regular(14.0)
            })
            .color(if is_active { p.accent } else { p.secondary });
            let response = ui.add(egui::Label::new(text).wrap());
            if is_active && scroll {
                ui.scroll_to_rect(
                    response.rect.expand2(egui::vec2(0.0, 48.0)),
                    Some(egui::Align::Center),
                );
            }
            ui.add_space(4.0);
        }
    });
    // The card's rounded corners, over everything the lyrics drew: the list runs to the card's
    // bottom edge now.
    widgets::paint_corner_notches(ui, card, theme::CARD_INSET_X, &p);
}
