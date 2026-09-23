//! A playlist page: header plus a numbered, playable track list.
//!
//! Same header language as the album page — large cover, Play / Save / ••• — with a short
//! description under the title and the owner linked into the meta line below it ("Owner •
//! N songs • duration • year"). The ••• also carries Edit and Delete for your own lists.

use crate::app::{App, PlaylistSort};
use crate::model::Playlist;
use crate::ui::{ListClick, PAGE_ART_SIZE, failed, track_rows_quiet};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    if app.playlist_loading {
        widgets::skeleton_header(
            ui,
            &app.palette,
            PAGE_ART_SIZE,
            widgets::SkeletonHeaderKind::Detail { description: true },
        );
        widgets::skeleton_track_rows(ui, &app.palette, 8, false);
        return;
    }
    if let Some(err) = app.playlist_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }
    if app.playlist.is_none() {
        return;
    }

    let wash = crate::ui::header_wash_colour(
        app,
        app.playlist.as_ref().and_then(|pl| pl.thumbnail.as_ref()),
    );
    crate::ui::page_scroll(app, "playlist").show(ui, |ui| {
        // The tint travels with the page: painted in scroll coordinates, it scrolls away
        // with the header instead of lingering as a background.
        crate::ui::header_wash(ui, &app.palette, app.content_rect, ui.cursor().min.y, wash);
        header(app, ui);
        crate::ui::info_popup(app, ui);
        // Breathing room between the header and the first row.
        ui.add_space(10.0);
        track_list(app, ui);
    });
}

