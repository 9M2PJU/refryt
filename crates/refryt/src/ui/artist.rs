//! The artist page: header, top songs, then YouTube's own shelves.
//!
//! The shelves are kept as YouTube sends them — Albums, Singles & EPs, Videos, Live performances,
//! Featured on, Playlists, Fans might also like — rather than flattened into one grid. That
//! flattening is what made albums and singles indistinguishable and threw the other five away.

use egui::{pos2, vec2};

use crate::app::App;
use crate::ui::{HEADER_ART_PT, ListClick, track_shelf_pages};
use crate::widgets;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    if app.artist_loading {
        widgets::skeleton_header(
            ui,
            &app.palette,
            HEADER_ART_PT,
            widgets::SkeletonHeaderKind::Artist,
        );
        widgets::skeleton_track_rows(ui, &app.palette, 6);
        widgets::skeleton_shelf(
            ui,
            &app.palette,
            widgets::SkeletonShelfShape::Cards(widgets::ArtShape::Square),
            4,
        );
        return;
    }
    if let Some(err) = app.artist_error.clone() {
        crate::ui::failed(app, ui, err, App::retry_route);
        return;
    }
    if app.artist.is_none() {
        return;
    }

    crate::ui::page_scroll(app, "artist").show(ui, |ui| {
        // The tint travels with the page: painted in scroll coordinates, it scrolls away
        // with the header instead of lingering as a background.
        // Artist pages stay on the fixed accent — the request behind the adaptive colour named
        // album/playlist headers only.
        crate::ui::header_wash(
            ui,
            &app.palette,
            app.content_rect,
            ui.cursor().min.y,
            app.palette.accent,
        );
        header(app, ui);
        crate::ui::info_popup(app, ui);
        if app.artist.as_ref().is_some_and(|a| !a.top_songs.is_empty()) {
            top_songs(app, ui);
        }
        shelves(app, ui);
    });
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let signed_in = app.signed_in();
    let art_top = ui.cursor().top();
    // Everything the header and its Shuffle/Mix/Subscribe/••• row need is read here, while
    // `app.artist` and `app.library` are borrowed — only the outcomes (all owned) cross back
    // out to act on `app`, the same shape `album.rs`'s header uses for its own ••• menu.
    let (out, id, below, play_target, subscribe_to, menu_action, menu_description) = {
        let crate::app::App {
            palette,
            images,
            backend,
            artist,
            library,
            ..
        } = app;
        let p = *palette;
        let Some(a) = artist.as_ref() else {
            return;
        };
        // The monthly audience alone, as the site shows it. A small artist has none — then the
        // subscriber line takes its place, complete with its word ("965 subscribers"), never
        // the bare button count.
        let audience = a
            .monthly_listeners
            .clone()
            .or_else(|| a.subscribers_long.clone())
            .unwrap_or_else(|| "Artist".to_owned());
        let actions = a.actions.clone();
        let has_monthly = a.monthly_listeners.is_some();
        let subscribers = a.subscribers.clone();
        let id = a.id.clone();
        let out = crate::ui::page_header(
            ui,
            &p,
            images,
            backend,
            crate::ui::PageHeader {
                art_url: a.thumbnail.as_ref(),
                art_size: HEADER_ART_PT,
                circle: true,
                // No type line on an artist: the name is the page.
                kind_label: "",
                title: &a.name,
                description: None,
                artists: &[],
                second_plain: Some(&audience),
                meta: None,
                owner: None,
                credit_gap: 36.0,
                // Shuffle, Mix, Subscribe and ••• are this page's own row below, not
                // `page_header`'s Play/Save/••• — that row and this one would otherwise land
                // on the exact same rect and paint on top of each other.
                play_size: None,
                playing: false,
                save_in_library: None,
                dots: false,
                salt: "artist-name",
            },
        );

        // Shuffle, Mix and Subscribe, from what the header itself carried — on the same
        // baseline as the releases' Play/Save/••• row, nudged up from the bottom of the art,
        // so the buttons sit inside the circle instead of floating under the name. With the
        // art shrunk to its floor, the row no longer fits inside the art column, so it drops
        // below both art and text and takes the card's full width.
        let inline_y = out.art_bottom - 46.0;
        let below = inline_y < out.text_bottom;
        let buttons_rect = if below {
            egui::Rect::from_min_size(
                pos2(out.text_x - out.art_size - 20.0, out.text_bottom + 6.0),
                vec2(ui.available_width(), 46.0),
            )
        } else {
            egui::Rect::from_min_size(pos2(out.text_x, inline_y), vec2(out.text_w, 46.0))
        };
        let mut buttons = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(buttons_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let mut play_target = None;
        // Mix leads: when both are on offer, starting a mix is the one worth reaching for
        // first.
        if let Some(target) = &actions.mix
            && widgets::chip(&mut buttons, &p, "Mix", false).clicked()
        {
            play_target = Some(target.clone());
        }
        if let Some(target) = &actions.shuffle
            && widgets::pill_button(&mut buttons, &p, "Shuffle").clicked()
        {
            play_target = Some(target.clone());
        }
        let mut subscribe_to = None;
        if let Some(subscribed) = actions.subscribed {
            // The count rides the button only while the audience line above shows something
            // else (a monthly listener total). When the line *is* the subscriber count,
            // repeating it here read as a bug: "965 subscribers" over "Subscribe · 965".
            let label = match (subscribed, has_monthly, subscribers.as_deref()) {
                (true, _, _) => "Subscribed".to_owned(),
                (false, true, Some(count)) => format!("Subscribe · {count}"),
                _ => "Subscribe".to_owned(),
            };
            if widgets::chip(&mut buttons, &p, &label, subscribed).clicked() {
                subscribe_to = Some(!subscribed);
            }
        }
        buttons.add_space(6.0);
        // Trails the row, same as the reference's Play/Save/•••. Just Share and — when
        // there's a bio — Info live behind it: `tracks: &[]`/`artist_id: None` make the
        // menu's own Shuffle/Mix/queueing/"Go to artist" items self-hide, since this page
        // already has its own Shuffle/Mix buttons and linking to itself is meaningless.
        let dots =
            widgets::icon_button(&mut buttons, &p, crate::icons::Icon::Ellipsis, 16.0, false);
        drop(buttons);

        let menu_share = crate::model::artist_url(&a.id);
        let menu_has_info = a
            .description
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty());
        let menu_action = widgets::page_menu(
            ui,
            &p,
            &dots,
            widgets::PageMenu {
                page_id: &a.id,
                tracks: &[],
                artist_id: None,
                playlists: &library.playlists.items,
                signed_in,
                share_url: menu_share,
                has_info: menu_has_info,
                manage: false,
                remove_from_library: false,
                context_only: false,
                save_as_new: None,
            },
        );
        let menu_description = matches!(menu_action, Some(crate::ui::RowAction::PageInfo))
            .then(|| a.description.clone())
            .flatten();

        (
            out,
            id,
            below,
            play_target,
            subscribe_to,
            menu_action,
            menu_description,
        )
    };

    if let Some(target) = play_target {
        app.play_target(id.clone(), target);
    }
    if let Some(subscribe) = subscribe_to {
        app.toggle_subscribed(&id, subscribe);
    }
    match menu_action {
        Some(crate::ui::RowAction::PageInfo) => app.page_info = menu_description,
        Some(other) => app.row_action(other),
        None => {}
    }

    // The header's own bottom, plus the pill row when it dropped below the art.
    ui.add_space(out.art_bottom - art_top + if below { 58.0 } else { 12.0 });
}
fn top_songs(app: &mut App, ui: &mut egui::Ui) {
    let signed_in = app.signed_in();
    let clicked = {
        let App {
            palette,
            images,
            backend,
            stats,
            now,
            artist,
            library,
            ..
        } = app;
        let tracks = artist
            .as_ref()
            .map(|a| a.top_songs.as_slice())
            .unwrap_or(&[]);
        let artist_id = artist.as_ref().map(|a| a.id.as_str()).unwrap_or_default();
        track_shelf_pages(
            ui,
            palette,
            images,
            backend,
            stats,
            "Top songs",
            now,
            tracks,
            &library.playlists.items,
            signed_in,
            artist_id,
        )
    };
    match clicked.click {
        Some(ListClick::Row(i)) => {
            if let Some(songs) = app.artist.as_ref().map(|a| a.top_songs.clone()) {
                app.start_from(songs, i);
            }
        }
        Some(ListClick::Link(route)) => app.navigate(route),
        None => {}
    }
    if let Some(action) = clicked.action {
        app.row_action(action);
    }
}

/// Every shelf YouTube sent, in its order: a grid with "Show all" in the top right where it
/// offered one, or — once that's been pressed — the full list, boxed off so it reads as its
/// own block rather than the preview quietly growing in place.
fn shelves(app: &mut App, ui: &mut egui::Ui) {
    let Some(count) = app.artist.as_ref().map(|a| a.shelves.len()) else {
        return;
    };
    for i in 0..count {
        // Cloned per shelf, once per frame: the alternative is holding a borrow on `app.artist`
        // across `artist_shelf`, which needs `&mut App` for navigation and the texture cache.
        let Some(s) = app.artist.as_ref().and_then(|a| a.shelves.get(i)).cloned() else {
            continue;
        };
        crate::ui::artist_shelf(app, ui, &s);
    }
}
