//! The queue: what is playing, and what follows.
//!
//! Reuses `spec_rows` so virtualisation, links and art budgets apply unchanged.
//!
//! No rating thumbs here (`show_rating: false`). This is a list of what happens next, and a
//! column of controls repeated down it is noise; rating a queued song is one right-click away.

use crate::app::App;
use crate::theme;
use crate::ui::{ListClick, RowSpec, centered, is_active, spec_rows, track_subtitle_no_album};
use crate::widgets::{self, ArtShape};

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let start = app.current.map(|i| i + 1).unwrap_or(0);
    if start >= app.queue.len() {
        centered(app, ui, "Empty queue");
        return;
    }

    let mut area = egui::ScrollArea::vertical()
        .id_salt("upnext")
        .auto_shrink([false, false]);
    if app.queue_scroll_reset {
        area = area.vertical_scroll_offset(0.0);
        app.queue_scroll_reset = false;
    }
    area.show(ui, |ui| {
        now_playing(app, ui);
        section(app, ui, 8.0, "Next up");
        let signed_in = app.signed_in();
        let clicked = {
            let App {
                palette,
                images,
                backend,
                stats,
                now,
                queue,
                library,
                ..
            } = app;
            spec_rows(
                ui,
                palette,
                images,
                backend,
                stats,
                queue.len() - start,
                |i| {
                    let t = &queue[start + i];
                    // No album names: this is what happens next, not where each song is
                    // from.
                    let subtitle = track_subtitle_no_album(t);
                    RowSpec {
                        number: None,
                        rank: t.rank.as_deref(),
                        trend: t.trend,
                        title: &t.title,
                        album_id: t.album_id.as_deref(),
                        subtitle,
                        duration: t.duration,
                        plays: t.plays.as_deref(),
                        explicit: t.explicit,
                        active: is_active(now, &t.video_id),
                        playing: is_active(now, &t.video_id) && !now.paused,
                        badge: None,
                        shape: if t.wide {
                            ArtShape::Wide
                        } else {
                            ArtShape::Square
                        },
                        thumbnail: t.thumbnail.as_ref(),
                        menu: Some(widgets::RowMenu {
                            track: t,
                            playlist_id: None,
                            playlists: &library.playlists.items,
                            signed_in,
                            queue_index: Some(start + i),
                        }),
                        like: t.like,
                        // The queue is a list of what happens next, not a place to
                        // rate things: a column of thumbs repeated down it is noise.
                        // Rating a queued song is one click away on the player bar.
                        show_rating: false,
                    }
                },
            )
        };

        // Forwarded before the click: a menu choice arrives with no row click, and the
        // `None` arm below returns early.
        if let Some(action) = clicked.action {
            app.row_action(action);
        }
        let i = match clicked.click {
            Some(ListClick::Row(i)) => start + i,
            Some(ListClick::Link(route)) => {
                app.navigate(route);
                return;
            }
            None => return,
        };
        // Jumping sets `current`; the `Playing` event's `prefetch_next` follows, same as
        // every other way of moving through the queue. Clicking the row already playing just
        // pauses/resumes it (see `play_index`) — nothing new to show, so the scroll position
        // is left alone rather than jumping to the top under you.
        let already_here = app.current == Some(i);
        app.play_index(i);
        if !already_here {
            app.queue_scroll_reset = true;
        }
    });
}

/// A small caps-ish section label, as the reference uses above each block. `gap` is the space
/// above it: sections that follow a row carry 8, the first block under the panel's top carries
/// none — the queue is dismissed from the player bar's list button, so there is no header row
/// to clear.
fn section(app: &App, ui: &mut egui::Ui, gap: f32, label: &str) {
    ui.add_space(gap);
    ui.label(
        egui::RichText::new(label)
            .font(theme::semibold(12.0))
            .color(app.palette.secondary),
    );
    ui.add_space(4.0);
}

/// The track the transport is on, above the list of what follows it.
fn now_playing(app: &mut App, ui: &mut egui::Ui) {
    let Some(i) = app.current else { return };
    let Some(track) = app.queue.get(i).cloned() else {
        return;
    };
    section(app, ui, 0.0, "Now playing");
    // Namespaced under its own id: this row and "Next up"'s first row both start their own
    // `spec_rows` call at index 0, so without this their title/artist links resolve to the
    // exact same widget id (`ui.id()` is the same parent for both — they're sequential calls on
    // the same `Ui`, not separately salted children) and neither one's link reliably works.
    let clicked = ui
        .push_id("now-playing", |ui| {
            let App {
                palette,
                images,
                backend,
                stats,
                now,
                ..
            } = app;
            let subtitle = track_subtitle_no_album(&track);
            spec_rows(ui, palette, images, backend, stats, 1, |_| RowSpec {
                number: None,
                rank: track.rank.as_deref(),
                trend: track.trend,
                title: &track.title,
                album_id: track.album_id.as_deref(),
                subtitle,
                duration: track.duration,
                plays: None,
                explicit: track.explicit,
                active: is_active(now, &track.video_id),
                playing: is_active(now, &track.video_id) && !now.paused,
                badge: None,
                shape: if track.wide {
                    ArtShape::Wide
                } else {
                    ArtShape::Square
                },
                thumbnail: track.thumbnail.as_ref(),
                // No menu on the current track: everything it offers is already in the player
                // bar, and a menu here would be the only row in the panel that could remove
                // itself.
                menu: None,
                like: track.like,
                show_rating: false,
            })
        })
        .inner;
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
    match clicked.click {
        Some(ListClick::Link(route)) => app.navigate(route),
        // Elsewhere on the row: it's already what's playing, so a click means pause/resume,
        // same as clicking it anywhere else it appears.
        Some(ListClick::Row(_)) => app.toggle_playback(),
        None => {}
    }
}
