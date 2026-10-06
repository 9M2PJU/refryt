//! The frameless window: rounded background, custom titlebar, drag and resize.
//!
//! We ask the OS for a borderless window — transparent where the compositor blends it and opaque
//! on Windows — and paint everything ourselves. That buys
//! the look, and costs us the behaviour a titlebar normally provides for free — dragging,
//! resizing from the edges, double-click to maximise. All of that lives here.
//!
//! macOS is the exception to "borderless". There the window is a normal native one whose title
//! bar is hidden and whose content runs underneath it (`main.rs`), so the system draws the
//! rounded corners, the shadow and the blur clipping, and keeps them correct when the window is
//! zoomed. The app therefore paints a plain, square background on macOS; see `App::ui`. The
//! custom buttons and drag area in this module still apply there.
//!
//! `--decorated` turns the whole thing off and hands the window back to the OS. That escape hatch
//! is not optional: X11 without a compositor paints the rounded corners black, and tiling window
//! managers have their own opinions.

use egui::{
    Color32, CornerRadius, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, vec2,
};

use crate::icons::Icon;
use crate::theme::{self, Palette};

/// Paint the window background: a rounded, outlined rect, or a plain fill when the window is
/// maximised or decorated (rounded corners on a maximised window look like a mistake).
pub fn paint_background(ui: &Ui, palette: &Palette, rounded: bool) {
    let rect = ui.ctx().content_rect();
    let radius = if rounded {
        CornerRadius::same(theme::WINDOW_RADIUS)
    } else {
        CornerRadius::ZERO
    };
    ui.painter().rect(
        rect,
        radius,
        // The one place the shell's opacity is applied: everything else is painted on top of
        // this. The stroke below keeps full alpha — the 1px edge is what tells a translucent
        // window from a hole in the desktop.
        palette.window_fill(),
        if rounded {
            Stroke::new(1.0, palette.outline)
        } else {
            Stroke::NONE
        },
        egui::StrokeKind::Inside,
    );
}

/// Invisible grab bands around the window edge that start a native resize.
///
/// Corners are checked before edges so the diagonal directions win in the overlap, which is what
/// makes dragging a corner feel right.
pub fn resize_handles(ui: &Ui) {
    let ctx = ui.ctx();
    let rect = ctx.content_rect();
    let h = theme::RESIZE_HANDLE;

    let corners = [
        (
            Rect::from_min_size(rect.left_top(), vec2(h, h)),
            ResizeDirection::NorthWest,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(rect.right_top() - vec2(h, 0.0), vec2(h, h)),
            ResizeDirection::NorthEast,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(rect.left_bottom() - vec2(0.0, h), vec2(h, h)),
            ResizeDirection::SouthWest,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(rect.right_bottom() - vec2(h, h), vec2(h, h)),
            ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
    ];
    let edges = [
        (
            Rect::from_min_size(rect.left_top(), vec2(rect.width(), h)),
            ResizeDirection::North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_size(rect.left_bottom() - vec2(0.0, h), vec2(rect.width(), h)),
            ResizeDirection::South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_size(rect.left_top(), vec2(h, rect.height())),
            ResizeDirection::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(rect.right_top() - vec2(h, 0.0), vec2(h, rect.height())),
            ResizeDirection::East,
            egui::CursorIcon::ResizeHorizontal,
        ),
    ];

    for (i, (zone, dir, cursor)) in corners.iter().chain(edges.iter()).enumerate() {
        let id = egui::Id::new("ytm-resize").with(i);
        let r = ui.interact(*zone, id, Sense::drag());
        if r.hovered() || r.dragged() {
            ctx.set_cursor_icon(*cursor);
        }
        if r.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(*dir));
        }
    }
}

/// One window button. Close gets a red hover; the others a neutral one.
fn window_button(ui: &mut Ui, palette: &Palette, icon: Icon, danger: bool) -> egui::Response {
    let size = vec2(32.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if hovered {
            let bg = if danger {
                palette.danger
            } else {
                palette.glass(palette.surface_hover)
            };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS_SMALL), bg);
        }
        let fg = if hovered && danger {
            Color32::WHITE
        } else if hovered {
            palette.text
        } else {
            palette.secondary
        };
        icon.image(fg, 14.0)
            .paint_at(ui, Rect::from_center_size(rect.center(), vec2(14.0, 14.0)));
    }
    response
}

/// Make a rect drag the window, and double-click maximise it.
///
/// Installed before any control in the bar, so widgets painted afterwards take the pointer
/// first — anything interactive drawn on top swallows the drag, which is what we want.
pub fn drag_surface(ui: &Ui, rect: Rect, decorated: bool) {
    if decorated {
        return;
    }
    let ctx = ui.ctx().clone();
    let drag = ui.interact(
        rect,
        egui::Id::new("ytm-titlebar-drag"),
        Sense::click_and_drag(),
    );
    if drag.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// Minimise, maximise and close, for the current layout's leading edge.
///
/// Drawn into whatever `Ui` is passed, so the caller decides where they sit; in practice that is
/// the right-to-left end of the top bar, which is the window's top-right corner.
///
/// Returns whether the close button was clicked, rather than acting on it directly: whether
/// that means actually closing or hiding to the tray depends on a setting only the caller
/// (which has `&mut App`) can see — see `App::request_close`.
pub fn window_buttons(ui: &mut Ui, palette: &Palette) -> bool {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let close_clicked = window_button(ui, palette, Icon::X, true).clicked();
    if window_button(ui, palette, Icon::Square, false).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if window_button(ui, palette, Icon::Minus, false).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        crate::pacing::minimize_requested();
    }
    close_clicked
}
