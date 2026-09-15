//! The bar across the top of the content: history, search, settings, account, window buttons.
//!
//! It used to be a full-width titlebar painted at the very top of the window, which is what kept
//! the sidebar from reaching the top edge. It is now a panel claimed *after* the sidebar, so the
//! rail runs the whole left side and this bar starts where the rail ends.
//!
//! It also carries the window buttons. That is the one place this shell departs from the
//! reference, where the queue panel runs full height and the top bar covers only the content:
//! minimise / maximise / close have to sit at the window's top-right corner and must not move
//! when the queue opens, and letting this bar run to the window's right edge is what keeps them
//! there.

use egui::{CornerRadius, Rect, Sense, Ui, vec2};

use crate::app::{App, Route};
use crate::chrome;
use crate::icons::Icon;
use crate::theme;
use crate::widgets;

/// How wide the search pill is at most, and how small it is allowed to get before the bar is
/// just too narrow to hold everything.
const SEARCH_W: f32 = 360.0;
const MIN_SEARCH_W: f32 = 140.0;
const SEARCH_H: f32 = 36.0;

/// Back + Forward and the gaps around them, the fixed part left of the search pill.
const LEFT_W: f32 = 32.0 + 4.0 + 32.0 + 12.0;

pub fn top_bar(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette;
    let rect = ui.available_rect_before_wrap();

    // The drag surface goes down first so every control painted after it wins the pointer.
    chrome::drag_surface(ui, rect, app.decorated);

    // The right-hand controls go down *first*, so the search can size itself to whatever is left
    // of the bar instead of running under them when the window is dragged narrow.
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if !app.decorated {
        let close_clicked = chrome::window_buttons(&mut right, &p);
        if close_clicked {
            app.request_close(ui.ctx());
        }
        right.add_space(10.0);
    }
    let signed_in = app.signed_in();
    let chip = account_chip(app, &mut right);
    if chip.clicked() && !signed_in {
        app.navigate(Route::Settings);
    }
    // Signed in, the avatar opens the account menu — View profile, Switch account, Settings,
    // Sign out. Button-anchored: `Toggle` records no pointer position, so a pointer-anchored
    // popup would never render.
    if signed_in {
        account_menu(app, &mut right, &chip);
    }
    right.add_space(2.0);
    // Listening history beside the chip, as a clock — it is a page, not a setting.
    // Settings itself lives only under the chip: signed out, the chip *is* the way in
    // (it navigates straight to Settings); signed in, the account menu carries it. One
    // door, one keyhole — a second gear beside the chip was two doors to the same room.
    if widgets::icon_button(&mut right, &p, Icon::History, 18.0, false).clicked() {
        app.navigate(Route::History);
    }
    // Whatever the right group took, plus a gap so the search never touches it.
    let reserved = (rect.right() - right.min_rect().left()).max(0.0) + 12.0;

    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );

    let can_back = app.can_go_back();
    let can_forward = app.can_go_forward();
    let back = history_button(&mut bar, &p, Icon::ChevronLeft, can_back);
    bar.add_space(4.0);
    let forward = history_button(&mut bar, &p, Icon::ChevronRight, can_forward);
    bar.add_space(12.0);
    let search_w = (rect.width() - LEFT_W - reserved).clamp(MIN_SEARCH_W, SEARCH_W);
    search_field(app, &mut bar, search_w);

    if back.clicked() && can_back {
        app.back();
    }
    if forward.clicked() && can_forward {
        app.forward();
    }
}

