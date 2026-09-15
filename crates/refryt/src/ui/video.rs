//! The music-video view: a black content card with the software-rendered frame.
//!
//! The frame is letterboxed by mpv to the buffer's aspect, so the buffer is sized to the card and
//! the texture is drawn straight into it.

use crate::app::App;
use crate::theme;

/// The widest buffer drawn, in physical pixels. Software video is CPU-bound, and the card is
/// rarely larger than this on any realistic window.
const MAX_WIDTH_PX: f32 = 1280.0;

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(theme::PANEL_RADIUS),
        egui::Color32::BLACK,
    );

    let Some(surface) = app.video.as_mut() else {
        return;
    };
    let ppp = ui.ctx().pixels_per_point();
    let width = (rect.width() * ppp).clamp(2.0, MAX_WIDTH_PX);
    let height = (width * (rect.height() / rect.width().max(1.0))).clamp(2.0, MAX_WIDTH_PX);
    let Some(texture) = surface.frame(ui, (width.round() as usize, height.round() as usize)) else {
        return;
    };

    ui.painter().image(
        texture.id(),
        rect.shrink(6.0),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}
