//! The signed-in user's library: songs, artists, albums, playlists — and Local, which needs
//! neither a sign-in nor a server round trip (see `local.rs`'s own doc comment).

use std::path::Path;

use crate::api::LibrarySection;
use crate::app::{App, LibrarySort, RailFilter, Route};
use crate::auth::AuthState;
use crate::theme;
use crate::ui::{
    CARD_ART_PT, CARD_W, ListClick, art_texture, card_is_visible, centered, failed, track_rows,
};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("Your library")
            .font(theme::semibold(18.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);

    crate::ui::page_scroll(app, "library").show(ui, |ui| {
        // One section at a time, as the site does. Stacking all five down one page is what left
        // the sidebar duplicating them, and it made every section a scroll away from the sort.
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for filter in RailFilter::ALL {
                if widgets::chip(ui, &app.palette, filter.label(), app.rail_filter == filter)
                    .clicked()
                {
                    picked = Some(filter);
                }
            }
        });
        if let Some(filter) = picked {
            app.rail_filter = filter;
        }
        ui.add_space(10.0);

        // Local needs no sign-in and no server round trip (see `local.rs`'s own doc comment),
        // so it has to be reachable before the auth check below — otherwise a signed-out user
        // could never see their own local files.
        if app.rail_filter == RailFilter::Local {
            local_folder_browser(app, ui);
            return;
        }
        // Downloads are on disk, so they need neither sign-in nor a server round trip either.
        if app.rail_filter == RailFilter::Downloads {
            downloads_list(app, ui);
            return;
        }

        match &app.auth {
            AuthState::SignedIn { .. } => content(app, ui),
            AuthState::Checking => centered(app, ui, "Checking your saved session…"),
            _ => prompt(app, ui),
        }
    });
}

/// Placeholders in the shape of whichever section is selected.
fn skeletons(app: &App, ui: &mut egui::Ui) {
    match app.rail_filter {
        RailFilter::Songs => widgets::skeleton_rows(ui, &app.palette, 8),
        // Artists are round here, as their cards are.
        RailFilter::Artists => {
            widgets::skeleton_cards(ui, &app.palette, 10, widgets::ArtShape::Circle)
        }
        RailFilter::Playlists | RailFilter::Albums => {
            widgets::skeleton_cards(ui, &app.palette, 10, widgets::ArtShape::Square)
        }
        // Local never shows this: it has its own loading state (`app.local_scanning`), since
        // it isn't gated by `library_loading`/`library_loaded` at all.
        RailFilter::Local | RailFilter::Downloads => widgets::skeleton_rows(ui, &app.palette, 8),
    }
}

fn prompt(app: &mut App, ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new("Sign in to see your library.")
            .font(theme::regular(13.0))
            .color(app.palette.secondary),
    );
    ui.add_space(12.0);
    if widgets::pill_button(ui, &app.palette, "Sign in").clicked() {
        app.navigate(Route::Settings);
    }
}

fn content(app: &mut App, ui: &mut egui::Ui) {
    // Skeletons are shaped like the section the chips are already showing, since Songs is a
    // list and the other three are grids: a page that loads as rows and lands as cards jumps.
    if app.library_loading {
        skeletons(app, ui);
        return;
    }
    if let Some(err) = app.library_error.clone() {
        failed(app, ui, err, App::retry_route);
        return;
    }
    if !app.library_loaded {
        // The safety net in App will have fired the fetch; skeletons, not nothing.
        skeletons(app, ui);
        return;
    }

    let empty = app.library.songs.is_empty()
        && app.library.artists.is_empty()
        && app.library.albums.is_empty()
        && app.library.playlists.is_empty();
    if empty {
        centered(
            app,
            ui,
            "Nothing here yet: songs, albums and playlists you save on YouTube Music \
             will show up here.",
        );
        return;
    }

    match app.rail_filter {
        RailFilter::Songs => {
            if app.library.songs.is_empty() {
                nothing_yet(app, ui, "No saved songs yet.");
            } else {
                sort_row(app, ui);
                song_list(app, ui);
            }
        }
        RailFilter::Artists => {
            if app.library.artists.is_empty() {
                nothing_yet(app, ui, "No saved artists yet.");
            } else {
                artist_grid(app, ui);
            }
        }
        RailFilter::Albums => {
            if app.library.albums.is_empty() {
                nothing_yet(app, ui, "No saved albums yet.");
            } else {
                album_grid(app, ui);
            }
        }
        RailFilter::Playlists => {
            if app.library.playlists.is_empty() {
                nothing_yet(app, ui, "No playlists yet.");
            } else {
                playlist_grid(app, ui);
            }
        }
        // `page()` never calls `content()` for `Local` — handled by `local_folder_browser`
        // before the auth check `content()` sits behind, since Local needs neither.
        RailFilter::Local => unreachable!("Local is intercepted in page() before content()"),
        RailFilter::Downloads => {
            unreachable!("Downloads is intercepted in page() before content()")
        }
    }
}

