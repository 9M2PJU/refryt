//! The now-playing bar at the bottom: cover, title, transport, seek and volume.

use egui::{Rect, Sense, vec2};

use crate::app::{App, RepeatMode, Route};
use crate::icons::Icon;
use crate::model::LikeStatus;
use crate::theme;
use crate::ui::RowAction;
use crate::widgets;

/// The player bar's cover is drawn 56pt across and is always on screen.
const BAR_ART_PT: f32 = 56.0;
/// How wide the title and credits field is allowed to grow before the rating column stops
/// following it. Wide enough for a full title at 13pt, narrow enough that the thumbs stay
/// beside the credits rather than out towards the transport.
const FIELD_MAX_W: f32 = 320.0;
/// The transport's width: shuffle, previous, play, next, repeat. Centred on the window, and
/// shared with the title cap so the two never overlap.
const TRANSPORT_W: f32 = 250.0;
/// The right-hand control cluster, pinned to the window's edge: the 14pt inset, the volume
/// slider, Queue, Lyrics and the video toggle, with their separations. The seek bar's duration
/// label must clear this, or a narrow window paints the video toggle over the number.
const CONTROL_CLUSTER_W: f32 = 14.0 + 76.0 + 8.0 + 30.0 + 8.0 + 30.0 + 8.0 + 30.0;

/// The seek bar's width: grows with the window from the transport's own width up to 420pt,
/// and never so wide that the duration label hanging right of it reaches the control cluster.
///
/// It grows from the transport's width rather than as a flat third of the window: at a
/// mid-size window a third was wide enough that the elapsed label, which the now-playing text
/// has to clear, cut "The Weeknd • Kiss Land" down to "Kiss Lan" with room to spare in the
/// middle.
fn seek_width(width: f32, cluster_left: f32, total_w: f32, center_x: f32) -> f32 {
    // The label is painted 10pt past the bar's right end; the 6pt is the gap it must leave.
    let room = cluster_left - 6.0 - 10.0 - total_w - center_x;
    (width * 0.5 - 250.0)
        .clamp(TRANSPORT_W, 420.0)
        .min(2.0 * room.max(0.0))
}

