//! "Liked Songs by <artist>": the songs of one artist that are in the liked index.
//!
//! The list comes from [`crate::liked::Index`] — a background page-through of Liked Music,
//! cached to disk. There is no liked-by-artist endpoint to fetch, so this page renders what the
//! index knows; `ensure_route_loaded` only has to make sure the right artist is loaded for the
//! name and portrait, which is also what Back/Forward and `--page artistliked:<id>` ride.
//!
//! Play leads, Shuffle sits beside it, and a sort dropdown trails both on the header's own
//! baseline — the playlist page's layout, because this is the same kind of page: one flat list
//! of tracks with a client-side order. Sorting needs no snapshot to get back to Default: the
//! index always holds the served order, and the rows are rebuilt from it every frame.

use crate::app::{App, PlaylistSort};
use crate::ui::{ListClick, PAGE_ART_SIZE, failed, track_rows};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    if app.artist_loading {
        widgets::skeleton_header(
            ui,
            &app.palette,
            PAGE_ART_SIZE,
            widgets::SkeletonHeaderKind::Artist,
        );
        widgets::skeleton_track_rows(ui, &app.palette, 6, false);
        return;
    }
    if let Some(err) = app.artist_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }
    let Some(id) = app.artist_id().map(str::to_owned) else {
        return;
    };
    let Some(name) = app.artist.as_ref().map(|a| a.name.clone()) else {
        return;
    };
    let tracks = sorted_tracks(app, &id);
    if tracks.is_empty() {
        let message = if app.liked_scanning {
            "Still reading your liked songs…".to_owned()
        } else {
            format!("No liked songs by {name}.")
        };
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(message)
                    .font(crate::theme::regular(13.0))
                    .color(app.palette.secondary),
            );
        });
        return;
    }

    crate::ui::page_scroll(app, "artist-liked").show(ui, |ui| {
        // The same tint language as the artist page, riding the artist's portrait.
        crate::ui::header_wash(
            ui,
            &app.palette,
            app.content_rect,
            ui.cursor().min.y,
            app.palette.accent,
        );
        header(app, ui, &tracks);
        ui.add_space(10.0);
        track_list(app, ui, &tracks);
    });
}

/// The artist's liked songs in the page's current order. `Default` is the index's own order —
/// recently saved first — and `Reverse` flips it; the rest re-sort by title or duration.
fn sorted_tracks(app: &App, id: &str) -> Vec<crate::model::Track> {
    let mut tracks = app.liked.tracks_for_artist(id);
    match app.artist_liked_sort {
        PlaylistSort::Reverse => tracks.reverse(),
        sort => sort.apply(&mut tracks),
    }
    tracks
}

fn header(app: &mut App, ui: &mut egui::Ui, tracks: &[crate::model::Track]) {
    let playing = app.is_playing_any(tracks);
    let out = {
        let crate::app::App {
            palette,
            images,
            backend,
            artist,
            ..
        } = app;
        let p = *palette;
        let Some(a) = artist.as_ref() else {
            return;
        };
        let title = format!("Liked Songs by {}", a.name);
        let meta = if tracks.len() == 1 {
            "1 song".to_owned()
        } else {
            format!("{} songs", tracks.len())
        };
        let art_top = ui.cursor().top();
        let out = crate::ui::page_header(
            ui,
            &p,
            images,
            backend,
            crate::ui::PageHeader {
                art_url: a.thumbnail.as_ref(),
                art_size: PAGE_ART_SIZE,
                circle: true,
                kind_label: "",
                title: &title,
                description: None,
                artists: &[],
                second_plain: Some(&meta),
                meta: None,
                owner: None,
                credit_air: 4.0,
                play_size: Some(crate::ui::DETAIL_PLAY_PT),
                playing,
                save_in_library: None,
                dots: false,
                salt: "artist-liked-name",
            },
        );
        ui.add_space(out.art_bottom - art_top + 12.0);
        out
    };
    // Shuffle and sort, on the header's own row just past Play. `controls_end` carries the 20 pt
    // the header leaves after Play for Save and •••, which this page has neither of; starting
    // there left Shuffle 24 px out from Play. Pulled back so Play, Shuffle and sort sit 12 px
    // apart, a group rather than three stragglers.
    let mut shuffled = false;
    if let Some(anchor) = out.controls_end {
        let rect =
            egui::Rect::from_min_size(anchor + egui::vec2(-8.0, -15.0), egui::vec2(300.0, 30.0));
        let mut cui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let p = app.palette;
        if widgets::chip(&mut cui, &p, "Shuffle", false).clicked() {
            shuffled = true;
        }
        // The chip's pill and the sort glyph each sit inside a larger hit box, which left 16 px of
        // air between them; pulled in so it matches Play → Shuffle.
        cui.add_space(-4.0);
        const OPTIONS: &[PlaylistSort] = &[
            PlaylistSort::Default,
            PlaylistSort::Reverse,
            PlaylistSort::TitleAsc,
            PlaylistSort::TitleDesc,
            PlaylistSort::DurationAsc,
            PlaylistSort::DurationDesc,
        ];
        let labels: Vec<&'static str> = OPTIONS.iter().map(|s| s.label()).collect();
        if let Some(i) = widgets::icon_dropdown(
            &mut cui,
            &p,
            ui.id().with("artist-liked-sort-dropdown"),
            crate::icons::Icon::ArrowUpDown,
            false,
            app.artist_liked_sort.label(),
            &labels,
        ) {
            app.artist_liked_sort = OPTIONS[i];
        }
    }
    if out.play {
        // Already on this list: the button showed Pause, so this is resume/pause, not a restart.
        if app.queue_matches(tracks) {
            app.toggle_playback();
        } else {
            app.start_from(tracks.to_vec(), 0);
        }
    }
    if shuffled {
        app.shuffle_play(tracks.to_vec());
    }
    if let Some(route) = out.link {
        app.navigate(route);
    }
}

fn track_list(app: &mut App, ui: &mut egui::Ui, tracks: &[crate::model::Track]) {
    let signed_in = app.signed_in();
    let clicked = {
        let App {
            palette,
            images,
            backend,
            stats,
            now,
            library,
            ..
        } = app;
        track_rows(
            ui,
            palette,
            images,
            backend,
            stats,
            now,
            tracks,
            true,
            None,
            &library.playlists.items,
            signed_in,
            false,
            // Songs from across the artist's albums, so each row keeps its own cover.
            false,
        )
    };
    match clicked.click {
        Some(ListClick::Row(i)) => app.start_from(tracks.to_vec(), i),
        Some(ListClick::Link(route)) => app.navigate(route),
        None => {}
    }
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
}