/// The Library's Downloads tab: the songs saved to `~/Music/refryt`, played from disk. A row's
/// own ••• menu offers Remove download (the menu reads the downloaded set published each frame).
fn downloads_list(app: &mut App, ui: &mut egui::Ui) {
    if app.downloads.entries.is_empty() {
        centered(
            app,
            ui,
            "Nothing downloaded yet. Use a song's ••• menu to save it for offline listening.",
        );
        return;
    }
    let tracks = app.downloads.tracks();
    let signed_in = app.signed_in();
    let out = {
        let App {
            palette,
            images,
            backend,
            now,
            library,
            stats,
            ..
        } = app;
        crate::ui::track_rows(
            ui,
            palette,
            images,
            backend,
            stats,
            now,
            &tracks,
            false,
            None,
            &library.playlists.items,
            signed_in,
            false,
        )
    };
    if let Some(action) = out.action {
        app.row_action(action);
    }
    match out.click {
        Some(crate::ui::ListClick::Row(i)) => app.start_from(tracks, i),
        Some(crate::ui::ListClick::Link(route)) => app.navigate(route),
        None => {}
    }
}

fn nothing_yet(app: &App, ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::regular(13.0))
            .color(app.palette.dim),
    );
}

/// Latest (server order) by default; the rest ask YouTube to resort the whole library. Same
/// plain-icon control as the playlist page's own sort, for the same reason: none of these
/// orders has a colour to show, so a swatch dot would only ever invent one.
fn sort_row(app: &mut App, ui: &mut egui::Ui) {
    const OPTIONS: &[LibrarySort] = &[
        LibrarySort::Latest,
        LibrarySort::NameAsc,
        LibrarySort::NameDesc,
    ];
    ui.horizontal(|ui| {
        let labels: Vec<&'static str> = OPTIONS.iter().map(|s| s.label()).collect();
        if let Some(i) = widgets::icon_dropdown(
            ui,
            &app.palette,
            ui.id().with("library-sort-dropdown"),
            crate::icons::Icon::ArrowUpDown,
            false,
            app.library_sort.label(),
            &labels,
        ) {
            app.sort_library(OPTIONS[i]);
        }
        if app.library_loading {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("…")
                    .font(theme::regular(11.0))
                    .color(app.palette.dim),
            );
        }
    });
    ui.add_space(4.0);
}

fn song_list(app: &mut App, ui: &mut egui::Ui) {
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
            &library.songs.items,
            false,
            None,
            &library.playlists.items,
            signed_in,
            false,
        )
    };
    // Ask for the next page once the viewport reaches the end of what we have. Nothing loads that
    // has not been scrolled to, so a 10,000-song library costs the same as a 25-song one until it
    // is looked at.
    if clicked.at_end {
        app.load_more_library(LibrarySection::Songs);
    }
    match clicked.click {
        Some(ListClick::Row(i)) => {
            let songs = app.library.songs.items.clone();
            app.start_from(songs, i);
        }
        Some(ListClick::Link(route)) => app.navigate(route),
        None => {}
    }
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
}

fn artist_grid(app: &mut App, ui: &mut egui::Ui) {
    let mut clicked = None;
    let mut last_visible = false;
    ui.horizontal_wrapped(|ui| {
        let count = app.library.artists.len();
        for (i, a) in app.library.artists.items.iter().enumerate() {
            let visible = card_is_visible(ui, CARD_W, widgets::ArtShape::Circle);
            // The last card being on screen is what "scrolled to the end" means for a grid.
            if visible && i + 1 == count {
                last_visible = true;
            }
            let art = visible
                .then(|| {
                    art_texture(
                        &mut app.images,
                        &app.backend,
                        ui.ctx(),
                        a.thumbnail.as_ref(),
                        CARD_ART_PT,
                    )
                })
                .flatten();
            if widgets::art_card(
                ui,
                &app.palette,
                CARD_W,
                art,
                &a.name,
                a.subscribers.as_deref().unwrap_or(""),
                // Round here too, so an artist looks the same wherever it appears.
                widgets::ArtShape::Circle,
                None,
                false,
                false,
            )
            .response
            .clicked()
            {
                clicked = Some(i);
            }
        }
    });
    if last_visible {
        app.load_more_library(LibrarySection::Artists);
    }
    if let Some(i) = clicked
        && let Some(artist) = app.library.artists.items.get(i)
    {
        let id = artist.id.clone();
        app.navigate(Route::Artist(id));
    }
}