/// Back / Forward, as round buttons on the window colour — dimmed and inert with nowhere to go.
fn history_button(
    ui: &mut Ui,
    palette: &theme::Palette,
    icon: Icon,
    enabled: bool,
) -> egui::Response {
    let d = 32.0;
    let (rect, response) = ui.allocate_exact_size(vec2(d, d), Sense::click());
    if ui.is_rect_visible(rect) {
        let t = if enabled {
            widgets::lit(ui, &response)
        } else {
            0.0
        };
        // No disc: it was painted in the window colour onto a transparent top bar that
        // already shows the window colour, so it drew nothing — and once the shell is
        // translucent it would have drawn an opaque coin floating over the blur.
        let fg = if enabled {
            widgets::mix(palette.secondary, palette.text, t)
        } else {
            palette.dim
        };
        let nudge = if enabled {
            widgets::press_offset(&response)
        } else {
            egui::Vec2::ZERO
        };
        icon.image(fg, 18.0).paint_at(
            ui,
            Rect::from_center_size(rect.center() + nudge, vec2(18.0, 18.0)),
        );
        if enabled && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// The search box as a pill with the magnifier inside it, rather than egui's default field.
fn search_field(app: &mut App, ui: &mut Ui, width: f32) {
    let p = app.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(width, SEARCH_H), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(search_id()));
    let t = widgets::lit(ui, &response).max(if focused { 1.0 } else { 0.0 });

    ui.painter().rect(
        rect,
        CornerRadius::same((SEARCH_H / 2.0) as u8),
        widgets::mix(p.surface, p.surface_hover, t),
        egui::Stroke::new(1.0, if focused { p.outline } else { p.surface }),
        egui::StrokeKind::Inside,
    );
    Icon::Search.image(p.secondary, 16.0).paint_at(
        ui,
        Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(16.0, 16.0)),
    );

    let inner = Rect::from_min_max(
        rect.left_center() + vec2(34.0, -SEARCH_H / 2.0),
        rect.right_center() + vec2(-12.0, SEARCH_H / 2.0),
    );
    let mut field_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    // Autocomplete is offered only while the field has focus and the reply matches the text in
    // the box, so a late answer for an older query never flashes up.
    let show = focused
        && !app.suggestions.is_empty()
        && app.suggestions_for.as_deref() == Some(app.query.as_str());
    // Consume the list-navigation keys *before* the field so they drive the popup rather than the
    // caret; Left/Right are left alone, so editing still feels normal.
    let (mut up, mut down, mut enter, mut esc) = (false, false, false, false);
    if show {
        ui.input_mut(|i| {
            up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        });
    }

    let field = egui::TextEdit::singleline(&mut app.query)
        .id(search_id())
        .hint_text("What do you want to play?")
        .desired_width(inner.width())
        .frame(egui::Frame::NONE)
        .font(theme::regular(13.0));
    let field_response = field_ui.add(field);
    if field_response.changed() {
        // Typing refetches autocomplete only; the search waits for a submit. `suggestion_navigated`
        // resets so a stale arrow selection cannot attach to new text.
        app.query_changed_at = Some(std::time::Instant::now());
        app.suggestion_cursor = 0;
        app.suggestion_navigated = false;
    }

    // Submit on a bare Enter. When the suggestion list is open it consumes Enter above, so this
    // path only runs with no list up: search exactly what was typed.
    if !show {
        if field_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            app.submit_search();
        }
        return;
    }

    // The items are cloned out so the popup closure does not borrow `app` while the actions
    // below still need it. `p` is a copy for the same reason.
    let items = app.suggestions.clone();
    let cursor = app.suggestion_cursor.min(items.len().saturating_sub(1));
    let mut picked = None;
    egui::Popup::from_response(&response)
        .id(ui.id().with("ytm-search-suggestions"))
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .align(egui::RectAlign::BOTTOM_START)
        .width(width)
        .gap(4.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_memory(Some(egui::SetOpenCommand::Bool(true)))
        .show(|mui| {
            for (i, item) in items.iter().enumerate() {
                let (row, resp) =
                    mui.allocate_exact_size(vec2(mui.available_width(), 30.0), Sense::click());
                if mui.is_rect_visible(row) {
                    let selected = i == cursor;
                    let t = widgets::lit(mui, &resp).max(if selected { 1.0 } else { 0.0 });
                    if t > 0.001 {
                        mui.painter().rect_filled(
                            row,
                            CornerRadius::same(theme::RADIUS_SMALL + 2),
                            widgets::mix(p.surface, p.surface_hover, t),
                        );
                    }
                    crate::emoji::paint_line(
                        mui,
                        row.left_center() + vec2(14.0, 0.0),
                        item,
                        theme::regular(13.0),
                        p.text,
                    );
                }
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        });

    // A click accepts and searches that suggestion outright. Enter searches what was typed,
    // unless the arrow keys moved the highlight — then the highlighted suggestion wins. Escape
    // hides the list without changing the query; Up/Down move the cursor.
    if let Some(i) = picked {
        app.query = items[i].clone();
        app.clear_suggestions();
        app.submit_search();
    } else if enter {
        if app.suggestion_navigated && !items.is_empty() {
            app.query = items[cursor].clone();
        }
        app.clear_suggestions();
        app.submit_search();
    } else if esc {
        app.clear_suggestions();
    } else if up {
        app.suggestion_cursor = cursor.saturating_sub(1);
        app.suggestion_navigated = true;
    } else if down {
        app.suggestion_cursor = (cursor + 1).min(items.len() - 1);
        app.suggestion_navigated = true;
    }
}

/// A stable id, so focus can be read before the field is added — and requested from outside
/// this module entirely, by the global "jump to search" keyboard shortcut in `App::handle_keys`.
pub(crate) fn search_id() -> egui::Id {
    egui::Id::new("ytm-search-field")
}

/// The account chip: the signed-in account's avatar, opening the account menu. A generic glyph
/// signed out.
fn account_chip(app: &mut App, ui: &mut Ui) -> egui::Response {
    let name = match &app.auth {
        crate::auth::AuthState::SignedIn { account } => Some(account.clone()),
        _ => None,
    };
    avatar_circle(app, ui, name.as_deref())
}

/// The circular avatar on its own: the photo, the initial while it loads, a glyph signed out.
fn avatar_circle(app: &mut App, ui: &mut Ui, name: Option<&str>) -> egui::Response {
    let p = app.palette;
    // The photo YouTube already sent with the channel list.
    let photo = name.and_then(|_| {
        let current = app
            .accounts
            .iter()
            .find(|a| a.page_id == app.settings.account_page_id)
            .or_else(|| app.accounts.iter().find(|a| a.selected))
            .or(app.accounts.first());
        let url = current?.thumbnail.clone()?;
        let crate::app::App {
            images, backend, ..
        } = app;
        crate::ui::art_texture(images, backend, ui.ctx(), Some(&url), 28.0)
    });
    let d = 28.0;
    let (rect, response) = ui.allocate_exact_size(vec2(d, d), Sense::click());
    if ui.is_rect_visible(rect) {
        let t = widgets::lit(ui, &response);
        match (name, photo) {
            (Some(_), Some(tex)) => {
                widgets::paint_art(ui, &p, rect, Some(tex), widgets::ArtShape::Circle);
            }
            (Some(account), None) => {
                ui.painter().circle_filled(
                    rect.center(),
                    d / 2.0,
                    widgets::mix(p.surface_active, p.accent, t),
                );
                let initial = account
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                ui.painter().text(
                    rect.center() + widgets::press_offset(&response),
                    egui::Align2::CENTER_CENTER,
                    initial,
                    theme::semibold(13.0),
                    p.text,
                );
            }
            (None, _) => {
                let fg = widgets::mix(p.secondary, p.text, t);
                Icon::CircleUser.image(fg, 20.0).paint_at(
                    ui,
                    Rect::from_center_size(
                        rect.center() + widgets::press_offset(&response),
                        vec2(20.0, 20.0),
                    ),
                );
            }
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response.on_hover_text(name.unwrap_or("Not signed in"))
}

/// What the chip opens: the channels this Google account can browse as — each with its photo
/// and a tick on the current one — then Settings and Sign out.
fn account_menu(app: &mut App, ui: &mut Ui, chip: &egui::Response) {
    let popup = egui::Popup::new(
        ui.id().with("ytm-account-menu"),
        ui.ctx().clone(),
        chip,
        ui.layer_id(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .width(widgets::MENU_WIDTH)
    .gap(0.0)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .open_memory(chip.clicked().then_some(egui::SetOpenCommand::Toggle));

    enum Choice {
        Profile(String),
        Switch(Option<String>),
        Settings,
        Help,
        SignOut,
    }
    let mut choice = None;
    popup.show(|mui| {
        let p = app.palette;
        // Cloned, not borrowed: the picks below take `&mut App` after the popup closes.
        let accounts = app.accounts.clone();
        let current_id = app.settings.account_page_id.clone();
        // The current channel's profile id, for the View profile item below the list.
        let current_channel_id = accounts
            .iter()
            .find(|a| a.page_id == current_id)
            .or_else(|| accounts.iter().find(|a| a.selected))
            .or(accounts.first())
            .and_then(|a| a.channel_id.clone());
        // The channels, inline and directly on top: photo, name, a tick on the current one. No
        // byline — the subscriber count is noise here.
        for account in accounts {
            let current = current_id == account.page_id;
            let (rect, resp) =
                mui.allocate_exact_size(vec2(mui.available_width(), 40.0), Sense::click());
            if mui.is_rect_visible(rect) {
                let t = widgets::lit(mui, &resp);
                if resp.is_pointer_button_down_on() {
                    mui.painter().rect_filled(
                        rect,
                        CornerRadius::same(theme::RADIUS_SMALL + 2),
                        p.surface_active,
                    );
                } else if t > 0.001 {
                    mui.painter().rect_filled(
                        rect,
                        CornerRadius::same(theme::RADIUS_SMALL + 2),
                        widgets::mix(p.surface, p.surface_hover, t),
                    );
                }
                let nudge = widgets::press_offset(&resp);
                let photo_rect = Rect::from_min_size(
                    rect.left_center() + vec2(6.0, -12.0) + nudge,
                    vec2(24.0, 24.0),
                );
                let (images, backend) = (&mut app.images, &app.backend);
                match account.thumbnail.as_ref().and_then(|url| {
                    crate::ui::art_texture(images, backend, mui.ctx(), Some(url), 24.0)
                }) {
                    Some(tex) => widgets::paint_art(
                        mui,
                        &p,
                        photo_rect,
                        Some(tex),
                        widgets::ArtShape::Circle,
                    ),
                    None => {
                        mui.painter()
                            .circle_filled(photo_rect.center(), 12.0, p.surface_active);
                    }
                }
                let fg = widgets::mix(p.secondary, p.text, t);
                mui.painter().text(
                    photo_rect.right_center() + vec2(8.0, 0.0) + nudge,
                    egui::Align2::LEFT_CENTER,
                    &account.name,
                    theme::medium(13.0),
                    fg,
                );
                if current {
                    Icon::Check.image(p.accent, 15.0).paint_at(
                        mui,
                        Rect::from_center_size(
                            rect.right_center() - vec2(16.0, 0.0),
                            vec2(15.0, 15.0),
                        ),
                    );
                }
                if resp.hovered() {
                    mui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            if resp.clicked() && !current {
                choice = Some(Choice::Switch(account.page_id.clone()));
                mui.close();
            }
        }
        if let Some(id) = current_channel_id
            && widgets::menu_item(mui, &p, Icon::CircleUser, "View profile", None).clicked()
        {
            choice = Some(Choice::Profile(id));
            mui.close();
        }
        if widgets::menu_item(mui, &p, Icon::Settings, "Settings", None).clicked() {
            choice = Some(Choice::Settings);
            mui.close();
        }
        if widgets::menu_item(mui, &p, Icon::Info, "Keyboard shortcuts", None).clicked() {
            choice = Some(Choice::Help);
            mui.close();
        }
        if widgets::menu_item(mui, &p, Icon::LogOut, "Sign out", None).clicked() {
            choice = Some(Choice::SignOut);
            mui.close();
        }
    });
    match choice {
        Some(Choice::Profile(id)) => app.navigate(Route::Artist(id)),
        Some(Choice::Switch(page_id)) => {
            let ctx = ui.ctx().clone();
            app.update_settings(&ctx, |s| s.account_page_id = page_id);
            app.forget_account_data();
        }
        Some(Choice::Settings) => app.navigate(Route::Settings),
        Some(Choice::Help) => app.help_open = true,
        Some(Choice::SignOut) => app.backend.sign_out(),
        None => {}
    }
}