/// The sort icon, painted right on the header's own Play/Save/••• baseline, just past the
/// last of them, rather than as a separate labelled row above the track list. Client-side
/// only — the playlist endpoints take no sort of their own, see [`PlaylistSort`]'s own doc
/// comment.
fn sort_control(app: &mut App, ui: &mut egui::Ui, anchor: egui::Pos2) {
    const OPTIONS: &[PlaylistSort] = &[
        PlaylistSort::Default,
        PlaylistSort::Reverse,
        PlaylistSort::TitleAsc,
        PlaylistSort::TitleDesc,
        PlaylistSort::DurationAsc,
        PlaylistSort::DurationDesc,
    ];
    let rect = egui::Rect::from_min_size(anchor + egui::vec2(10.0, -15.0), egui::vec2(260.0, 30.0));
    let mut cui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let labels: Vec<&'static str> = OPTIONS.iter().map(|s| s.label()).collect();
    if let Some(i) = widgets::icon_dropdown(
        &mut cui,
        &app.palette,
        ui.id().with("playlist-sort-dropdown"),
        crate::icons::Icon::ArrowUpDown,
        // Plain, at rest, either way — the popup's own tick is where "sorted" shows,
        // not a second indicator glued to the trigger icon.
        false,
        app.playlist_sort.label(),
        &labels,
    ) {
        app.sort_playlist(OPTIONS[i]);
    }
    if app.playlist_sort_pending {
        cui.add_space(6.0);
        // A running count, not a static label: a long playlist is dozens of continuation
        // requests chained one after another (each page's token comes from the last, so they
        // can't run in parallel), and a label that never changes for the ten-plus seconds that
        // takes reads as stuck even when it is working normally.
        let loaded = app.playlist.as_ref().map(|pl| pl.tracks.items.len());
        let text = match loaded {
            Some(n) => format!("Loading the rest to sort… ({n} loaded)"),
            None => "Loading the rest to sort…".to_owned(),
        };
        cui.label(
            egui::RichText::new(text)
                .font(crate::theme::regular(11.0))
                .color(app.palette.dim),
        );
    }
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let signed_in = app.signed_in();
    let playing = app
        .playlist
        .as_ref()
        .is_some_and(|pl| app.is_playing_any(&pl.tracks));

    // Read everything the header and its ••• menu need while `app.playlist` / `app.library`
    // are borrowed: the menu takes the real track slice, not a per-frame clone of a list that
    // is 5000 rows on Liked Music. Only the outcomes cross back out to act on `app`.
    let (out, menu_action, write, rail_ref, in_library, menu_description, is_expanding) = {
        let crate::app::App {
            palette,
            images,
            backend,
            playlist,
            library,
            ..
        } = &mut *app;
        let p = *palette;
        let Some(pl) = playlist.as_ref() else {
            return;
        };
        // The type line is the kind only; the year joins the meta line below, like the album.
        let kind_label = "Playlist".to_owned();
        let meta = meta_line(pl);
        // The owner, folded into the meta line below rather than a separate row: linked where
        // the page gave a channel id, so their profile can be opened from the name. Liked Music
        // is an auto list with no maker to show.
        let owner: Option<crate::model::ArtistLink> = (!crate::model::is_liked_auto(&pl.id)
            && !pl.author.trim().is_empty())
        .then(|| crate::model::ArtistLink {
            name: pl.author.trim().to_owned(),
            id: pl.author_id.clone(),
        });
        // The write the header offers: a feedback token where the bookmark toggle carries one,
        // else the like endpoint a public playlist's toggle points at — the same call that
        // saves an album. Your own header has no bookmark at all, and the typed fallback
        // cannot tell, so `saveable` keeps the control hidden there rather than writing blind.
        let token = (if pl.in_library {
            pl.library_remove.as_deref()
        } else {
            pl.library_add.as_deref()
        })
        .map(str::to_owned);
        let like =
            (signed_in && pl.saveable && !pl.editable && !crate::model::is_liked_auto(&pl.id))
                .then(|| crate::model::bare_playlist_id(&pl.id).to_owned());
        let (write, rail_ref) = match (token, like) {
            (Some(token), _) => (Some(crate::backend::PageWrite::Feedback(token)), None),
            (None, Some(id)) => (
                Some(crate::backend::PageWrite::LikePlaylist(id)),
                Some(crate::model::PlaylistRef {
                    id: pl.id.clone(),
                    title: pl.title.clone(),
                    author: pl.author.clone(),
                    author_id: pl.author_id.clone(),
                    track_count: pl.track_count.clone(),
                    thumbnail: pl.thumbnail.clone(),
                    editable: false,
                    source: pl.source,
                }),
            ),
            (None, None) => (None, None),
        };
        let in_library = pl.in_library;
        let art_top = ui.cursor().top();
        let out = crate::ui::page_header(
            ui,
            &p,
            images,
            backend,
            crate::ui::PageHeader {
                art_url: pl.thumbnail.as_ref(),
                art_size: PAGE_ART_SIZE,
                circle: false,
                kind_label: &kind_label,
                title: &pl.title,
                description: pl.description.as_deref(),
                artists: &[],
                second_plain: None,
                meta: Some(&meta),
                // The owner folds into the meta line as a linked name, so the card reads
                // "Owner • N songs • duration • year" on one line instead of a row each.
                owner: owner.as_ref(),
                credit_air: 4.0,
                play_size: Some(crate::ui::DETAIL_PLAY_PT),
                playing,
                save_in_library: signed_in
                    .then(|| write.is_some().then_some(in_library))
                    .flatten(),
                dots: true,
                salt: "playlist-author",
            },
        );
        // The header's own bottom: the art's, or the controls' when the row dropped below.
        ui.add_space(out.art_bottom - art_top + 12.0);

        // There is no real YouTube URL for a local playlist to share.
        let menu_share = if pl.source == crate::model::PlaylistSource::Local {
            String::new()
        } else {
            crate::model::playlist_url(&pl.id)
        };
        let menu_has_info = pl
            .description
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty());
        // A header-link click navigates away this frame; skip the menu then, as before.
        let menu_action = out
            .link
            .is_none()
            .then(|| {
                out.dots.as_ref().and_then(|dots| {
                    widgets::page_menu(
                        ui,
                        &p,
                        dots,
                        widgets::PageMenu {
                            page_id: &pl.id,
                            tracks: &pl.tracks.items,
                            artist_id: None,
                            playlists: &library.playlists.items,
                            signed_in,
                            share_url: menu_share,
                            has_info: menu_has_info,
                            manage: pl.editable,
                            remove_from_library: false,
                            context_only: false,
                            // A local playlist has no YouTube radio to start.
                            radio_id: matches!(pl.source, crate::model::PlaylistSource::YouTube)
                                .then_some(pl.id.as_str()),
                            save_as_new: None,
                        },
                    )
                })
            })
            .flatten();
        let menu_description = matches!(menu_action, Some(crate::ui::RowAction::PageInfo))
            .then(|| pl.description.clone())
            .flatten();
        let is_expanding = pl.is_expanding();
        (
            out,
            menu_action,
            write,
            rail_ref,
            in_library,
            menu_description,
            is_expanding,
        )
    };

    if out.play {
        let tracks = app
            .playlist
            .as_ref()
            .map(|pl| pl.tracks.items.clone())
            .unwrap_or_default();
        // Already on this playlist: the button just showed Pause, so clicking it again means
        // resume/pause, not "start over from track 1".
        if app.queue_matches(&tracks) {
            app.toggle_playback();
        } else {
            app.start_from(tracks, 0);
        }
    }
    // A mix/radio playlist has no fixed track list to sort — see `Playlist::is_expanding`'s
    // own doc comment — so the control simply doesn't render for one.
    if let Some(anchor) = out.controls_end
        && !is_expanding
    {
        sort_control(app, ui, anchor);
    }
    if out.save
        && let Some(write) = write
    {
        app.row_action(crate::ui::RowAction::PageLibrary {
            album: false,
            write,
            in_library: !in_library,
        });
        // The rail updates at once: a freshly saved list is "recently saved", which is where
        // YouTube itself puts it. A refetch here would race server propagation and can put the
        // old list right back; the next library load stays authoritative instead.
        if let Some(playlist) = rail_ref {
            app.splice_rail_playlist(playlist, !in_library);
        }
    }
    if let Some(route) = out.link {
        app.navigate(route);
        return;
    }
    match menu_action {
        Some(crate::ui::RowAction::PageInfo) => {
            app.page_info = menu_description;
        }
        Some(other) => app.row_action(other),
        None => {}
    }
}
/// Count, length and year: the type line above the title already names the kind, and the owner
/// now has a linked line of its own, so this is the stats alone.
fn meta_line(playlist: &Playlist) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(n) = &playlist.track_count {
        parts.push(n.clone());
    } else {
        parts.push(format!("{} songs", playlist.tracks.len()));
    }
    if let Some(d) = &playlist.duration_text {
        parts.push(d.clone());
    }
    if let Some(y) = &playlist.year {
        parts.push(y.clone());
    }
    parts.join(" • ")
}