fn album_grid(app: &mut App, ui: &mut egui::Ui) {
    let mut clicked = None;
    let mut played = None;
    let mut last_visible = false;
    ui.horizontal_wrapped(|ui| {
        let count = app.library.albums.len();
        for (i, al) in app.library.albums.items.iter().enumerate() {
            let visible = card_is_visible(ui, CARD_W, widgets::ArtShape::Square);
            // The last card being on screen is what "scrolled to the end" means for a grid.
            if visible && i + 1 == count {
                last_visible = true;
            }
            let art = visible
                .then(|| {
                    art_texture(
                        &mut app.images,
                        &app.backend,
                        ui.ctx(),
                        al.thumbnail.as_ref(),
                        CARD_ART_PT,
                    )
                })
                .flatten();
            let mut sub = al.artist.clone();
            if let Some(y) = &al.year {
                sub.push_str(" • ");
                sub.push_str(y);
            }
            let playing = app
                .now
                .track
                .as_ref()
                .is_some_and(|t| t.album_id.as_deref() == Some(al.id.as_str()))
                && !app.now.paused;
            let card = widgets::art_card(
                ui,
                &app.palette,
                CARD_W,
                art,
                &al.title,
                &sub,
                widgets::ArtShape::Square,
                None,
                playing,
                false,
            );
            // The corner button plays the album; a click anywhere else opens it.
            if card.play {
                played = Some(i);
            } else if card.response.clicked() {
                clicked = Some(i);
            }
        }
    });
    if last_visible {
        app.load_more_library(LibrarySection::Albums);
    }
    if let Some(i) = played
        && let Some(album) = app.library.albums.items.get(i)
    {
        let id = album.id.clone();
        app.play_album(id);
    }
    if let Some(i) = clicked
        && let Some(album) = app.library.albums.items.get(i)
    {
        let id = album.id.clone();
        app.navigate(Route::Album(id));
    }
}

fn playlist_grid(app: &mut App, ui: &mut egui::Ui) {
    let mut clicked = None;
    let mut played = None;
    let mut last_visible = false;
    ui.horizontal_wrapped(|ui| {
        let count = app.library.playlists.len();
        for (i, pl) in app.library.playlists.items.iter().enumerate() {
            let visible = card_is_visible(ui, CARD_W, widgets::ArtShape::Square);
            // The last card being on screen is what "scrolled to the end" means for a grid.
            if visible && i + 1 == count {
                last_visible = true;
            }
            let art = visible
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
            // Name only. These are all your own playlists, so the line underneath was either
            // your own account name or a track count — the same shape under all 25 cards, and
            // nothing you pick a playlist by.
            let card = widgets::art_card(
                ui,
                &app.palette,
                CARD_W,
                art,
                &pl.title,
                "",
                widgets::ArtShape::Square,
                None,
                false,
                false,
            );
            // The corner button plays the playlist; a click anywhere else opens it.
            if card.play {
                played = Some(i);
            } else if card.response.clicked() {
                clicked = Some(i);
            }
        }
    });
    if last_visible {
        app.load_more_library(LibrarySection::Playlists);
    }
    if let Some(i) = played
        && let Some(pl) = app.library.playlists.items.get(i)
    {
        let id = pl.id.clone();
        app.play_playlist(id);
    }
    if let Some(i) = clicked
        && let Some(pl) = app.library.playlists.items.get(i)
    {
        let id = pl.id.clone();
        app.navigate(Route::Playlist(id));
    }
}

