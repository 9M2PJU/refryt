//! The search results page: two tiers, matching [`SearchScope`]'s own doc comment.
//!
//! `All` renders the one ranked cross-type response as a single list — the unfiltered response
//! ranks every type against every other, and splitting it up throws that away, which is how
//! `travis` used to surface the Scottish band ahead of Travis Scott. Every other scope instead
//! renders `app.scoped_results`, a dedicated per-category request: the mixed response often
//! carries far fewer of a given kind than actually exist (confirmed live — twelve real artist
//! matches for "the weeknd" against one in the mixed response), so narrowing to a scope re-asks
//! YouTube for that category specifically rather than filtering the same short list down further.
//!
//! There is no separate "top result" card in the `All` view. Every row is the same height, and
//! the top result is simply the first one — see [`crate::innertube::parse_search`] for why
//! YouTube's own card is folded into the ranking rather than shown separately.

use crate::app::{App, Route};
use crate::model::{Entry, SearchScope};
use crate::theme;
use crate::ui::{ListClick, centered, spec_rows};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(6.0);

    let heading = if app.searching {
        format!("Searching “{}”…", app.searched)
    } else if app.searched.is_empty() {
        "Search YouTube Music".to_owned()
    } else {
        format!("Results for “{}”", app.searched)
    };
    ui.label(
        egui::RichText::new(heading)
            .font(theme::semibold(18.0))
            .color(app.palette.text),
    );
    ui.add_space(10.0);

    if let Some(status) = &app.status {
        centered(app, ui, status);
        return;
    }
    if app.results.is_empty() {
        if app.searching {
            widgets::skeleton_rows(ui, &app.palette, 8);
        } else {
            centered(app, ui, "Type in the search box above. No sign-in needed.");
        }
        return;
    }

    scope_chips(app, ui);

    if app.search_scope == SearchScope::All {
        if !app
            .results
            .items
            .iter()
            .any(|e| app.search_scope.matches(e))
        {
            centered(app, ui, "Nothing of that kind in these results.");
            return;
        }
    } else if app.scoped_loading {
        widgets::skeleton_rows(ui, &app.palette, 8);
        return;
    } else if let Some(error) = app.scoped_error.clone() {
        centered(app, ui, &error);
        return;
    } else if app.scoped_results.is_empty() {
        centered(app, ui, "Nothing of that kind found.");
        return;
    }

    crate::ui::page_scroll(app, "search").show(ui, |ui| {
        results_list(app, ui);
    });
}

/// Picks which tier of the page renders: `All` narrows the same ranked list down to one kind
/// (never a second request); every other scope triggers (or reuses) a dedicated per-category
/// fetch — see [`SearchScope`]'s own doc comment for why. Chips rather than a dropdown, matching
/// the rail's own filter row (`library.rs`) — a handful of mutually-exclusive choices read
/// fastest as a row you can see all of at once.
fn scope_chips(app: &mut App, ui: &mut egui::Ui) {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for scope in SearchScope::ALL {
            if widgets::chip(ui, &app.palette, scope.label(), app.search_scope == scope).clicked() {
                picked = Some(scope);
            }
        }
    });
    if let Some(scope) = picked {
        app.search_scope = scope;
        app.fetch_scoped_search(scope);
    }
    ui.add_space(4.0);
}

/// The list. Every kind renders through the same row, so art, hover, truncation and the
/// active-track highlight are shared, and the list virtualises like every other.
///
/// `indices` is `All`'s scope filter: positions in `app.results.items` that match
/// `app.search_scope`, in the same (already-correct) order YouTube ranked them — `i` below is
/// always a position in `indices`, not in the underlying list itself. Every other scope has
/// nothing to filter (the dedicated fetch already returns only that kind), so `indices` is just
/// `0..app.scoped_results.len()`.
fn results_list(app: &mut App, ui: &mut egui::Ui) {
    let signed_in = app.signed_in();
    let scope = app.search_scope;
    let all = scope == SearchScope::All;
    let indices: Vec<usize> = if all {
        // Cached against `results_generation`: `results.items` doesn't change between one
        // search landing and the next, so re-filtering it every frame is wasted work.
        let generation = app.results_generation;
        match &app.search_all_indices {
            Some((cached_gen, indices)) if *cached_gen == generation => indices.clone(),
            _ => {
                let indices: Vec<usize> = app
                    .results
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| scope.matches(e))
                    .map(|(i, _)| i)
                    .collect();
                app.search_all_indices = Some((generation, indices.clone()));
                indices
            }
        }
    } else {
        (0..app.scoped_results.len()).collect()
    };
    let clicked = {
        let App {
            palette,
            images,
            backend,
            stats,
            now,
            results,
            scoped_results,
            library,
            ..
        } = app;
        let items: &[Entry] = if all { &results.items } else { scoped_results };
        spec_rows(ui, palette, images, backend, stats, indices.len(), |i| {
            crate::ui::entry_row_spec(&items[indices[i]], now, library, signed_in)
        })
    };

    // Forwarded before the click: a menu choice arrives with no row click, and the `None`
    // arm below returns early.
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
    let i = match clicked.click {
        // Translated back to a real position in the source list — `i` from `spec_rows` is only
        // ever a position in this frame's `indices`.
        Some(ListClick::Row(i)) => indices[i],
        Some(ListClick::Link(route)) => {
            app.navigate(route);
            return;
        }
        None => return,
    };
    let item = if all {
        &app.results.items[i]
    } else {
        &app.scoped_results[i]
    };
    match item {
        Entry::Song(_) | Entry::Video(_) => {
            let (index, queue) = if all {
                (app.results.playable_index(i), app.results.playable())
            } else {
                (
                    crate::ui::playable_index(&app.scoped_results, i),
                    crate::ui::playable_queue(&app.scoped_results),
                )
            };
            if let Some(index) = index {
                app.play_row(queue, index);
            }
        }
        Entry::Artist(a) => {
            let id = a.id.clone();
            app.navigate(Route::Artist(id));
        }
        Entry::Album(a) => {
            let id = a.id.clone();
            app.navigate(Route::Album(id));
        }
        Entry::Playlist(p) => {
            let id = p.id.clone();
            app.navigate(Route::Playlist(id));
        }
        Entry::Mood(m) => {
            let params = m.params.clone();
            app.navigate(Route::Mood(params));
        }
    }
}