fn track_list(app: &mut App, ui: &mut egui::Ui) {
    let signed_in = app.signed_in();
    let clicked = {
        let App {
            palette,
            images,
            backend,
            stats,
            now,
            playlist,
            library,
            ..
        } = app;
        let tracks: &[crate::model::Track] = playlist.as_ref().map(|p| &*p.tracks).unwrap_or(&[]);
        // "Remove from playlist" only on your own lists — never LM, never someone else's.
        let playlist_id = playlist
            .as_ref()
            .filter(|p| p.editable)
            .map(|p| p.id.as_str());
        // Liked Music is liked all the way down; its thumbs show only under the pointer.
        let liked_music = playlist
            .as_ref()
            .is_some_and(|p| crate::model::is_liked_auto(&p.id));
        track_rows_quiet(
            ui,
            palette,
            images,
            backend,
            stats,
            now,
            tracks,
            true,
            playlist_id,
            &library.playlists.items,
            signed_in,
            false,
            // Each track is from its own album, so the per-row cover stays.
            false,
            liked_music,
        )
    };
    // A long playlist arrives 100 tracks at a time; ask for the next when the viewport gets there.
    if clicked.at_end {
        app.load_more_playlist();
    }
    if let Some((from, to)) = clicked.reorder {
        app.reorder_playlist_track(from, to);
    }
    match clicked.click {
        Some(ListClick::Row(i)) => {
            if let Some(tracks) = app.playlist.as_ref().map(|p| p.tracks.items.clone()) {
                app.start_from(tracks, i);
            }
        }
        Some(ListClick::Link(route)) => app.navigate(route),
        None => {}
    }
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
}

/// The playlist management modals — rename and delete-confirm — painted over everything.
/// Called once from `App::ui`, so they show on any route.
pub(crate) fn modals(app: &mut App, ui: &mut egui::Ui) {
    edit_modal(app, ui);
    delete_modal(app, ui);
}