/// The Local tab: a folder tree over `app.local_tracks`, browsed rather than paged — there's
/// nothing to page, the whole scan already sits in memory.
fn local_folder_browser(app: &mut App, ui: &mut egui::Ui) {
    if app.settings.local_folders.is_empty() {
        centered(
            app,
            ui,
            "No folders added yet. Add one in Settings to see your local music here.",
        );
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            if widgets::pill_button(ui, &app.palette, "Open Settings").clicked() {
                app.navigate(Route::Settings);
            }
        });
        return;
    }
    if app.local_scanning {
        widgets::skeleton_rows(ui, &app.palette, 8);
        return;
    }
    if let Some(err) = app.local_scan_error.clone() {
        failed(app, ui, err, |app| {
            app.local_scanning = true;
            let roots = app.settings.local_folders.clone();
            app.backend.scan_local_folders(roots);
        });
        return;
    }
    if app.local_tracks.is_empty() {
        centered(app, ui, "No audio files found in the configured folders.");
        return;
    }
    let Some(entries) = local_entries(app) else {
        centered(app, ui, "Could not resolve the configured folders.");
        return;
    };

    local_breadcrumb(app, ui);

    // `indices` is every track at or below the folder currently open — what plays, and what
    // "Save as playlist" saves, since opening a subfolder's own card already shows its tracks
    // and saving one archives it whole, the same as archiving a real nested folder would.
    // `direct` is only what sits right in this folder, with no subfolder in between — the flat
    // list below the cards shows only this, so a track never appears twice: once here and again
    // one click into whichever subfolder actually holds it.
    let depth = app.local_browse_path.len();
    let mut subfolders: Vec<String> = Vec::new();
    let mut indices: Vec<usize> = Vec::new();
    let mut direct: Vec<usize> = Vec::new();
    for (segs, i) in &entries {
        if segs.len() < depth || segs[..depth] != app.local_browse_path[..] {
            continue;
        }
        indices.push(*i);
        match segs.get(depth) {
            None => direct.push(*i),
            Some(name) if !subfolders.contains(name) => subfolders.push(name.clone()),
            Some(_) => {}
        }
    }
    subfolders.sort();

    save_as_playlist_row(app, ui, &indices);

    if !subfolders.is_empty() {
        local_folder_grid(app, ui, &entries, depth, &subfolders);
    }

    if direct.is_empty() {
        if subfolders.is_empty() {
            centered(app, ui, "Nothing directly in this folder.");
        }
        return;
    }
    local_track_list(app, ui, direct);
}

/// Every local track's folder-path segments (relative to whichever configured root it sits
/// under) paired with its position in `app.local_tracks`.
///
/// Cached against `(local_tracks_generation, local_folders)`: neither changes on most frames the
/// Local tab is open, and re-walking every track's path into freshly allocated `String` segments
/// every frame is a real cost once the local library holds any real number of files.
fn local_entries(app: &mut App) -> Option<crate::app::LocalEntries> {
    if app.settings.local_folders.is_empty() {
        return None;
    }
    let generation = app.local_tracks_generation;
    if let Some((cached_gen, cached_folders, entries)) = &app.local_entries_cache
        && *cached_gen == generation
        && cached_folders == &app.settings.local_folders
    {
        return Some(entries.clone());
    }

    let roots: Vec<std::path::PathBuf> = app
        .settings
        .local_folders
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    if roots.is_empty() {
        return None;
    }
    let entries: crate::app::LocalEntries = app
        .local_tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let path = t.local_path.as_ref()?;
            let root = roots.iter().find(|r| path.starts_with(r))?;
            let rel = path.strip_prefix(root).ok()?;
            let mut segs: Vec<String> = rel
                .iter()
                .map(|c| c.to_string_lossy().into_owned())
                .collect();
            // The filename itself is not a folder segment.
            segs.pop();
            Some((segs, i))
        })
        .collect();
    app.local_entries_cache = Some((
        generation,
        app.settings.local_folders.clone(),
        entries.clone(),
    ));
    Some(entries)
}