pub fn player_bar(app: &mut App, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let p = app.palette;

    // No divider: the bar sits on the gutter with the cards above it, so the gap is the edge.

    let state = app.player_state;

    let art = Rect::from_min_size(
        rect.left_top() + vec2(14.0, (rect.height() - 56.0) / 2.0),
        vec2(56.0, 56.0),
    );
    let bar_art = crate::ui::art_texture(
        &mut app.images,
        &app.backend,
        ui.ctx(),
        app.now.track.as_ref().and_then(|t| t.thumbnail.as_ref()),
        BAR_ART_PT,
    );
    widgets::paint_art(ui, &p, art, bar_art, widgets::ArtShape::Square);

    // A status line replaces the credits while resolving or after a failure; otherwise the
    // credits are the same linked pieces a row draws, which is what makes *every* artist here
    // clickable rather than only the first.
    let (title, status) = match (&app.now.track, &app.now.error) {
        (Some(t), Some(e)) => (t.title.clone(), Some((e.clone(), p.danger))),
        (Some(t), None) if app.now.resolving => (
            t.title.clone(),
            Some(("Resolving…".to_owned(), p.secondary)),
        ),
        (Some(t), None) => (t.title.clone(), None),
        // Nothing playing, nothing said — not even a dash.
        (None, _) => (String::new(), None),
    };

    let title_x = art.right() + 12.0;
    // Rating in a fixed column at the end of the text field — not hugging the title
    // text, which dragged the thumbs across the credits on a short title. The title
    // and the credits share the field width from this column, so a short title leaves
    // the credits the whole room instead of almost none. No thumb width when signed
    // out: the thumbs do not draw, so the title keeps the room.
    let thumbs_w = if app.signed_in() { 72.0 } else { 0.0 };
    let transport_start = rect.center().x - TRANSPORT_W / 2.0;
    // The seek bar is wider than the transport, and its elapsed-time label hangs left of
    // the bar at subtitle height — the title has to clear the label, not just the buttons,
    // or long titles sit underneath it. The total stands in for the elapsed (at least as
    // wide, and stable while time advances), and the thumbs clear it too. The 24pt floor
    // keeps a degenerate window an ellipsis rather than the full title over everything.
    let total_w = widgets::text_width(
        ui,
        &widgets::fmt_duration(state.duration as u64),
        &theme::regular(11.0),
    );
    let bar_w = seek_width(
        rect.width(),
        rect.right() - CONTROL_CLUSTER_W,
        total_w,
        rect.center().x,
    );
    let bar_min_x = rect.center().x - bar_w / 2.0;
    // The field hugs its own content — the wider of the title and the credits — so the
    // rating column rides next to the text instead of at a fixed 320pt from the title, which
    // on a short song left the thumbs drifting in space and is the whole reason this line is
    // not a clamp. The credits are never shortened to get there: the field is at least the
    // credits' own width whenever the clearances allow. Past the cap (or squeezed by the
    // transport) the title and credits scroll on hover instead, which is what the marquee is
    // for; the 24pt floor keeps a degenerate window an ellipsis.
    let content_w = app
        .now
        .track
        .as_ref()
        .map(|t| widgets::subtitle_width(ui, &crate::ui::track_subtitle(t), None))
        .unwrap_or(0.0)
        .max(widgets::text_width(ui, &title, &theme::medium(13.0)));
    let text_w = (content_w + 8.0)
        .clamp(24.0, FIELD_MAX_W)
        .min((transport_start - thumbs_w - 8.0 - title_x).max(0.0))
        .min((bar_min_x - 10.0 - total_w - 22.0 - thumbs_w - title_x).max(0.0))
        .max(24.0);
    // The column, then the field the two lines share: title above, credits below.
    let thumbs_x = (title_x + text_w + 10.0)
        .min(transport_start - thumbs_w)
        .min(bar_min_x - 10.0 - total_w - 4.0 - thumbs_w);
    let field_w = (thumbs_x - 10.0 - title_x).max(24.0);
    // The title links to its album, and — with the cover — is the only place the song's
    // menu opens. `marquee_line` hands back its own click-sensing viewport when the title
    // overflows; a title that fits gets a hit rect the width of its text, so neither the
    // blank field nor the credits answer the pointer.
    let title_font = theme::medium(13.0);
    let title_pos = egui::pos2(title_x, rect.center().y - 9.0);
    let title_w = widgets::text_width(ui, &title, &title_font).min(field_w);
    let title_rect = Rect::from_min_size(
        egui::pos2(title_pos.x, title_pos.y - widgets::MARQUEE_HIT_H / 2.0),
        vec2(title_w, widgets::MARQUEE_HIT_H),
    );
    // A long title scrolls on hover inside the field rather than pushing anything away.
    let title_hit = widgets::marquee_line(
        ui,
        title_pos,
        field_w,
        &title,
        title_font,
        p.text,
        "np-title",
        Sense::click(),
        false,
    )
    .unwrap_or_else(|| ui.interact(title_rect, ui.id().with("np-title-hit"), Sense::click()));
    let art_hit = ui.interact(art, ui.id().with("np-art-hit"), Sense::click());
    if app.now.track.as_ref().is_some_and(|t| t.album_id.is_some()) && title_hit.hovered() {
        // The same hover treatment a row's artist link gets: a pointing hand and a hairline
        // under the text.
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().hline(
            title_pos.x..=(title_pos.x + title_w),
            title_pos.y + 7.0,
            egui::Stroke::new(1.0, p.text),
        );
    }
    let menu_response = app
        .now
        .track
        .as_ref()
        .map(|_| art_hit.union(title_hit.clone()));

    // The credits, through the same painter a row uses: one hit rect per artist, so every
    // artist is clickable and the album is a link too. The viewport is the field, not a
    // width derived from the title — a short song name no longer starves its own credits.
    // Scrolls on hover like the title.
    let baseline = egui::pos2(title_x, rect.center().y + 9.0);
    let mut navigate = None;
    match &status {
        Some((text, colour)) => {
            ui.painter().text(
                baseline,
                egui::Align2::LEFT_CENTER,
                widgets::truncate_to(ui, text, theme::regular(12.0), field_w),
                theme::regular(12.0),
                *colour,
            );
        }
        None => {
            if let Some(track) = app.now.track.as_ref()
                && field_w > 4.0
            {
                let subtitle = crate::ui::track_subtitle(track);
                if let Some(link) = widgets::paint_subtitle_marquee(
                    ui,
                    &p,
                    baseline,
                    field_w,
                    &subtitle,
                    None,
                    "np-subtitle",
                    false,
                    theme::regular(12.0),
                    p.secondary,
                ) {
                    navigate = Some(crate::ui::link_route(link));
                }
            }
        }
    }
    if let Some(route) = navigate {
        app.navigate(route);
    }
    // A primary click on the title opens its album or single; the menu is the right-click.
    if title_hit.clicked()
        && let Some(id) = app.now.track.as_ref().and_then(|t| t.album_id.clone())
    {
        app.navigate(Route::Album(id));
    }

    // Borrow the track to read `like`; the whole `Track` was being deep-cloned every frame
    // just for this. `video_id` is only cloned on an actual click.
    let mut rate: Option<RowAction> = None;
    if app.signed_in()
        && let Some(track) = app.now.track.as_ref()
    {
        let thumbs = Rect::from_min_size(
            egui::pos2(thumbs_x, rect.center().y - 16.0),
            vec2(thumbs_w, 32.0),
        );
        let mut tui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(thumbs)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        for (up, pressed) in [(true, LikeStatus::Like), (false, LikeStatus::Dislike)] {
            let (hit, _) = tui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            if widgets::rate_button(
                &mut tui,
                &p,
                hit,
                egui::Id::new(("ytm-bar-thumb", up)),
                up,
                track.like == pressed,
                true,
            )
            .clicked()
            {
                rate = Some(RowAction::Rate {
                    video_id: track.video_id.clone(),
                    status: track.like.toggled(pressed),
                });
            }
        }
    }
    if let Some(action) = rate {
        app.row_action(action);
    }

    // Transport, centred on the window so it does not shift with the track title: shuffle and
    // repeat flank previous/play/next, the way the reference lays them out.
    let controls = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.center().y - 12.0),
        vec2(TRANSPORT_W, 40.0),
    );
    let mut cui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(controls)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );

    if widgets::icon_button(&mut cui, &p, Icon::Shuffle, 16.0, app.shuffle).clicked() {
        app.toggle_shuffle();
    }
    let mut skip_delta = 0isize;
    if widgets::icon_button(&mut cui, &p, Icon::SkipBackFilled, 16.0, false).clicked() {
        skip_delta = -1;
    }
    let playing = app.playback_started && !state.paused;
    if widgets::play_button(&mut cui, &p, playing, 40.0).clicked() {
        app.toggle_playback();
    }
    if widgets::icon_button(&mut cui, &p, Icon::SkipForwardFilled, 16.0, false).clicked() {
        skip_delta = 1;
    }
    let repeat_icon = if app.repeat == RepeatMode::One {
        Icon::Repeat1
    } else {
        Icon::Repeat
    };
    if widgets::icon_button(
        &mut cui,
        &p,
        repeat_icon,
        16.0,
        app.repeat != RepeatMode::Off,
    )
    .clicked()
    {
        app.cycle_repeat();
    }
    if skip_delta != 0 {
        app.skip(skip_delta);
    }

    // Seek bar, click or drag to scrub. `bar_w` is shared with the title cap above, so
    // the elapsed-time label the bar paints can never sit under the title.
    let bar = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.center().y + 16.0),
        vec2(bar_w, 4.0),
    );
    // A restored, not-yet-loaded track still shows its saved position and length: mpv has no
    // state until play, so `resume` stands in, and scrubbing updates it rather than the player.
    let restored = (!app.playback_started)
        .then_some(app.resume)
        .flatten()
        .filter(|(i, _)| Some(*i) == app.current);
    let position = restored.map(|(_, p)| p).unwrap_or(state.position);
    let duration = if state.duration > 0.0 {
        state.duration
    } else {
        app.now
            .track
            .as_ref()
            .and_then(|t| t.duration)
            .map(|d| d as f64)
            .unwrap_or(0.0)
    };
    let fraction = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if let Some(seek_to) = widgets::seek_bar(ui, &p, bar, fraction as f32, duration, position) {
        if app.playback_started {
            if let Some(pl) = &app.player {
                pl.seek(seek_to);
            }
        } else if let Some(index) = app.current {
            // Scrubbing before play just moves the restored position.
            app.resume = Some((index, seek_to));
        }
    }

    // Volume. The child Ui spans the whole cluster, not just the slider: it packs from the
    // right, and a narrower rect would only hide the room the buttons already use.
    let vol_rect = Rect::from_min_max(
        egui::pos2(rect.right() - CONTROL_CLUSTER_W, rect.top()),
        rect.right_bottom(),
    );
    let mut vui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(vol_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    // `vui` packs right-to-left, so whatever is added first ends up closest to the window's
    // edge: the volume slider goes first to sit rightmost, with Queue and Lyrics to its left.
    vui.add_space(14.0);
    // The user's volume, not `player.volume()`: a crossfade may be holding the live value down.
    let volume = if app.muted { 0.0 } else { app.last_volume };
    if let Some(v) = widgets::volume_slider(&mut vui, &p, volume, app.last_volume)
        && let Some(pl) = &mut app.player
    {
        app.last_volume = v;
        app.muted = v <= 0.0;
        pl.set_volume(v);
    }
    vui.add_space(8.0);
    if widgets::icon_button(&mut vui, &p, Icon::ListMusic, 16.0, app.upnext_open).clicked() {
        app.upnext_open = !app.upnext_open;
    }
    vui.add_space(8.0);
    if widgets::icon_button(&mut vui, &p, Icon::MicVocal, 16.0, app.lyrics_open).clicked() {
        app.toggle_lyrics();
    }
    // The video view, further left still, and only when the track actually has one.
    if app.now.track.as_ref().is_some_and(|t| t.wide) {
        vui.add_space(8.0);
        if widgets::icon_button(&mut vui, &p, Icon::Video, 16.0, app.video_open).clicked() {
            app.toggle_video();
        }
    }

    // The now-playing song's menu, last: painting is done, and the choices route through the
    // same handler every row's menu uses.
    if let Some(menu) = menu_response
        && let Some(track) = app.now.track.as_ref()
    {
        let action = widgets::row_menu(
            ui,
            &p,
            &menu,
            widgets::RowMenu {
                track,
                playlist_id: None,
                playlists: &app.library.playlists.items,
                signed_in: app.signed_in(),
                queue_index: None,
            },
        );
        if let Some(action) = action {
            app.row_action(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wide window keeps the bar's designed width; a narrow one shrinks it so the duration
    /// label stops short of the control cluster, where the video toggle used to sit on it.
    #[test]
    fn the_seek_bar_shrinks_before_its_label_reaches_the_controls() {
        // 1100pt window: half less 250 is 300, under the cap and the cluster.
        let wide = seek_width(1100.0, 1100.0 - 14.0 - CONTROL_CLUSTER_W, 30.0, 550.0);
        assert!((wide - 300.0).abs() < 0.01, "{wide}");
        // A maximised 1920pt window reaches the cap.
        let full = seek_width(1920.0, 1920.0 - 14.0 - CONTROL_CLUSTER_W, 30.0, 960.0);
        assert!((full - 420.0).abs() < 0.01, "{full}");

        // 760pt window with a long duration: the cluster is the binding limit.
        let narrow = seek_width(760.0, 760.0 - 14.0 - CONTROL_CLUSTER_W, 45.0, 380.0);
        assert!(narrow < TRANSPORT_W, "{narrow}");
        // The label's right edge — half the bar, the 10pt offset and its own width — clears
        // the cluster with the 6pt gap intact.
        let label_right = 380.0 + narrow / 2.0 + 10.0 + 45.0;
        let cluster_left = 760.0 - 14.0 - CONTROL_CLUSTER_W;
        assert!(label_right <= cluster_left - 6.0 + 0.01, "{label_right}");

        // A window narrower than the controls can afford: the bar collapses rather than
        // painting under them.
        assert_eq!(
            seek_width(400.0, 400.0 - 14.0 - CONTROL_CLUSTER_W, 30.0, 200.0),
            0.0
        );
    }
}