fn edit_modal(app: &mut App, ui: &mut egui::Ui) {
    let Some(mut editor) = app.playlist_edit.take() else {
        return;
    };
    let Some(screen) = ui.ctx().input(|i| i.raw.screen_rect) else {
        app.playlist_edit = Some(editor);
        return;
    };
    let p = app.palette;
    let (mut save, mut cancel, mut dismissed) = (false, false, false);
    egui::Area::new(egui::Id::new("ytm-edit-playlist-modal"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            let (full, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter()
                .rect_filled(full, 0.0, egui::Color32::from_black_alpha(150));
            let card = egui::Rect::from_center_size(
                full.center() - egui::vec2(0.0, 12.0),
                egui::vec2(360.0, 244.0),
            );
            // Only a click outside the card dismisses — a click in a blank gap between fields
            // inside the card used to fall through to this backdrop and discard the edit.
            if response.clicked()
                && response
                    .interact_pointer_pos()
                    .is_some_and(|pos| !card.contains(pos))
            {
                dismissed = true;
            }
            let painter = ui.painter();
            for (grow, alpha) in [(14.0, 24), (7.0, 40), (2.0, 60)] {
                painter.rect_filled(
                    card.expand(grow),
                    egui::CornerRadius::same(crate::theme::RADIUS + 2 + grow as u8),
                    egui::Color32::from_black_alpha(alpha),
                );
            }
            painter.rect_filled(
                card,
                egui::CornerRadius::same(crate::theme::RADIUS + 2),
                p.overlay,
            );

            let mut body = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card.shrink(16.0))
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            body.label(
                egui::RichText::new("Edit playlist")
                    .font(crate::theme::semibold(15.0))
                    .color(p.text),
            );
            body.add_space(8.0);
            body.label(
                egui::RichText::new("Title")
                    .font(crate::theme::regular(11.0))
                    .color(p.dim),
            );
            body.add(
                egui::TextEdit::singleline(&mut editor.title)
                    .id(egui::Id::new("ytm-edit-playlist-title"))
                    .hint_text(egui::RichText::new("Playlist name").color(p.dim))
                    .desired_width(f32::INFINITY)
                    .font(crate::theme::regular(13.0)),
            );
            body.add_space(8.0);
            body.label(
                egui::RichText::new("Description")
                    .font(crate::theme::regular(11.0))
                    .color(p.dim),
            );
            body.add(
                egui::TextEdit::multiline(&mut editor.description)
                    .id(egui::Id::new("ytm-edit-playlist-description"))
                    .hint_text(egui::RichText::new("Add a description").color(p.dim))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .font(crate::theme::regular(13.0)),
            );
            body.add_space(10.0);
            body.allocate_ui(egui::vec2(body.available_width(), 30.0), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::chip(ui, &p, "Save", true).clicked() {
                        save = true;
                    }
                    if widgets::chip(ui, &p, "Cancel", false).clicked() {
                        cancel = true;
                    }
                });
            });
            cancel = cancel || body.input(|i| i.key_pressed(egui::Key::Escape));
        });
    if save {
        app.playlist_edit = Some(editor);
        app.save_playlist_editor();
    } else if !(cancel || dismissed) {
        app.playlist_edit = Some(editor);
    }
}

fn delete_modal(app: &mut App, ui: &mut egui::Ui) {
    let Some((id, title)) = app.playlist_delete.clone() else {
        return;
    };
    let Some(screen) = ui.ctx().input(|i| i.raw.screen_rect) else {
        app.playlist_delete = Some((id, title));
        return;
    };
    let p = app.palette;
    let (mut delete, mut cancel, mut dismissed) = (false, false, false);
    egui::Area::new(egui::Id::new("ytm-delete-playlist-modal"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            let (full, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter()
                .rect_filled(full, 0.0, egui::Color32::from_black_alpha(150));
            if response.clicked() {
                dismissed = true;
            }
            let card = egui::Rect::from_center_size(
                full.center() - egui::vec2(0.0, 12.0),
                egui::vec2(340.0, 136.0),
            );
            let painter = ui.painter();
            for (grow, alpha) in [(14.0, 24), (7.0, 40), (2.0, 60)] {
                painter.rect_filled(
                    card.expand(grow),
                    egui::CornerRadius::same(crate::theme::RADIUS + 2 + grow as u8),
                    egui::Color32::from_black_alpha(alpha),
                );
            }
            painter.rect_filled(
                card,
                egui::CornerRadius::same(crate::theme::RADIUS + 2),
                p.overlay,
            );

            let mut body = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card.shrink(16.0))
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            body.label(
                egui::RichText::new("Delete playlist")
                    .font(crate::theme::semibold(15.0))
                    .color(p.text),
            );
            body.add_space(6.0);
            body.label(
                egui::RichText::new(format!("Delete \"{title}\"? This cannot be undone."))
                    .font(crate::theme::regular(12.0))
                    .color(p.secondary),
            );
            body.add_space(12.0);
            body.allocate_ui(egui::vec2(body.available_width(), 30.0), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::chip(ui, &p, "Delete", true).clicked() {
                        delete = true;
                    }
                    if widgets::chip(ui, &p, "Cancel", false).clicked() {
                        cancel = true;
                    }
                });
            });
            cancel = cancel || body.input(|i| i.key_pressed(egui::Key::Escape));
        });
    if delete {
        // `confirm_delete_playlist` takes the pending entry and fires the request.
        app.confirm_delete_playlist();
    } else if cancel || dismissed {
        app.playlist_delete = None;
    }
}