/// The current level's subfolders, as cards — a file-explorer view rather than a row of plain
/// buttons. Left click opens a card the same way a playlist card's click does; right click
/// offers "Save as playlist" (on top) plus the ordinary shuffle/queue/save controls, via
/// [`widgets::folder_card`]'s own [`widgets::PageMenu`] — so a folder can be turned into a real
/// playlist, or just kept as a browsable folder, without ever opening it first.
fn local_folder_grid(
    app: &mut App,
    ui: &mut egui::Ui,
    entries: &[(Vec<String>, usize)],
    depth: usize,
    names: &[String],
) {
    let signed_in = app.signed_in();
    let mut open = None;
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for name in names {
            // This subfolder's own tracks, at or below it — not the parent's — so the menu
            // acts on exactly what the card names, whether or not it's ever opened.
            let tracks: Vec<crate::model::Track> = entries
                .iter()
                .filter(|(segs, _)| segs.len() > depth && segs[depth] == *name)
                .filter_map(|(_, i)| app.local_tracks.get(*i).cloned())
                .collect();
            let menu = widgets::PageMenu {
                page_id: name.as_str(),
                tracks: &tracks,
                artist_id: None,
                playlists: &app.library.playlists.items,
                signed_in,
                share_url: String::new(),
                has_info: false,
                manage: false,
                remove_from_library: false,
                context_only: true,
                // A folder of local files has no YouTube radio id.
                radio_id: None,
                save_as_new: Some(name.as_str()),
            };
            let click = widgets::folder_card(ui, &app.palette, CARD_W, name, tracks.len(), menu);
            if click.response.clicked() {
                open = Some(name.clone());
            }
            if let Some(a) = click.action {
                action = Some(a);
            }
        }
    });
    ui.add_space(8.0);
    if let Some(name) = open {
        app.local_browse_path.push(name);
    }
    if let Some(action) = action {
        app.row_action(action);
    }
}

/// One button per level already drilled into, chevron-separated — clicking an earlier one
/// jumps straight back to it. No separate "All folders" root: nothing shows at the top level,
/// and getting back there one level at a time (via each ancestor's own name) is enough.
fn local_breadcrumb(app: &mut App, ui: &mut egui::Ui) {
    // Nothing to show at the top level — there's no ancestor to jump back to yet.
    if app.local_browse_path.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let mut jump_to = None;
        for (i, seg) in app.local_browse_path.iter().enumerate() {
            if i > 0 {
                ui.label(
                    egui::RichText::new("›")
                        .font(theme::regular(13.0))
                        .color(app.palette.dim),
                );
            }
            if widgets::pill_button(ui, &app.palette, seg).clicked() {
                jump_to = Some(i);
            }
        }
        // Clicking a crumb steps back out of it, to its parent — not into it, which is what
        // used to make clicking the deepest (often only) crumb a no-op and leave no way back
        // at all. Clicking the first crumb this way clears the whole path, so the top level is
        // always reachable one click from wherever you are, with no separate root button.
        if let Some(i) = jump_to {
            app.local_browse_path.truncate(i);
        }
    });
    ui.add_space(6.0);
}

/// Turns the folder currently open into a real, saved local playlist — every track at or below
/// it. Goes through the same new-playlist modal the sidebar's own **+** opens (see
/// [`crate::app::NewPlaylistKind`]) rather than a second dialog, since both are just "type a
/// name, confirm."
fn save_as_playlist_row(app: &mut App, ui: &mut egui::Ui, indices: &[usize]) {
    if indices.is_empty() {
        return;
    }
    let default_name = app
        .local_browse_path
        .last()
        .cloned()
        .unwrap_or_else(|| "Local files".to_owned());
    if widgets::pill_button(ui, &app.palette, "Save as playlist").clicked() {
        let track_paths = indices
            .iter()
            .filter_map(|&i| {
                app.local_tracks
                    .get(i)?
                    .local_path
                    .as_deref()
                    .map(Path::to_path_buf)
            })
            .collect();
        app.new_playlist = Some(default_name);
        app.new_playlist_kind = crate::app::NewPlaylistKind::Local { track_paths };
    }
    ui.add_space(8.0);
}

/// The tracks sitting directly in the current folder — not any subfolder's, which already get
/// their own listing one card-click away — through the same row renderer every other track
/// list in the app uses, so art, hover, truncation, the active-track highlight and the "Save
/// to playlist" menu all work exactly as they do everywhere else.
fn local_track_list(app: &mut App, ui: &mut egui::Ui, indices: Vec<usize>) {
    let signed_in = app.signed_in();
    let tracks: Vec<crate::model::Track> = indices
        .iter()
        .filter_map(|&i| app.local_tracks.get(i).cloned())
        .collect();
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
            &tracks,
            false,
            None,
            &library.playlists.items,
            signed_in,
            false,
        )
    };
    match clicked.click {
        Some(ListClick::Row(i)) => {
            app.start_from(tracks, i);
        }
        Some(ListClick::Link(route)) => app.navigate(route),
        None => {}
    }
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
}
