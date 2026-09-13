//! Shared widgets, all drawn by hand.
//!
//! Hover and press are *felt*, not just shown: every control fades its fill over
//! [`theme::HOVER_FADE_SECS`] and sinks a pixel while it is held down. See [`lit`] for why that
//! does not cost an idle app any frames.

#![allow(dead_code)] // design system built ahead of the pages that consume it

use egui::{Color32, CornerRadius, Rect, Sense, Ui, vec2};

use crate::icons::Icon;
use crate::theme::{self, Palette};

/// How lit a control is: `0.0` at rest, `1.0` hovered, moving between them over
/// [`theme::HOVER_FADE_SECS`].
///
/// `animate_bool_with_time` asks for a repaint only while the value is actually travelling — a
/// handful of frames after the pointer arrives or leaves — so an app nobody is touching still
/// sits at zero. That is the whole basis on which the speed budget's "no animation" rule was
/// relaxed, and the idle phase of `scripts/perf.sh` is what holds it.
pub fn lit(ui: &Ui, response: &egui::Response) -> f32 {
    fade(ui, response.id.with("lit"), response.hovered())
}

/// [`lit`] for a condition of your own — a row lights on `contains_pointer` rather than
/// `hovered`, because the links and thumbs stacked on top of it take the hover flag and the
/// highlight would blink out as the pointer crossed them.
pub fn fade(ui: &Ui, id: egui::Id, on: bool) -> f32 {
    ui.ctx()
        .animate_bool_with_time(id, on, theme::HOVER_FADE_SECS)
}

/// Whether the pointer is being held down on this control. Instant, not faded: a press should
/// answer immediately or it reads as lag.
pub fn held(response: &egui::Response) -> bool {
    response.is_pointer_button_down_on()
}

/// The pixel a held control sinks by, so pressing something feels like pressing something.
pub fn press_offset(response: &egui::Response) -> egui::Vec2 {
    if held(response) {
        vec2(0.0, 1.0)
    } else {
        egui::Vec2::ZERO
    }
}

/// Blend `from` toward `to` by `t`.
pub fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(
        lerp(from.r(), to.r()),
        lerp(from.g(), to.g()),
        lerp(from.b(), to.b()),
    )
}

/// The background a control shows for its current state, faded in.
///
/// `None` at rest, so a control at rest paints nothing at all rather than a transparent rect.
fn state_fill(ui: &Ui, palette: &Palette, response: &egui::Response) -> Option<Color32> {
    if held(response) {
        return Some(palette.surface_active);
    }
    let t = lit(ui, response);
    (t > 0.001).then(|| palette.surface_hover.gamma_multiply(t))
}

/// A square icon button that lights up under the pointer and sinks when pressed.
///
/// `active` is for toggles that have a persistent on state — the queue's list button is the
/// first — and paints the accent glyph plus a small dot beneath it, so "on" reads at a glance.
/// The button keeps its resting look, so an open queue does not read as a pressed button.
pub fn icon_button(
    ui: &mut Ui,
    palette: &Palette,
    icon: Icon,
    size: f32,
    active: bool,
) -> egui::Response {
    let box_size = size + 14.0;
    let (rect, response) = ui.allocate_exact_size(vec2(box_size, box_size), Sense::click());

    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        if let Some(fill) = state_fill(ui, palette, &response) {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
        }
        let fg = if active {
            palette.accent
        } else {
            mix(palette.secondary, palette.text, t)
        };
        let centre = rect.center() + press_offset(&response);
        icon.image(fg, size)
            .paint_at(ui, Rect::from_center_size(centre, vec2(size, size)));
        if active {
            // Inside the box, clear of the glyph: an indicator, not a layout citizen, so the
            // row's spacing does not move when it appears.
            ui.painter().circle_filled(
                egui::pos2(rect.center().x, rect.center().y + size / 2.0 + 3.0),
                2.0,
                palette.accent,
            );
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// The round accent play button in the player bar.
pub fn play_button(ui: &mut Ui, palette: &Palette, playing: bool, diameter: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(diameter, diameter), Sense::click());

    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        let fill = mix(palette.accent, palette.accent_hover, t);
        // Grows a little under the pointer and squashes when held — the one control worth
        // making feel like a physical button, since it is the one everybody presses.
        let scale = if held(&response) {
            0.94
        } else {
            1.0 + 0.04 * t
        };
        let r = diameter / 2.0 * scale;
        ui.painter().circle_filled(rect.center(), r, fill);

        let glyph = if playing {
            Icon::PauseFilled
        } else {
            Icon::PlayFilled
        };
        let glyph_size = diameter * 0.44 * scale;
        // The play triangle's visual centre sits left of its bounding box, so nudge it right.
        let offset = if playing { 0.0 } else { glyph_size * 0.06 };
        glyph.image(palette.on_accent, glyph_size).paint_at(
            ui,
            Rect::from_center_size(
                rect.center() + vec2(offset, 0.0),
                vec2(glyph_size, glyph_size),
            ),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// A sidebar navigation entry: Home, Search, Library.
///
/// No pill behind the selected one — selection is carried by weight and colour alone, which is
/// what keeps a rail of three from reading as a row of buttons. Hover still gets a fill, because
/// a target that does not answer the pointer feels dead.
pub fn nav_item(
    ui: &mut Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::NAV_HEIGHT),
        Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        if let Some(fill) = state_fill(ui, palette, &response) {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
        }

        let fg = if selected {
            palette.text
        } else {
            mix(palette.secondary, palette.text, t)
        };
        let nudge = press_offset(&response);
        let icon_rect = Rect::from_center_size(
            rect.left_center() + vec2(24.0, 0.0) + nudge,
            vec2(22.0, 22.0),
        );
        icon.image(fg, 22.0).paint_at(ui, icon_rect);

        let font = if selected {
            theme::bold(15.0)
        } else {
            theme::semibold(15.0)
        };
        crate::emoji::paint_line(
            ui,
            rect.left_center() + vec2(48.0, 0.0) + nudge,
            label,
            font,
            fg,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// How a piece of artwork is masked.
///
/// Artists are people and get circles; releases get rounded squares. That is how YouTube Music
/// itself separates the two, and in a mixed search list it is the difference between being able to
/// tell an artist from an album at a glance and not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtShape {
    Square,
    Circle,
    /// A music-video still: 16:9, drawn in a shorter card.
    Wide,
}

impl ArtShape {
    /// The corner radius that produces this shape for `rect`. A radius of half the side is a
    /// circle; `CornerRadius` is a `u8`, so very large art clamps (harmlessly — nothing we draw is
    /// anywhere near 510px across).
    fn radius(self, rect: Rect) -> CornerRadius {
        match self {
            // The curve grows with the art — an eighth of the edge, floored at the general
            // radius so 40px row covers keep today's corner, capped so a big card's corners
            // do not start eating the image.
            ArtShape::Square | ArtShape::Wide => CornerRadius::same(
                ((rect.width().min(rect.height()) * 0.125) as u8)
                    .clamp(theme::RADIUS, theme::ART_RADIUS_MAX),
            ),
            ArtShape::Circle => {
                CornerRadius::same((rect.width().min(rect.height()) / 2.0).clamp(0.0, 255.0) as u8)
            }
        }
    }
}

/// Placeholder artwork: a filled shape with a glyph, shown until the real cover arrives so rows
/// never change height when images load.
pub fn art_placeholder(ui: &Ui, palette: &Palette, rect: Rect, shape: ArtShape) {
    ui.painter()
        .rect_filled(rect, shape.radius(rect), palette.surface);
    let g = rect.width() * 0.5;
    // A disc reads as a release; a person outline would read as an artist, but the icon set has no
    // person, so the artist placeholder is simply the empty circle.
    if shape != ArtShape::Circle {
        Icon::Disc3
            .image(palette.dim, g)
            .paint_at(ui, Rect::from_center_size(rect.center(), vec2(g, g)));
    }
}

/// Draw cover art, falling back to the placeholder so the slot is always filled.
pub fn paint_art(
    ui: &Ui,
    palette: &Palette,
    rect: Rect,
    texture: Option<&egui::TextureHandle>,
    shape: ArtShape,
) {
    match texture {
        Some(tex) => {
            let radius = shape.radius(rect);
            // The backing is the placeholder's own colour, never black: a light theme was
            // growing a dark rim wherever the image did not quite reach the rect's edges.
            ui.painter()
                .add(egui::Shape::rect_filled(rect, radius, palette.surface));
            // Fit, never stretch: a crop that arrives at a different aspect than its slot
            // letterboxes instead of squashing. A square cover in a square slot is unchanged.
            let tex_size = tex.size_vec2();
            let scale = (rect.width() / tex_size.x).min(rect.height() / tex_size.y);
            egui::Image::new(tex)
                .corner_radius(radius)
                .paint_at(ui, Rect::from_center_size(rect.center(), tex_size * scale));
        }
        None => art_placeholder(ui, palette, rect, shape),
    }
}

/// The play affordance that rises over artwork under the pointer.
///
/// The single most-felt thing in this design system: a cover you can hover and press is a
/// control, a cover you cannot is a picture. `t` is the hover fade from [`lit`], so the button
/// grows and rises into place rather than appearing.
///
/// `corner` puts it at the bottom-right of a grid card, the way a shelf does; otherwise it is
/// centred, which is what a small row cover wants.
pub fn play_overlay(ui: &Ui, palette: &Palette, art: Rect, t: f32, corner: bool) {
    if t <= 0.01 {
        return;
    }
    let full = if corner { 34.0 } else { art.width() * 0.55 };
    let d = full * (0.7 + 0.3 * t);
    let centre = if corner {
        // Rises 8pt as it fades in.
        art.right_bottom() - vec2(full * 0.5 + 8.0, full * 0.5 + 8.0 - 8.0 * (1.0 - t))
    } else {
        // A scrim under a centred glyph, so it reads over a busy cover.
        ui.painter().rect_filled(
            art,
            ArtShape::Square.radius(art),
            Color32::from_black_alpha((150.0 * t) as u8),
        );
        art.center()
    };
    ui.painter()
        .circle_filled(centre, d / 2.0, palette.accent.gamma_multiply(t));
    let glyph = d * 0.44;
    Icon::PlayFilled
        .image(palette.on_accent.gamma_multiply(t), glyph)
        .paint_at(
            ui,
            Rect::from_center_size(centre + vec2(glyph * 0.06, 0.0), vec2(glyph, glyph)),
        );
}

/// Format a duration the way a track list does.
pub fn fmt_duration(secs: u64) -> String {
    let (m, s) = (secs / 60, secs % 60);
    if m >= 60 {
        format!("{}:{:02}:{:02}", m / 60, m % 60, s)
    } else {
        format!("{m}:{s:02}")
    }
}

/// Where a clickable piece of a row's subtitle leads.
///
/// Deliberately not an `app::Route`: widgets stay independent of routing, and the caller turns
/// this into navigation. It also means no `String` is allocated per row per frame just to describe
/// a link nobody clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkTarget<'a> {
    Artist(&'a str),
    Album(&'a str),
}

/// One piece of a row's subtitle. A piece with a target is a link.
///
/// `sep` is what precedes the piece: `" • "` between fields, `", "` between artists in the same
/// credit, and nothing before the first. Without it several artists would read `A • B • C` rather
/// than the `A, B` YouTube writes.
#[derive(Clone, Copy, Debug)]
pub struct Part<'a> {
    pub text: &'a str,
    pub target: Option<LinkTarget<'a>>,
    pub sep: &'a str,
}

/// What separates two fields of a subtitle.
pub const FIELD_SEP: &str = " • ";
/// What separates two artists inside one credit.
pub const NAME_SEP: &str = ", ";

impl<'a> Part<'a> {
    pub fn plain(text: &'a str) -> Self {
        Self {
            text,
            target: None,
            sep: FIELD_SEP,
        }
    }

    pub fn link(text: &'a str, target: LinkTarget<'a>) -> Self {
        Self {
            text,
            target: Some(target),
            sep: FIELD_SEP,
        }
    }

    /// The same piece, joined to the one before it by `sep`.
    pub fn joined(mut self, sep: &'a str) -> Self {
        self.sep = sep;
        self
    }
}

/// A row's subtitle: a fixed handful of pieces, so no allocation is needed to describe one.
///
/// Six rather than three because a track can credit several artists and each is its own link —
/// four artists, an album and the queue's "Radio" tail is the realistic ceiling.
pub const SUBTITLE_PARTS: usize = 6;
pub type Subtitle<'a> = [Option<Part<'a>>; SUBTITLE_PARTS];

/// An empty subtitle, to fill in.
pub fn no_subtitle<'a>() -> Subtitle<'a> {
    [None; SUBTITLE_PARTS]
}

/// Build a one-piece, unlinked subtitle — what most rows have.
pub fn plain_subtitle(text: &str) -> Subtitle<'_> {
    let mut out = no_subtitle();
    out[0] = Some(Part::plain(text));
    out
}

/// What a click on a row meant.
///
/// The link is reported separately from the row's own response so a click on the artist name does
/// not *also* start the track. The caller checks `link` first.
pub struct RowClick<'a> {
    pub response: egui::Response,
    pub link: Option<LinkTarget<'a>>,
    pub(crate) action: Option<super::ui::RowAction>,
}

/// What a song's context menu is built from: the track everything derives from.
///
/// Later milestones add shape here — like state, playlist membership — rather than re-plumbing
/// every call site.
pub struct RowMenu<'a> {
    pub track: &'a crate::model::Track,
    /// The open playlist, when this row is one of its tracks and the playlist is editable
    /// (yours — not LM, not someone else's). Enables "Remove from playlist".
    pub playlist_id: Option<&'a str>,
    /// Your playlists, for the "Save to playlist" picker. Empty when signed out, which hides
    /// the item with no auth check — the library is only ever fetched on sign-in.
    pub playlists: &'a [crate::model::PlaylistRef],
    /// Whether there is a session. Gates the rating thumbs: without one the write is
    /// impossible, and a control that cannot do anything should not be drawn.
    pub signed_in: bool,
    /// This row's own position in the queue, only from the Queue panel. Enables
    /// "Remove from queue" — every other caller leaves it `None`.
    pub queue_index: Option<usize>,
}

/// The fields a [`track_row`] shows, grouped so the function stays under clippy's arity limit
/// and the call sites read like a row.
pub struct TrackRow<'a> {
    pub number: Option<usize>,
    /// A chart rank, shown in place of the position number where the track names one.
    pub rank: Option<&'a str>,
    /// Which way the rank moved. Unknown paints nothing.
    pub trend: crate::model::ChartTrend,
    pub title: &'a str,
    pub subtitle: Subtitle<'a>,
    pub duration: Option<u64>,
    pub plays: Option<&'a str>,
    pub explicit: bool,
    pub active: bool,
    /// A small right-aligned type label ("ARTIST", "ALBUM"), for mixed lists where the rows are
    /// not all songs. Shown in place of the duration, which those rows do not have.
    pub badge: Option<&'a str>,
    pub art: Option<&'a egui::TextureHandle>,
    pub shape: ArtShape,
    /// Distinguishes this row's interactive pieces from its neighbours'.
    pub id_salt: usize,
    /// Right-click menu. `None` for rows that are not songs.
    pub menu: Option<RowMenu<'a>>,
    /// Your rating. Drives the hover thumbs; `Indifferent` on rows without a menu.
    pub like: crate::model::LikeStatus,
    /// Whether this row offers rating thumbs at all. The queue sets it `false`: it is a list of
    /// what happens next, and a column of controls repeated down it is noise. Rating a queued
    /// song is still one right-click away.
    pub show_rating: bool,
}

/// Paint a subtitle left to right, each linked piece getting its own hit area, and report the
/// piece that was clicked.
///
/// Shared by track rows and the player bar. The bar used to paint its own single string with one
/// hit rect over the whole thing, which is why only one artist there was ever clickable and why
/// it had no album link and no hover underline.
#[allow(clippy::too_many_arguments)]
pub fn paint_subtitle<'a>(
    ui: &mut Ui,
    palette: &Palette,
    pos: egui::Pos2,
    right: f32,
    subtitle: &Subtitle<'a>,
    tail: Option<&'a str>,
    id_salt: impl std::hash::Hash + Clone + std::fmt::Debug,
) -> Option<LinkTarget<'a>> {
    let font = theme::regular(12.0);
    let mut x = pos.x;
    let mut clicked = None;

    let parts = subtitle
        .iter()
        .flatten()
        .filter(|p| !p.text.is_empty())
        .copied()
        .chain(tail.map(Part::plain));

    for (i, part) in parts.enumerate() {
        if x >= right {
            break;
        }
        // Measure the piece before painting its separator: a trailing " • " with nothing
        // after it reads as broken, so both go when the piece has no room left.
        let sep_w = if i > 0 && !part.sep.is_empty() {
            text_width(ui, part.sep, &font)
        } else {
            0.0
        };
        let shown = truncate_to(ui, part.text, font.clone(), (right - x - sep_w).max(0.0));
        if shown.is_empty() {
            break;
        }
        if sep_w > 0.0 {
            ui.painter().text(
                egui::pos2(x, pos.y),
                egui::Align2::LEFT_CENTER,
                part.sep,
                font.clone(),
                palette.dim,
            );
            x += sep_w;
        }
        let w = text_width(ui, &shown, &font);

        let mut colour = palette.secondary;
        if let Some(target) = part.target {
            let hit = Rect::from_min_size(egui::pos2(x, pos.y - 8.0), vec2(w, 16.0));
            let r = ui.interact(hit, ui.id().with((id_salt.clone(), i)), Sense::click());
            if r.hovered() {
                colour = palette.text;
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                ui.painter().hline(
                    x..=(x + w),
                    pos.y + 7.0,
                    egui::Stroke::new(1.0, palette.text),
                );
            }
            if r.clicked() {
                clicked = Some(target);
            }
        }
        crate::emoji::paint_line(ui, egui::pos2(x, pos.y), &shown, font.clone(), colour);
        x += w;
    }
    clicked
}

/// How wide the dissolve at an overlong line's edge is.
pub const FADE_W: f32 = 24.0;
/// Hover this long before an overlong line starts moving.
const MARQUEE_DWELL_SECS: f64 = 0.6;
/// Pixels per second once moving.
const MARQUEE_SPEED: f32 = 30.0;
/// Rest at each end of the travel before turning around.
const MARQUEE_HOLD_SECS: f64 = 0.4;
/// The click/hover height of a marquee line — the player bar's title uses it for its hit rect
/// when the text fits and `marquee_line` draws no viewport of its own.
pub(crate) const MARQUEE_HIT_H: f32 = 18.0;

/// Dissolve the trailing edge of a text viewport into `bg`, so an overlong line never
/// touches whatever sits past it. Slices, not a mesh: epaint has no gradient fill, and 24
/// 1px rects are cheaper than one mesh for a single edge.
///
/// `h` is the line's own height — the fade is clamped to it, so one line's dissolve can
/// never bleed onto its neighbour.
pub fn fade_edge(ui: &mut Ui, right: f32, center_y: f32, h: f32, bg: Color32) {
    fade_span(ui, right - FADE_W, center_y, h, bg, false);
}

/// The mirror: dissolve the leading edge, shown only once the line has scrolled and its
/// head is cut off.
fn fade_leading(ui: &mut Ui, left: f32, center_y: f32, h: f32, bg: Color32) {
    fade_span(ui, left, center_y, h, bg, true);
}

fn fade_span(ui: &mut Ui, from: f32, center_y: f32, h: f32, bg: Color32, mirrored: bool) {
    let steps = FADE_W as usize;
    for i in 0..steps {
        let f = (i as f32 + 1.0) / steps as f32;
        let t = if mirrored { 1.0 - f } else { f };
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(from + i as f32, center_y - h / 2.0),
                vec2(1.0, h),
            ),
            CornerRadius::ZERO,
            // `gamma_multiply` scales a premultiplied colour, alpha included, so this ramps
            // toward `bg` *as painted* — a translucent shell no longer grows an opaque plate
            // at the end of a long title, and the old `from_rgba_unmultiplied` also mixed
            // premultiplied channels as if they were straight.
            bg.gamma_multiply(t),
        );
    }
}

/// What a scrolling line remembers between frames. Lives in egui's temp storage under the
/// caller's id salt, so no scroll state threads through the app.
#[derive(Clone, Default)]
struct MarqueeState {
    /// The full width last measured. A different width means a different line — a new
    /// track on the same salt — so the offset resets rather than starting mid-scroll.
    full_w: f32,
    offset: f32,
    /// +1 moving away from the head, -1 coming back.
    dir: f32,
    hover_since: Option<f64>,
    last_t: f64,
    hold_until: f64,
    /// A link was hovered last frame. Links sit on top of the viewport and steal its
    /// hover flag, so this keeps the scroll alive under the pointer.
    link_hovered: bool,
}

/// One travel step with end flips: never leaves `0..=max_off`, and reports which way it is
/// heading afterwards. Pure, so the bounds are unit-testable without an egui context.
fn pingpong(offset: f32, dir: f32, max_off: f32, advance: f32) -> (f32, f32) {
    let mut offset = offset + dir * advance;
    let mut dir = dir;
    if offset >= max_off {
        offset = max_off;
        dir = -1.0;
    } else if offset <= 0.0 {
        offset = 0.0;
        dir = 1.0;
    }
    (offset, dir)
}

/// A line that scrolls itself on hover when it does not fit.
///
/// Fits and it is an ordinary painted line — no state, no cost. Overflows and the paint is
/// clipped to `max_width` with a dissolve on the trailing edge; hovering the text scrolls
/// it ping-pong after a short dwell, with a dissolve appearing on the leading edge once
/// the head is cut off. Returns the viewport's response when one was drawn — only the
/// overflow case creates one — so a caller can make the text itself clickable without
/// covering the hover that drives the scroll; `None` means it fit and was painted plainly.
///
/// The dissolves are clamped to the font's own row height, so a title's fade stops where
/// its line stops and never washes the credits below. Repaint discipline: frames are
/// asked for only while hovered-and-overflowing. At rest nothing is requested; unhovering
/// snaps home on the pointer-move frame that caused it.
#[allow(clippy::too_many_arguments)]
pub fn marquee_line(
    ui: &mut Ui,
    pos: egui::Pos2,
    max_width: f32,
    text: &str,
    font: egui::FontId,
    color: Color32,
    // `None` skips the dissolve entirely rather than guessing: a shell-translucent background
    // is dominated by whatever a real, live-blurred desktop shows through it, which cannot be
    // known ahead of painting — an opaque guess there paints a confidently wrong plate, and a
    // still-translucent guess washes an extra, still-wrong tint over the correctly-rendered
    // pixel. The text is already hard-clipped at this same boundary regardless (below), so
    // skipping just loses the decorative dissolve, not any actual coverage.
    bg: Option<Color32>,
    id_salt: impl std::hash::Hash + Clone + std::fmt::Debug,
    sense: egui::Sense,
    // Whether the caller's wider row (art, duration, blank space — not just this text strip)
    // is under the pointer. A row's own hover already exists at the call site; without this,
    // the scroll only ever started from the narrow text itself, which felt inconvenient to
    // trigger from anywhere else on the row.
    row_hovered: bool,
) -> Option<egui::Response> {
    let full = text_width(ui, text, &font);
    if full <= max_width {
        crate::emoji::paint_line(ui, pos, text, font, color);
        return None;
    }
    let id = ui.id().with(id_salt);
    let viewport = Rect::from_min_size(
        egui::pos2(pos.x, pos.y - MARQUEE_HIT_H / 2.0),
        vec2(max_width, MARQUEE_HIT_H),
    );
    let response = ui.interact(viewport, id, sense);
    let hovered = response.hovered() || row_hovered;
    let now = ui.input(|i| i.time);
    let mut state: MarqueeState = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    if (state.full_w - full).abs() > 0.5 {
        state = MarqueeState {
            full_w: full,
            dir: 1.0,
            last_t: now,
            ..Default::default()
        };
    }
    if hovered {
        ui.ctx().request_repaint();
        if state.hover_since.is_none() {
            state.hover_since = Some(now);
        }
        let since = state.hover_since.unwrap_or(now);
        if now - since >= MARQUEE_DWELL_SECS && now >= state.hold_until {
            let dt = (now - state.last_t).clamp(0.0, 0.25) as f32;
            let max_off = (full - max_width).max(0.0);
            let (offset, dir) = pingpong(state.offset, state.dir, max_off, MARQUEE_SPEED * dt);
            if (dir - state.dir).abs() > f32::EPSILON {
                state.hold_until = now + MARQUEE_HOLD_SECS;
            }
            state.offset = offset;
            state.dir = dir;
        }
    } else {
        state.hover_since = None;
        state.offset = 0.0;
        state.dir = 1.0;
        state.hold_until = 0.0;
    }
    state.last_t = now;
    state.full_w = full;
    ui.ctx().data_mut(|d| d.insert_temp(id, state.clone()));

    let clip = ui.clip_rect().intersect(viewport);
    crate::emoji::paint_clipped_line(
        ui,
        clip,
        egui::pos2(pos.x - state.offset, pos.y),
        text,
        font.clone(),
        color,
    );
    if let Some(bg) = bg {
        let line_h = ui.fonts_mut(|f| f.row_height(&font));
        fade_edge(ui, pos.x + max_width, pos.y, line_h, bg);
        if state.offset > 0.5 {
            fade_leading(ui, pos.x, pos.y, line_h, bg);
        }
    }
    Some(response)
}

/// The player bar's credits line: like [`paint_subtitle`], but scrolling on hover when the
/// credits are wider than the room — which is exactly the short-title-long-artists case.
///
/// Fits and this *is* `paint_subtitle`: same paint, same hit rects, no state. Overflows and
/// the whole line scrolls ping-pong under a clipped viewport with dissolves on both cut
/// edges. Link hit rects ride the same offset as the paint, so artists stay clickable
/// mid-scroll; a link hovered last frame counts as viewport hover, because stacked links
/// take the hover flag from the viewport beneath them and the scroll would otherwise stop
/// dead under the pointer.
/// The full laid-out width of a subtitle, separators included — what the marquee would measure
/// if nothing clipped it. The player bar's rating column hugs this, so the thumbs sit beside
/// the text rather than beside an empty field.
pub fn subtitle_width(ui: &Ui, subtitle: &Subtitle<'_>, tail: Option<&str>) -> f32 {
    let font = theme::regular(12.0);
    subtitle
        .iter()
        .flatten()
        .filter(|p| !p.text.is_empty())
        .copied()
        .chain(tail.map(Part::plain))
        .enumerate()
        .map(|(i, part)| {
            let sep = if i > 0 {
                text_width(ui, part.sep, &font)
            } else {
                0.0
            };
            sep + text_width(ui, part.text, &font)
        })
        .sum()
}

#[allow(clippy::too_many_arguments)]
pub fn paint_subtitle_marquee<'a>(
    ui: &mut Ui,
    palette: &Palette,
    pos: egui::Pos2,
    max_width: f32,
    // See `marquee_line`'s own doc on this parameter: `None` skips the dissolve rather than
    // guessing a colour a live-blurred shell background has no way to actually match.
    bg: Option<Color32>,
    subtitle: &Subtitle<'a>,
    tail: Option<&'a str>,
    id_salt: impl std::hash::Hash + Clone + std::fmt::Debug,
    // See `marquee_line`'s own doc on this parameter.
    row_hovered: bool,
) -> Option<LinkTarget<'a>> {
    let font = theme::regular(12.0);
    let visible_parts = || {
        subtitle
            .iter()
            .flatten()
            .filter(|p| !p.text.is_empty())
            .copied()
            .chain(tail.map(Part::plain))
    };
    // Measure straight off the iterator: the common case is a subtitle that fits, and it
    // returns here without ever building the `Vec`.
    let mut full = 0.0;
    for (i, part) in visible_parts().enumerate() {
        if i > 0 && !part.sep.is_empty() {
            full += text_width(ui, part.sep, &font);
        }
        full += text_width(ui, part.text, &font);
    }
    if full <= max_width {
        return paint_subtitle(ui, palette, pos, pos.x + max_width, subtitle, tail, id_salt);
    }

    let parts: Vec<Part> = visible_parts().collect();

    let id = ui.id().with(id_salt);
    let viewport = Rect::from_min_size(
        egui::pos2(pos.x, pos.y - MARQUEE_HIT_H / 2.0),
        vec2(max_width, MARQUEE_HIT_H),
    );
    let over_view = ui.interact(viewport, id, Sense::hover()).hovered();
    let now = ui.input(|i| i.time);
    let mut state: MarqueeState = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    if (state.full_w - full).abs() > 0.5 {
        state = MarqueeState {
            full_w: full,
            dir: 1.0,
            last_t: now,
            ..Default::default()
        };
    }
    // See the doc comment: the viewport loses hover to the links stacked on it.
    let hovered = over_view || state.link_hovered || row_hovered;
    if hovered {
        ui.ctx().request_repaint();
        if state.hover_since.is_none() {
            state.hover_since = Some(now);
        }
        let since = state.hover_since.unwrap_or(now);
        if now - since >= MARQUEE_DWELL_SECS && now >= state.hold_until {
            let dt = (now - state.last_t).clamp(0.0, 0.25) as f32;
            let max_off = (full - max_width).max(0.0);
            let (offset, dir) = pingpong(state.offset, state.dir, max_off, MARQUEE_SPEED * dt);
            if (dir - state.dir).abs() > f32::EPSILON {
                state.hold_until = now + MARQUEE_HOLD_SECS;
            }
            state.offset = offset;
            state.dir = dir;
        }
    } else {
        state.hover_since = None;
        state.offset = 0.0;
        state.dir = 1.0;
        state.hold_until = 0.0;
    }
    state.last_t = now;
    state.full_w = full;

    let clip = ui.clip_rect().intersect(viewport);
    let painter = ui.painter().with_clip_rect(clip);
    // No early break: the clip rect does the cutting, scrolled or not.
    let mut x = pos.x - state.offset;
    let mut clicked = None;
    let mut link_hovered = false;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 && !part.sep.is_empty() {
            painter.text(
                egui::pos2(x, pos.y),
                egui::Align2::LEFT_CENTER,
                part.sep,
                font.clone(),
                palette.dim,
            );
            x += text_width(ui, part.sep, &font);
        }
        let w = text_width(ui, part.text, &font);
        let mut colour = palette.secondary;
        if let Some(target) = part.target {
            let hit = Rect::from_min_size(egui::pos2(x, pos.y - 8.0), vec2(w, 16.0));
            let r = ui.interact(hit, ui.id().with((id, i)), Sense::click());
            if r.hovered() {
                link_hovered = true;
                colour = palette.text;
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                painter.hline(
                    x..=(x + w),
                    pos.y + 7.0,
                    egui::Stroke::new(1.0, palette.text),
                );
            }
            if r.clicked() {
                clicked = Some(target);
            }
        }
        crate::emoji::paint_clipped_line(
            ui,
            clip,
            egui::pos2(x, pos.y),
            part.text,
            font.clone(),
            colour,
        );
        x += w;
    }
    state.link_hovered = link_hovered;
    let offset = state.offset;
    ui.ctx().data_mut(|d| d.insert_temp(id, state));

    if let Some(bg) = bg {
        let line_h = ui.fonts_mut(|f| f.row_height(&font));
        fade_edge(ui, pos.x + max_width, pos.y, line_h, bg);
        if offset > 0.5 {
            fade_leading(ui, pos.x, pos.y, line_h, bg);
        }
    }
    clicked
}

/// The loading answer: a bare accent sweep along the very top edge of the window while
/// anything is in flight — above the topbar and sidebar alike, not just the content card, so
/// it reads the same regardless of which page is loading.
///
/// Time is the only state — no timers, no stored values — so this costs nothing when it is
/// not drawn, and the caller already asks for repaints while loading. Painted on the
/// foreground layer rather than into the calling `Ui`, which is what lets it sit above the
/// topbar and sidebar panels that get laid out after it without caring about draw order. No
/// track, no container — just the sweep itself, the way YouTube's own loading line reads —
/// and its ends are held clear of the window's own rounded top corners (`radius` — `0` when
/// OS-decorated or maximized, [`theme::WINDOW_RADIUS`] otherwise), which a full-bleed bar at
/// the very top would otherwise ride over.
pub fn progress_bar(ui: &Ui, palette: &Palette, radius: u8) {
    let rect = ui.ctx().content_rect();
    let (left, right) = (rect.left(), rect.right());
    if right <= left {
        return;
    }
    let y = rect.top() + 1.0;
    // Where the window's own outline begins on the bar's top row, given the corner radius:
    // the sweep emerges from behind the corner rather than crossing the curve. One row below
    // the top edge the inset is large — most of the radius — so clamping by it keeps the
    // whole 3 pt bar inside the window at both ends.
    let rad = f32::from(radius);
    let dy = rad - (y - rect.top());
    let inset = (rad - (rad * rad - dy * dy).max(0.0).sqrt()).min(rad);
    let (start, end) = (left + inset, right - inset);
    if end <= start {
        return;
    }
    let seg = 120.0;
    let travel = f64::from(right - left) + f64::from(seg);
    let t = ui.input(|i| i.time);
    let x = left - seg + ((t * 800.0) % travel) as f32;
    let (lo, hi) = (x.max(start), (x + seg).min(end));
    if hi > lo {
        ui.ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("ytm-loading-bar"),
            ))
            .rect_filled(
                Rect::from_min_size(egui::pos2(lo, y), egui::vec2(hi - lo, 3.0)),
                CornerRadius::same(2),
                palette.accent,
            );
    }
}

/// One grey placeholder bar — the raw material every skeleton below is built from.
fn skeleton_bar(ui: &mut Ui, palette: &Palette, w: f32, h: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS_SMALL),
        palette.surface_hover,
    );
}

/// Grey placeholder rows while a track list loads — static shapes, so they answer without
/// costing a single extra frame.
pub fn skeleton_rows(ui: &mut Ui, palette: &Palette, n: usize) {
    for _ in 0..n {
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), theme::ROW_HEIGHT),
            Sense::hover(),
        );
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let art = Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.center().y - theme::ART_SIZE / 2.0),
            vec2(theme::ART_SIZE, theme::ART_SIZE),
        );
        ui.painter().rect_filled(
            art,
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
        let tx = art.right() + 12.0;
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(tx, rect.center().y - 10.0),
                vec2((rect.width() * 0.4).max(80.0), 10.0),
            ),
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(tx, rect.center().y + 4.0),
                vec2((rect.width() * 0.25).max(60.0), 8.0),
            ),
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
    }
}

/// [`skeleton_rows`] for a numbered track list (Album/Playlist): adds the number column
/// `skeleton_rows` leaves out and a right-aligned duration slot, matching what [`track_row`]
/// actually paints for those pages rather than a generic row.
pub fn skeleton_track_rows(ui: &mut Ui, palette: &Palette, n: usize) {
    let duration_w = 44.0;
    let right_pad = 20.0;
    for _ in 0..n {
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), theme::ROW_HEIGHT),
            Sense::hover(),
        );
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let num_x =
            rect.left() + theme::ROW_LEFT_INSET + theme::ROW_NUMBER_W - theme::ROW_NUMBER_GAP;
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(num_x - 12.0, rect.center().y - 5.0),
                vec2(12.0, 10.0),
            ),
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
        let art = Rect::from_min_size(
            egui::pos2(
                rect.left() + theme::ROW_LEFT_INSET + theme::ROW_NUMBER_W,
                rect.center().y - theme::ART_SIZE / 2.0,
            ),
            vec2(theme::ART_SIZE, theme::ART_SIZE),
        );
        ui.painter().rect_filled(
            art,
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
        let tx = art.right() + 12.0;
        let text_w = (rect.right() - right_pad - duration_w - 8.0 - tx).max(40.0);
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(tx, rect.center().y - 10.0),
                vec2((text_w * 0.6).max(80.0), 10.0),
            ),
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(tx, rect.center().y + 4.0),
                vec2((text_w * 0.35).max(60.0), 8.0),
            ),
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(rect.right() - right_pad - duration_w, rect.center().y - 5.0),
                vec2(duration_w, 9.0),
            ),
            CornerRadius::same(theme::RADIUS_SMALL),
            palette.surface_hover,
        );
    }
}

/// Grey placeholder stripe chips, the shape [`crate::ui::mood_chips`] actually lays out — flat
/// bars with no art slot, for the Moods page (which has no cover art anywhere on it).
pub fn skeleton_chips(ui: &mut Ui, palette: &Palette, n: usize) {
    let cols = ((ui.available_width() / 180.0).floor() as usize).clamp(2, 6);
    let gap = 8.0;
    let chip_w = (ui.available_width() - (cols - 1) as f32 * gap) / cols as f32;
    let chip_h = 44.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for _ in 0..n {
            let (rect, _) = ui.allocate_exact_size(vec2(chip_w, chip_h), Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
    });
}

/// Grey placeholder cells for Charts' three-column ranked grid — [`ranked`]'s own cell shape,
/// laid out three wide instead of `skeleton_rows`' single column.
///
/// [`ranked`]: crate::ui::charts::ranked
pub fn skeleton_chart_grid(ui: &mut Ui, palette: &Palette, n: usize) {
    let gap = 12.0;
    let cols = 3;
    let cell_w = (ui.available_width() - (cols - 1) as f32 * gap) / cols as f32;
    let cell_h = 56.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for _ in 0..n {
            let (rect, _) = ui.allocate_exact_size(vec2(cell_w, cell_h), Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let art = Rect::from_min_size(rect.left_center() + vec2(4.0, -20.0), vec2(40.0, 40.0));
            ui.painter().rect_filled(
                art,
                CornerRadius::same(theme::RADIUS_SMALL),
                palette.surface_hover,
            );
            let tx = art.right() + 8.0;
            let text_w = (rect.right() - 4.0 - tx).max(30.0);
            ui.painter().rect_filled(
                Rect::from_min_size(
                    egui::pos2(tx, rect.center().y - 9.0),
                    vec2(text_w * 0.7, 9.0),
                ),
                CornerRadius::same(theme::RADIUS_SMALL),
                palette.surface_hover,
            );
            ui.painter().rect_filled(
                Rect::from_min_size(
                    egui::pos2(tx, rect.center().y + 6.0),
                    vec2(text_w * 0.45, 8.0),
                ),
                CornerRadius::same(theme::RADIUS_SMALL),
                palette.surface_hover,
            );
        }
    });
}

/// The shape a [`skeleton_shelf`] stands in for.
#[derive(Clone, Copy)]
pub enum SkeletonShelfShape {
    Cards(ArtShape),
    Rows,
}

/// A whole titled shelf's placeholder — a title-bar bar in [`crate::ui::section`]'s own spot,
/// followed by cards or rows. Covers every "titled shelf" page (Home, Explore, Releases,
/// History, an artist's shelves): those load into several of these, not one untitled block.
pub fn skeleton_shelf(ui: &mut Ui, palette: &Palette, shape: SkeletonShelfShape, n: usize) {
    skeleton_title(ui, palette);
    match shape {
        SkeletonShelfShape::Cards(art_shape) => skeleton_cards(ui, palette, n, art_shape),
        SkeletonShelfShape::Rows => skeleton_rows(ui, palette, n),
    }
}

/// A section-heading placeholder in [`crate::ui::section`]'s own spot — for a skeleton that
/// needs a title bar of its own without a whole shelf under it (Moods' chip sections).
pub fn skeleton_title(ui: &mut Ui, palette: &Palette) {
    ui.add_space(12.0);
    skeleton_bar(ui, palette, 130.0, 14.0);
    ui.add_space(4.0);
}

/// Grey placeholder cards while a grid page loads. Same contract as the rows: static.
///
/// `shape` is the shape the real cards will have, so a grid of artists does not load as
/// squares and land as circles.
pub fn skeleton_cards(ui: &mut Ui, palette: &Palette, n: usize, shape: ArtShape) {
    ui.horizontal_wrapped(|ui| {
        for _ in 0..n {
            let size = card_size(super::ui::CARD_W, shape);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let art = Rect::from_min_size(
                rect.min + vec2(8.0, 8.0),
                vec2(
                    super::ui::CARD_W - 16.0,
                    card_art_height(super::ui::CARD_W, shape),
                ),
            );
            let radius = match shape {
                ArtShape::Circle => {
                    CornerRadius::same((art.height() / 2.0).clamp(0.0, 255.0) as u8)
                }
                ArtShape::Square | ArtShape::Wide => CornerRadius::same(theme::RADIUS),
            };
            ui.painter().rect_filled(art, radius, palette.surface_hover);
            for (dy, w) in [(14.0, 0.7), (28.0, 0.45)] {
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        art.left_bottom() + vec2(0.0, dy),
                        vec2((super::ui::CARD_W - 16.0) * w, 9.0),
                    ),
                    CornerRadius::same(theme::RADIUS_SMALL),
                    palette.surface_hover,
                );
            }
        }
    });
}

/// What kind of detail header [`skeleton_header`] is standing in for — controls how many text
/// bars it draws and whether a button-row bar appears beneath them, so an album's placeholder
/// stops looking like an artist's.
#[derive(Clone, Copy)]
pub enum SkeletonHeaderKind {
    /// Round art, a name bar, one audience/subscriber-line bar — no kind label, no button row:
    /// the artist page draws its own Shuffle/Mix/Subscribe row separately.
    Artist,
    /// Square art, a kind-label bar, a title bar, an optional description bar, a credit-line
    /// bar, and a Play/Save/•••-shaped button-row bar — what the album and playlist headers
    /// actually paint.
    Detail { description: bool },
}

/// Grey placeholder header while a detail page loads — art plus however many text bars and
/// whatever button row the real header for `kind` would paint, rather than always exactly two
/// bars regardless of what's loading.
pub fn skeleton_header(ui: &mut Ui, palette: &Palette, size: f32, kind: SkeletonHeaderKind) {
    ui.horizontal(|ui| {
        let circle = matches!(kind, SkeletonHeaderKind::Artist);
        let (art_rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        let radius = if circle {
            CornerRadius::same((size / 2.0).clamp(0.0, 255.0) as u8)
        } else {
            CornerRadius::same(theme::RADIUS)
        };
        ui.painter()
            .rect_filled(art_rect, radius, palette.surface_hover);
        ui.vertical(|ui| match kind {
            SkeletonHeaderKind::Artist => {
                ui.add_space(size / 2.0 - 22.0);
                for (w, h) in [(0.5, 14.0), (0.3, 10.0)] {
                    let bar_w = (ui.available_width() * w).max(80.0);
                    skeleton_bar(ui, palette, bar_w, h);
                    ui.add_space(8.0);
                }
            }
            SkeletonHeaderKind::Detail { description } => {
                ui.add_space(8.0);
                skeleton_bar(ui, palette, (ui.available_width() * 0.15).max(50.0), 10.0);
                ui.add_space(10.0);
                skeleton_bar(ui, palette, (ui.available_width() * 0.55).max(120.0), 18.0);
                ui.add_space(10.0);
                if description {
                    skeleton_bar(ui, palette, (ui.available_width() * 0.4).max(100.0), 10.0);
                    ui.add_space(8.0);
                }
                skeleton_bar(ui, palette, (ui.available_width() * 0.35).max(90.0), 10.0);
                ui.add_space(14.0);
                skeleton_bar(ui, palette, (ui.available_width() * 0.4).max(110.0), 34.0);
            }
        });
    });
    ui.add_space(12.0);
}

/// A single track row: number (optional), art, title with an explicit badge, artist with play
/// count, duration.
///
/// Returns the response so the caller decides what a click means.
pub fn track_row<'a>(ui: &mut Ui, palette: &Palette, row: TrackRow<'a>) -> RowClick<'a> {
    let TrackRow {
        number,
        rank,
        trend,
        title,
        subtitle,
        duration,
        plays,
        explicit,
        active,
        badge,
        art,
        shape,
        id_salt,
        menu,
        like,
        show_rating,
    } = row;
    let mut link = None;
    // `click_and_drag`, not `click`: a row inside the open playlist's own editable track list
    // can be dragged to reorder it (see `spec_rows`, the only place that ever reads a drag
    // start off this response). Harmless everywhere else — nothing reads it unless that row
    // came from an editable playlist, and egui only treats a press as a drag once the pointer
    // has actually moved, so an ordinary click is unaffected.
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click_and_drag(),
    );
    let right_pad = 20.0;
    // A narrow row — the open queue on a small window — compresses instead of starving:
    // the number column and the art yield first, and what they give back is the title's
    // room. A full-size art and a 30pt number column left 21pt of it on a 215pt card,
    // which is one ellipsis. Decided here, because the duration slot below leans on it.
    let narrow = rect.width() < 340.0;
    // Measured, not constant: an hour-long mix ("1:02:05") is wider than the 44pt a
    // "3:33" needs, and a fixed slot grazes the title in the narrow queue. Nothing
    // painted, nothing reserved beyond the old minimum. On a narrow rated row the slot
    // is the thumbs' corner — the pair spans 62pt, so the text must clear it even though
    // the duration itself only shows when the thumbs do not.
    let duration_w = match duration {
        Some(d) => text_width(ui, &fmt_duration(d), &theme::regular(12.0)) + 12.0,
        None => badge.map_or(0.0, |b| text_width(ui, b, &theme::semibold(9.0)) + 12.0),
    }
    .max(if narrow && show_rating { 76.0 } else { 44.0 });
    // Reserved whether or not the thumbs are drawn, so a row does not re-truncate its title as
    // the pointer crosses it — but only where they can appear at all. The queue turns rating
    // off, and giving that panel back its 68pt is the difference between "Jigsaw Fallin…" and
    // a readable title in a 300pt rail.
    // Wide enough for the pair: two 30pt buttons at offsets 50/18 span 62pt, and anything
    // narrower paints the Like button over the title's tail.
    // ...and only where the row is wide enough to pay for the reservation. On a card squeezed
    // by the open queue — a 750pt window leaves ~200pt — reserving 68pt for hover-only
    // controls left the title two characters and the duration an island. Below the floor the
    // thumbs take the duration's corner instead (see the rating block), so nothing is
    // reserved and nothing covers the text.
    let thumb_w = if show_rating && rect.width() >= 340.0 {
        68.0
    } else {
        0.0
    };

    // Geometric, not `hovered()`: children stacked on top (links, thumbs) steal the hover
    // flag, which would blink the highlight out as the pointer crosses them.
    let over = response.contains_pointer();
    let t = fade(ui, response.id.with("row"), over);
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(theme::RADIUS);
        if t > 0.001 {
            ui.painter()
                .rect_filled(rect, radius, palette.surface_hover.gamma_multiply(t));
        }

        // A narrow row — the open queue on a small window — compresses instead of starving:
        // the number column and the art yield first, and what they give back is the title's
        // room. (Decided up with `duration_w`, which leans on it.)
        let num_w = if (number.is_some() || rank.is_some()) && !narrow {
            theme::ROW_NUMBER_W
        } else {
            0.0
        };
        let row_art = if narrow { 32.0 } else { theme::ART_SIZE };
        // A video still keeps its 16:9 aspect instead of being squeezed into the square slot.
        let art_w = if shape == ArtShape::Wide {
            row_art * 16.0 / 9.0
        } else {
            row_art
        };
        let art_rect = Rect::from_center_size(
            rect.left_center() + vec2(theme::ROW_LEFT_INSET + num_w + art_w / 2.0, 0.0),
            vec2(art_w, row_art),
        );
        paint_art(ui, palette, art_rect, art, shape);
        // A cover you can press is a control; one you cannot is a picture.
        if shape == ArtShape::Square {
            play_overlay(ui, palette, art_rect, t, false);
        }

        if !narrow && let Some(rank) = rank {
            // A chart rank takes the number's slot; the movement rides beside it.
            let (arrow, fg) = match trend {
                crate::model::ChartTrend::Up => (Some(Icon::TrendingUp), palette.accent),
                crate::model::ChartTrend::Down => (Some(Icon::TrendingDown), palette.danger),
                crate::model::ChartTrend::Steady => (None, palette.dim),
                crate::model::ChartTrend::Unknown => (None, palette.dim),
            };
            let label_x = rect.left() + theme::ROW_LEFT_INSET + num_w - theme::ROW_NUMBER_GAP;
            ui.painter().text(
                egui::pos2(label_x, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                rank,
                theme::semibold(12.0),
                fg,
            );
            if let Some(arrow) = arrow {
                let w = text_width(ui, rank, &theme::semibold(12.0));
                arrow.image(fg, 11.0).paint_at(
                    ui,
                    Rect::from_min_size(
                        // Clamped: a rank's arrow sits left of the number, and the number now
                        // hugs the row's left edge, so without this a two-digit rank would push
                        // the arrow off the row.
                        egui::pos2(
                            (label_x - w - 15.0).max(rect.left() + 1.0),
                            rect.center().y - 5.5,
                        ),
                        vec2(11.0, 11.0),
                    ),
                );
            }
        } else if !narrow && let Some(n) = number {
            ui.painter().text(
                egui::pos2(
                    rect.left() + theme::ROW_LEFT_INSET + num_w - theme::ROW_NUMBER_GAP,
                    rect.center().y,
                ),
                egui::Align2::RIGHT_CENTER,
                n.to_string(),
                theme::regular(12.0),
                palette.dim,
            );
        }

        let text_x = art_rect.right() + 12.0;

        // Explicit badge first, at a fixed spot so the title never shifts when art loads.
        let mut title_x = text_x;
        if explicit {
            let badge =
                Rect::from_min_size(egui::pos2(text_x, rect.center().y - 17.0), vec2(15.0, 15.0));
            ui.painter().rect(
                badge,
                CornerRadius::same(3),
                Color32::TRANSPARENT,
                egui::Stroke::new(1.0, palette.dim),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "E",
                theme::semibold(9.0),
                palette.dim,
            );
            title_x += 19.0;
        }
        let text_w = (rect.right() - right_pad - duration_w - thumb_w - title_x).max(0.0);

        let title_color = if active { palette.accent } else { palette.text };
        // A long title scrolls on hover instead of hard-truncating; a title that fits is painted
        // plainly. Hover-only sense, so the row's own click is never stolen.
        //
        // The fade dissolve at the scroll edges is skipped in translucent mode: `surface_hover`
        // is a real opaque highlight painted over the whole row regardless of opacity settings,
        // so it stays safe to dissolve into, but `row_fill()` is only ever a guess at what a
        // live-blurred shell shows through — see `Palette::row_fill`'s own doc for why an
        // opaque *or* translucent guess there both painted something visibly wrong.
        let marquee_bg = if over {
            Some(palette.surface_hover)
        } else if palette.translucent() {
            None
        } else {
            Some(palette.row_fill())
        };
        let _ = marquee_line(
            ui,
            egui::pos2(title_x, rect.center().y - 9.0),
            text_w,
            title,
            theme::medium(13.0),
            title_color,
            marquee_bg,
            ("ytm-row-title", id_salt),
            Sense::hover(),
            over,
        );
        // Subtitle pieces, laid out left to right so each linked piece gets its own hit area.
        // The play-count tail rides the same line and scrolls with it.
        let sub_right = rect.right() - right_pad - duration_w - thumb_w;
        let sub_w = (sub_right - text_x).max(0.0);
        if let Some(target) = paint_subtitle_marquee(
            ui,
            palette,
            egui::pos2(text_x, rect.center().y + 9.0),
            sub_w,
            marquee_bg,
            &subtitle,
            plays,
            ("ytm-row-link", id_salt),
            over,
        ) {
            link = Some(target);
        }

        // Duration when there is one, otherwise the type badge. They occupy the same slot because
        // a row has one or the other: only songs carry a duration, only mixed lists need a badge.
        // On a narrow row the rating thumbs live in this corner instead of beside the text, so
        // the slot steps aside for as long as a thumb is showing — a liked row keeps its thumb
        // here at rest, and the duration returns when the pointer moves on.
        let rated = matches!(
            like,
            crate::model::LikeStatus::Like | crate::model::LikeStatus::Dislike
        );
        let thumb_corner = narrow && show_rating && (over || rated);
        if let Some(d) = duration
            && !thumb_corner
        {
            ui.painter().text(
                egui::pos2(rect.right() - right_pad, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                fmt_duration(d),
                theme::regular(12.0),
                palette.dim,
            );
        } else if !thumb_corner && let Some(badge) = badge {
            ui.painter().text(
                egui::pos2(rect.right() - right_pad, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                badge,
                theme::semibold(9.0),
                palette.dim,
            );
        }
    }

    // The menu lives outside the visibility check: allocation and interaction happen whether
    // the row is painted or not, and a menu opened at the pointer must survive the row
    // scrolling out from under it.
    let ratable = menu
        .as_ref()
        .filter(|_| show_rating)
        .map(|m| (m.track.video_id.clone(), m.signed_in));
    let mut action = menu.and_then(|m| row_menu(ui, palette, &response, m));
    // Rating thumbs are interacted whenever the row is visible — never gated on drawn — for
    // the reason documented on `rate_thumbs`. A click is only reachable where they are
    // visible anyway, since their zone sits inside the row.
    if ui.is_rect_visible(rect)
        && let Some((id, true)) = ratable
    {
        // Wide rows reserve a 68pt zone left of the duration and anchor the thumbs there.
        // Narrow rows have no zone to anchor to — the thumbs take the duration's corner
        // instead, where the duration itself has just stepped aside for them.
        let thumb_right = if rect.width() < 340.0 {
            rect.right() - 17.0
        } else {
            rect.right() - right_pad - duration_w
        };
        if let Some(rate) = rate_thumbs(ui, palette, rect, thumb_right, like, &id, id_salt, over) {
            action = Some(rate);
        }
    }
    RowClick {
        response,
        link,
        action,
    }
}

/// A song's right-click menu, reporting the choice. Shared by every row and the player bar.
///
/// The popup id is the video id, not the row index: indices shift as the virtualised range
/// moves, which would close the menu on scroll — or worse, re-anchor it onto another row.
/// Two rows for the same track share one menu, which is the same menu, so that is harmless.
pub(crate) fn row_menu(
    ui: &mut Ui,
    palette: &Palette,
    response: &egui::Response,
    menu: RowMenu<'_>,
) -> Option<super::ui::RowAction> {
    use super::ui::RowAction;
    let t = menu.track;
    let popup = egui::Popup::new(
        ui.id().with(("ytm-row-menu", t.video_id.as_str())),
        ui.ctx().clone(),
        response,
        ui.layer_id(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .width(MENU_WIDTH)
    .gap(0.0)
    // The default is `CloseOnClick`, which closes on *any* click anywhere — including one
    // landing inside the menu, so a click on the submenu's scrollbar tore the whole thing down.
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .open_memory(if response.secondary_clicked() {
        Some(egui::SetOpenCommand::Bool(true))
    } else if response.clicked() {
        // A primary click both plays and dismisses, as on the site.
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    })
    .at_pointer_fixed();
    popup
        .show(|mui| {
            let mut choice = None;

            // Block one: what to do with it now. No title header — you know which row you
            // right-clicked, and a truncated one only ate a row's worth of height.
            if menu_item(mui, palette, Icon::ListStart, "Play next", None).clicked() {
                choice = Some(RowAction::PlayNext(t.clone()));
                mui.close();
            }
            if menu_item(mui, palette, Icon::ListPlus, "Add to queue", None).clicked() {
                choice = Some(RowAction::AddToQueue(t.clone()));
                mui.close();
            }
            if menu_item(mui, palette, Icon::Radio, "Start mix", None).clicked() {
                choice = Some(RowAction::StartMix(t.video_id.clone()));
                mui.close();
            }
            if let Some(i) = menu.queue_index
                && menu_item(mui, palette, Icon::ListX, "Remove from queue", None).clicked()
            {
                choice = Some(RowAction::RemoveFromQueue(i));
                mui.close();
            }

            // Block two: where it lives. Save to library / Remove from library, when the row
            // carries the token for the direction it needs. Signed-out rows carry no tokens,
            // so the item hides itself.
            let saving = !t.in_library;
            let token = if saving {
                &t.library_add
            } else {
                &t.library_remove
            };
            // Only the playlists you can actually add to, and only ones of the same source as
            // this track — YouTube's API has no way to accept a local file, and a local
            // playlist has nowhere to send a YouTube video id, so offering the other source
            // would be an option that always fails. Liked Music is an auto playlist and
            // someone else's saved playlist is not yours to edit either; both would have been
            // offered and both would have been refused by YouTube.
            let wanted_source = if t.local_path.is_some() {
                crate::model::PlaylistSource::Local
            } else {
                crate::model::PlaylistSource::YouTube
            };
            let targets: Vec<&crate::model::PlaylistRef> = menu
                .playlists
                .iter()
                .filter(|p| p.editable && p.source == wanted_source)
                .collect();
            let removable = menu.playlist_id.zip(t.set_video_id.as_deref());
            if token.is_some() || !targets.is_empty() || removable.is_some() {
                mui.add_space(8.0);
            }
            if token.is_some()
                && menu_item(
                    mui,
                    palette,
                    Icon::Bookmark,
                    if saving {
                        "Save to library"
                    } else {
                        "Remove from library"
                    },
                    None,
                )
                .clicked()
            {
                choice = Some(RowAction::ToggleLibrary(t.clone()));
                mui.close();
            }
            if !targets.is_empty()
                && let Some(playlist_id) = sub_menu(
                    mui,
                    palette,
                    Icon::Plus,
                    "Save to playlist",
                    mui.id().with(("ytm-save-to", t.video_id.as_str())),
                    |sui| {
                        let mut picked = None;
                        egui::ScrollArea::vertical()
                            .id_salt("save-to-scroll")
                            .max_height(320.0)
                            .show(sui, |sui| {
                                for pl in targets {
                                    if menu_item(sui, palette, Icon::ListMusic, &pl.title, None)
                                        .clicked()
                                    {
                                        picked = Some(pl.id.clone());
                                    }
                                }
                            });
                        picked
                    },
                )
            {
                choice = Some(RowAction::SaveToPlaylist {
                    playlist_id,
                    video_id: t.video_id.clone(),
                });
                mui.close();
            }
            // Remove from playlist, only on an editable page and only where the item id is
            // actually present — a wrong assumption here corrupts a playlist, so both gates
            // must pass.
            if let Some((playlist_id, set_video_id)) = removable
                && menu_item(mui, palette, Icon::ListX, "Remove from playlist", None).clicked()
            {
                choice = Some(RowAction::RemoveFromPlaylist {
                    playlist_id: playlist_id.to_owned(),
                    video_id: t.video_id.clone(),
                    set_video_id: set_video_id.to_owned(),
                });
                mui.close();
            }

            // Block three: where it came from, and sending it on.
            mui.add_space(8.0);
            if let Some(id) = &t.album_id
                && menu_item(mui, palette, Icon::Disc3, "Go to album", None).clicked()
            {
                choice = Some(RowAction::GoToAlbum(id.clone()));
                mui.close();
            }
            if let Some(id) = t.primary_artist_id()
                && menu_item(mui, palette, Icon::CircleUser, "Go to artist", None).clicked()
            {
                choice = Some(RowAction::GoToArtist(id.to_owned()));
                mui.close();
            }
            // There is no real YouTube URL for a local track to share.
            if t.local_path.is_none()
                && menu_item(mui, palette, Icon::Share2, "Share", None).clicked()
            {
                choice = Some(RowAction::Share(t.video_id.clone()));
                mui.close();
            }
            choice
        })
        .and_then(|r| r.inner)
}

/// What a page header's ••• menu is built from: the whole loaded list, not one track.
///
/// Queueing and saving act on every loaded track — a paged playlist only has its loaded pages —
/// and every capability gates itself on the data, so a signed-out page offers no save picker
/// and a description-less page offers no Info.
pub struct PageMenu<'a> {
    /// Stable popup id: the album or playlist id. Row menus key on video id for the same
    /// reason — indices shift, ids do not.
    pub page_id: &'a str,
    pub tracks: &'a [crate::model::Track],
    /// The artist the page is by, when the source linked one. Enables "Go to artist".
    pub artist_id: Option<&'a str>,
    /// Your playlists, for the "Save to playlist" picker. Empty when signed out.
    pub playlists: &'a [crate::model::PlaylistRef],
    pub signed_in: bool,
    /// A `watch?v=` URL is wrong for a page; the caller builds the right one.
    pub share_url: String,
    /// The page has a description worth showing. Enables "Info".
    pub has_info: bool,
    /// Set on an owned playlist page: enables "Edit playlist" and "Delete playlist".
    pub manage: bool,
    /// A saved (not owned) playlist in the rail: enables "Remove from library". The pages
    /// already carry a Save/Remove chip, so they leave this false.
    pub remove_from_library: bool,
    /// Opened from a right-click (the sidebar rail) rather than a ••• button: only a secondary
    /// click opens it, and a left click (which navigates) closes it instead of toggling.
    pub context_only: bool,
    /// Offers "Save as playlist" first, above everything else, seeded with the whole page and
    /// defaulting to this name — the Local tab's folder cards, which aren't playlists until
    /// this is actually chosen. `None` everywhere else; the existing "Save to playlist"
    /// (`targets`, below) already covers saving into a playlist that exists.
    pub save_as_new: Option<&'a str>,
}

/// A page header's ••• menu, reporting the choice.
///
/// Order: save-as-new (only where `save_as_new` is set), mix, shuffle, queueing, saving,
/// navigation, share, info. Mix leads among the rest — starting a mix is the one worth
/// reaching for first when both are on offer. No block separators — menus carry none anywhere
/// since the divider cleanup.
pub(crate) fn page_menu(
    ui: &mut Ui,
    palette: &Palette,
    response: &egui::Response,
    menu: PageMenu<'_>,
) -> Option<super::ui::RowAction> {
    use super::ui::RowAction;
    let popup = egui::Popup::new(
        ui.id().with(("ytm-page-menu", menu.page_id)),
        ui.ctx().clone(),
        response,
        ui.layer_id(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .width(MENU_WIDTH)
    .gap(0.0)
    // Toggle on primary click: this opens from a ••• button, not from a right-click like
    // the row menus. Anchored to the button (not `at_pointer_fixed`): `Toggle` flips the
    // open flag but records no pointer position, and a pointer-anchored popup with no
    // recorded position never renders — the menu opened invisibly. The context-menu
    // pattern (primary closes) would force it shut on the opening click instead.
    .open_memory(if response.secondary_clicked() {
        Some(egui::SetOpenCommand::Bool(true))
    } else if menu.context_only {
        // A rail context menu: the left click navigates, so it only closes the menu.
        response
            .clicked()
            .then_some(egui::SetOpenCommand::Bool(false))
    } else {
        response.clicked().then_some(egui::SetOpenCommand::Toggle)
    });
    popup
        .show(|mui| {
            let mut choice = None;
            // Cloned on pick, not per frame: the menu borrows the page's tracks.
            let tracks = || menu.tracks.to_vec();

            if let Some(title) = menu.save_as_new
                && !menu.tracks.is_empty()
                && menu_item(mui, palette, Icon::ListPlus, "Save as playlist", None).clicked()
            {
                choice = Some(RowAction::PageSaveAsNewPlaylist {
                    title: title.to_owned(),
                    video_ids: menu.tracks.iter().map(|t| t.video_id.clone()).collect(),
                });
                mui.close();
            }
            if let Some(first) = menu.tracks.first()
                && menu_item(mui, palette, Icon::Radio, "Start mix", None).clicked()
            {
                choice = Some(RowAction::StartMix(first.video_id.clone()));
                mui.close();
            }
            if !menu.tracks.is_empty()
                && menu_item(mui, palette, Icon::Shuffle, "Shuffle play", None).clicked()
            {
                choice = Some(RowAction::PageShuffle(tracks()));
                mui.close();
            }
            if !menu.tracks.is_empty() {
                if menu_item(mui, palette, Icon::ListStart, "Play next", None).clicked() {
                    choice = Some(RowAction::PagePlayNext(tracks()));
                    mui.close();
                }
                if menu_item(mui, palette, Icon::ListPlus, "Add to queue", None).clicked() {
                    choice = Some(RowAction::PageQueue(tracks()));
                    mui.close();
                }
            }
            // Only the playlists you can actually add to, and only ones of the same source —
            // same rule as a row's picker, and for the same reason. Signed-out pages carry no
            // playlists, so the item hides itself with no auth check.
            let wanted_source = if menu.tracks.iter().any(|t| t.local_path.is_some()) {
                crate::model::PlaylistSource::Local
            } else {
                crate::model::PlaylistSource::YouTube
            };
            let targets: Vec<&crate::model::PlaylistRef> = menu
                .playlists
                .iter()
                .filter(|p| p.editable && p.source == wanted_source)
                .collect();
            if !targets.is_empty()
                && !menu.tracks.is_empty()
                && let Some((playlist_id, title)) = sub_menu(
                    mui,
                    palette,
                    Icon::Plus,
                    "Save to playlist",
                    mui.id().with(("ytm-page-save-to", menu.page_id)),
                    |sui| {
                        let mut picked = None;
                        egui::ScrollArea::vertical()
                            .id_salt(("ytm-page-save-scroll", menu.page_id))
                            .max_height(320.0)
                            .show(sui, |sui| {
                                for pl in targets {
                                    if menu_item(sui, palette, Icon::ListMusic, &pl.title, None)
                                        .clicked()
                                    {
                                        picked = Some((pl.id.clone(), pl.title.clone()));
                                    }
                                }
                            });
                        picked
                    },
                )
            {
                choice = Some(RowAction::PageSaveToPlaylist {
                    playlist_id,
                    title,
                    video_ids: menu.tracks.iter().map(|t| t.video_id.clone()).collect(),
                });
                mui.close();
            }
            if let Some(id) = menu.artist_id
                && menu_item(mui, palette, Icon::CircleUser, "Go to artist", None).clicked()
            {
                choice = Some(RowAction::GoToArtist(id.to_owned()));
                mui.close();
            }
            if !menu.share_url.is_empty()
                && menu_item(mui, palette, Icon::Share2, "Share", None).clicked()
            {
                choice = Some(RowAction::PageShare(menu.share_url.clone()));
                mui.close();
            }
            if menu.has_info && menu_item(mui, palette, Icon::Info, "Info", None).clicked() {
                choice = Some(RowAction::PageInfo);
                mui.close();
            }
            if menu.manage {
                if menu_item(mui, palette, Icon::Pencil, "Edit playlist", None).clicked() {
                    choice = Some(RowAction::EditPlaylist(menu.page_id.to_owned()));
                    mui.close();
                }
                if menu_item(mui, palette, Icon::Trash2, "Delete playlist", None).clicked() {
                    choice = Some(RowAction::DeletePlaylist(menu.page_id.to_owned()));
                    mui.close();
                }
            }
            if menu.remove_from_library
                && menu_item(mui, palette, Icon::ListX, "Remove from library", None).clicked()
            {
                choice = Some(RowAction::UnsavePlaylist(menu.page_id.to_owned()));
                mui.close();
            }
            choice
        })
        .and_then(|r| r.inner)
}

/// A submenu row that opens a nested menu on hover, and the choice made in it.
///
/// **Never `Popup::open_id` from inside a popup.** `egui::Memory` keeps exactly one popup slot
/// per viewport, so `open_id` *replaces* whatever is open — which meant hovering this row evicted
/// the parent menu's own entry, and on the next frame the parent's `keep_popup_open` was a no-op
/// and the whole menu vanished. The menu opens at the pointer and this row sits some 150pt below
/// it, so any downward mouse travel swept across it and killed the menu: "moving the mouse over
/// the songs closes it". egui's own `SubMenuButton` keeps its state in `ctx.data` and passes
/// `.open(bool)`, which is what this does.
pub(crate) fn sub_menu<T>(
    ui: &mut Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    id: egui::Id,
    content: impl FnOnce(&mut Ui) -> Option<T>,
) -> Option<T> {
    let row = menu_item(ui, palette, icon, label, Some("›"));
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if row.hovered() {
        open = true;
    }
    if !open {
        ui.ctx().data_mut(|d| d.insert_temp(id, false));
        return None;
    }

    let popup = egui::Popup::from_response(&row)
        .id(id)
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .align(egui::RectAlign::RIGHT_START)
        .width(MENU_WIDTH)
        .gap(2.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open(true);
    let Some(inner) = popup.show(content) else {
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
        return None;
    };

    // Closed when the pointer is over neither the row nor the submenu. Nothing else would take
    // it down, and it must not be the memory slot that does it.
    let pointer = ui.ctx().pointer_latest_pos();
    let inside = pointer
        .is_some_and(|p| row.rect.expand(4.0).contains(p) || inner.response.rect.contains(p));
    ui.ctx().data_mut(|d| d.insert_temp(id, inside));
    inner.inner
}

/// How wide a song menu is. Fixed, so every row's labels line up and the popup does not
/// resize itself as items appear and disappear with the row's capabilities.
pub(crate) const MENU_WIDTH: f32 = 236.0;

/// How wide a [`dropdown`]'s popup is. Fixed for the same reason a menu is — nine scheme
/// labels should not make the box dance open to the width of whichever is longest.
const DROPDOWN_WIDTH: f32 = 216.0;

/// Like/Dislike thumbs in the row's right-hand slot, left of the duration.
///
/// Like/Dislike thumbs in the row's right-hand slot, left of the duration.
///
/// Drawn when the pointer is geometrically over the row *or* the track is already rated —
/// which is also what the site does. The draw condition must be geometric
/// (`contains_pointer`, not `hovered`): egui hovers only the topmost stacked widget, so a
/// condition on the row's own hover blinks the thumbs out the moment the pointer crosses
/// into their zone, and they oscillate every frame after that. Interaction is never gated
/// on drawn for the same reason; a click is only reachable where the thumbs are visible
/// anyway, since their zone sits inside the row. Pressing a thumb toggles: pressing Like
/// on a liked song un-likes it.
///
/// One known gap: a right-click landing exactly in the thumb zone does not open the row
/// menu — the thumbs sit on top and swallow it. The zone is 68px of a full-width row, so
/// right-clicking the title always works.
#[allow(clippy::too_many_arguments)]
fn rate_thumbs(
    ui: &mut Ui,
    palette: &Palette,
    rect: Rect,
    thumb_right: f32,
    like: crate::model::LikeStatus,
    video_id: &str,
    id_salt: usize,
    over: bool,
) -> Option<super::ui::RowAction> {
    use super::ui::RowAction;
    use crate::model::LikeStatus;
    let size = 30.0;
    let mut choice = None;
    for (up, pressed, label, dx) in [
        (true, LikeStatus::Like, "like", 50.0),
        (false, LikeStatus::Dislike, "dislike", 18.0),
    ] {
        let hit_rect = Rect::from_center_size(
            egui::pos2(thumb_right - dx, rect.center().y),
            vec2(size, size),
        );
        // At rest a row shows only the rating it *has* — a liked song shows one filled thumb,
        // not a filled one beside an empty one. Both appear together under the pointer, which
        // is the only time the other one can be clicked anyway. A column of paired thumbs down
        // a library of liked songs was the single noisiest thing on the page.
        let set = like == pressed;
        let hit = rate_button(
            ui,
            palette,
            hit_rect,
            ui.id().with(("ytm-row-thumb", id_salt, label)),
            up,
            set,
            over || set,
        );
        if hit.clicked() {
            choice = Some(RowAction::Rate {
                video_id: video_id.to_owned(),
                status: like.toggled(pressed),
            });
        }
    }
    choice
}

/// How tall a card's cover is: full width less the inset, or 16:9 of that for a video still.
fn card_art_height(width: f32, shape: ArtShape) -> f32 {
    let w = width - 16.0;
    match shape {
        ArtShape::Wide => w * 9.0 / 16.0,
        _ => w,
    }
}

/// The space a card occupies: an 8pt top inset, the cover, and a 52pt two-line label band.
///
/// One function because two places must agree — [`art_card`] allocates it and
/// [`crate::ui::card_is_visible`] reserves it, and a card whose visibility math disagrees
/// with its allocation either fetches art it never draws or pops when the art lands.
pub fn card_size(width: f32, shape: ArtShape) -> egui::Vec2 {
    vec2(width, 8.0 + card_art_height(width, shape) + 52.0)
}

/// A clickable card for grids: rounded cover over a title and subtitle.
///
/// Allocates a square cover plus a two-line label band — or a 16:9 cover in a shorter card for
/// [`ArtShape::Wide`] — so rows never reflow when art lands.
#[allow(clippy::too_many_arguments)]
pub fn art_card(
    ui: &mut Ui,
    palette: &Palette,
    width: f32,
    texture: Option<&egui::TextureHandle>,
    title: &str,
    subtitle: &str,
    shape: ArtShape,
    menu: Option<CardMenu<'_>>,
) -> CardClick {
    let size = card_size(width, shape);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(theme::RADIUS);
        let t = lit(ui, &response);
        if t > 0.001 {
            // Lifts: a fill plus a shadow underneath, both fading with the hover.
            ui.painter().add(egui::epaint::RectShape::filled(
                rect,
                radius,
                palette.surface_hover.gamma_multiply(t),
            ));
        }
        let art = Rect::from_min_size(
            rect.min + vec2(8.0, 8.0),
            vec2(width - 16.0, card_art_height(width, shape)),
        );
        paint_art(ui, palette, art, texture, shape);
        // Circles are people — an artist has no single thing to play, so no play button.
        if shape != ArtShape::Circle {
            play_overlay(ui, palette, art, t, true);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let label_w = width - 16.0;
        crate::emoji::paint_line(
            ui,
            art.left_bottom() + vec2(0.0, 14.0),
            &truncate_to(ui, title, theme::medium(13.0), label_w),
            theme::medium(13.0),
            palette.text,
        );
        crate::emoji::paint_line(
            ui,
            art.left_bottom() + vec2(0.0, 28.0),
            &truncate_to(ui, subtitle, theme::regular(12.0), label_w),
            theme::regular(12.0),
            palette.secondary,
        );
    }
    // Outside the visibility check, as the row menu is: a menu opened at the pointer must
    // survive its card scrolling out from under it.
    let action = menu.and_then(|m| card_menu(ui, palette, &response, m));
    CardClick { response, action }
}

/// A folder card for the Local tab's file-explorer-style browser — always a plain folder
/// glyph (a folder has no cover to fetch), its name, and how many tracks sit inside it,
/// recursively. Left click opens it; right click offers `menu`, built by the caller from
/// [`PageMenu`] with `context_only: true` — the same right-click contract the sidebar rail's
/// own playlist rows use, so "Save as playlist" plus the ordinary shuffle/queue/save controls
/// are available without opening the folder first.
///
/// A sibling to [`art_card`] rather than a texture-carrying case of it — there is no thumbnail
/// concept for a folder, so this skips straight to painting the glyph, and skips the play
/// overlay `art_card` always draws: a folder isn't itself playable by clicking its cover, only
/// through the menu's own Shuffle/Play controls.
pub fn folder_card(
    ui: &mut Ui,
    palette: &Palette,
    width: f32,
    name: &str,
    track_count: usize,
    menu: PageMenu<'_>,
) -> CardClick {
    let shape = ArtShape::Square;
    let size = card_size(width, shape);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(theme::RADIUS);
        let t = lit(ui, &response);
        if t > 0.001 {
            ui.painter().add(egui::epaint::RectShape::filled(
                rect,
                radius,
                palette.surface_hover.gamma_multiply(t),
            ));
        }
        let art = Rect::from_min_size(
            rect.min + vec2(8.0, 8.0),
            vec2(width - 16.0, card_art_height(width, shape)),
        );
        ui.painter()
            .rect_filled(art, shape.radius(art), palette.surface);
        let g = art.width() * 0.5;
        Icon::Folder
            .image(palette.dim, g)
            .paint_at(ui, Rect::from_center_size(art.center(), vec2(g, g)));
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let label_w = width - 16.0;
        crate::emoji::paint_line(
            ui,
            art.left_bottom() + vec2(0.0, 14.0),
            &truncate_to(ui, name, theme::medium(13.0), label_w),
            theme::medium(13.0),
            palette.text,
        );
        let subtitle = format!("{track_count} tracks");
        crate::emoji::paint_line(
            ui,
            art.left_bottom() + vec2(0.0, 28.0),
            &truncate_to(ui, &subtitle, theme::regular(12.0), label_w),
            theme::regular(12.0),
            palette.secondary,
        );
    }
    let action = page_menu(ui, palette, &response, menu);
    CardClick { response, action }
}

/// What a card reports: the click, and any menu choice.
pub struct CardClick {
    pub response: egui::Response,
    pub(crate) action: Option<super::ui::RowAction>,
}

/// What a card's right-click menu is built from.
pub enum CardMenu<'a> {
    /// A song or video card: the same menu its row would have.
    Track(RowMenu<'a>),
    /// An artist card: what the artist's own menu on the site offers.
    Artist(&'a crate::model::ArtistRef),
    /// An album or playlist card: what can be done with a whole release.
    Release {
        /// A name for the toast, and the id the menu shares.
        title: &'a str,
        share_id: &'a str,
    },
}

/// A card's right-click menu, dispatched by what the card is.
///
/// Cards had no menu at all, which is why right-clicking worked on an artist page's Top songs
/// and did nothing on its Videos shelf — the same songs, drawn as cards.
fn card_menu(
    ui: &mut Ui,
    palette: &Palette,
    response: &egui::Response,
    menu: CardMenu<'_>,
) -> Option<super::ui::RowAction> {
    use super::ui::RowAction;
    match menu {
        CardMenu::Track(row) => row_menu(ui, palette, response, row),
        CardMenu::Artist(artist) => {
            let popup = card_popup(ui, response, ("ytm-artist-menu", artist.id.as_str()));
            popup
                .show(|mui| {
                    let mut choice = None;
                    if let Some(target) = &artist.actions.mix
                        && menu_item(mui, palette, Icon::Radio, "Start mix", None).clicked()
                    {
                        choice = Some(RowAction::PlayTarget {
                            seed: artist.id.clone(),
                            target: target.clone(),
                        });
                        mui.close();
                    }
                    if let Some(target) = &artist.actions.shuffle
                        && menu_item(mui, palette, Icon::Shuffle, "Shuffle play", None).clicked()
                    {
                        choice = Some(RowAction::PlayTarget {
                            seed: artist.id.clone(),
                            target: target.clone(),
                        });
                        mui.close();
                    }
                    if let Some(subscribed) = artist.actions.subscribed {
                        mui.add_space(8.0);
                        let label = if subscribed {
                            "Unsubscribe"
                        } else {
                            "Subscribe"
                        };
                        if menu_item(mui, palette, Icon::CircleUser, label, None).clicked() {
                            choice = Some(RowAction::Subscribe {
                                channel_id: artist.id.clone(),
                                subscribe: !subscribed,
                            });
                            mui.close();
                        }
                    }
                    mui.add_space(8.0);
                    if menu_item(mui, palette, Icon::Share2, "Share", None).clicked() {
                        // Not `RowAction::Share`: that builds a `watch?v=` URL, right for a
                        // track's video id but wrong for a channel id.
                        choice = Some(RowAction::PageShare(crate::model::artist_url(&artist.id)));
                        mui.close();
                    }
                    if let Some(token) = &artist.actions.dont_recommend
                        && menu_item(mui, palette, Icon::ListX, "Don't recommend artist", None)
                            .clicked()
                    {
                        choice = Some(RowAction::DontRecommend {
                            token: token.to_string(),
                            name: artist.name.clone(),
                        });
                        mui.close();
                    }
                    choice
                })
                .and_then(|r| r.inner)
        }
        CardMenu::Release { title: _, share_id } => {
            let popup = card_popup(ui, response, ("ytm-release-menu", share_id));
            popup
                .show(|mui| {
                    let mut choice = None;
                    if menu_item(mui, palette, Icon::Share2, "Share", None).clicked() {
                        choice = Some(RowAction::Share(share_id.to_owned()));
                        mui.close();
                    }
                    choice
                })
                .and_then(|r| r.inner)
        }
    }
}

/// The popup a card menu opens in — same shape as a row's, keyed on the card's own id rather
/// than its index, which shifts as a wrapped grid reflows.
fn card_popup<'a>(
    ui: &Ui,
    response: &'a egui::Response,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Popup<'a> {
    egui::Popup::new(ui.id().with(id), ui.ctx().clone(), response, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .width(MENU_WIDTH)
        .gap(0.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_memory(if response.secondary_clicked() {
            Some(egui::SetOpenCommand::Bool(true))
        } else if response.clicked() {
            Some(egui::SetOpenCommand::Bool(false))
        } else {
            None
        })
        .at_pointer_fixed()
}

/// How wide `text` renders in `font`. One font lock, as with [`truncate_to`].
pub fn text_width(ui: &Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts_mut(|f| text.chars().map(|c| f.glyph_width(font, c)).sum())
}

/// Shorten text with an ellipsis so it fits `max_width`.
///
/// egui truncates inside its own layout, but these rows are painted directly, so the measuring
/// has to happen here.
pub fn truncate_to<'t>(
    ui: &Ui,
    text: &'t str,
    font: egui::FontId,
    max_width: f32,
) -> std::borrow::Cow<'t, str> {
    use std::borrow::Cow;
    // No room is not "no bound". A piece past its line's right edge — a subtitle whose
    // separator ate the last of the width — must vanish, not paint the whole string over
    // whatever sits beyond the row. Every caller computes this from real geometry, so a
    // zero here always means "nothing fits".
    if max_width <= 0.0 {
        return Cow::Borrowed("");
    }
    // One lock for the whole measurement. Taking it per glyph — as this used to — is tens of
    // thousands of acquisitions per frame on a long list, twice per row. The common case —
    // text that already fits — is returned borrowed, with no allocation at all.
    ui.fonts_mut(|f| {
        let total: f32 = text.chars().map(|c| f.glyph_width(&font, c)).sum();
        if total <= max_width {
            return Cow::Borrowed(text);
        }
        let ellipsis_w = f.glyph_width(&font, '…');
        let mut out = String::with_capacity(text.len());
        let mut used = 0.0;
        for ch in text.chars() {
            let w = f.glyph_width(&font, ch);
            if used + w + ellipsis_w > max_width {
                break;
            }
            out.push(ch);
            used += w;
        }
        out.push('…');
        Cow::Owned(out)
    })
}

/// Text in the accent colour on a pill, used for primary actions.
pub fn pill_button(ui: &mut Ui, palette: &Palette, label: &str) -> egui::Response {
    pill(ui, palette, label, PillStyle::Accent)
}

/// A filter chip: a pill that carries a selected state.
///
/// Selected inverts — the label's colour becomes the fill and the window's becomes the label —
/// which is how a chip row reads at a glance without needing an outline or a checkmark.
pub fn chip(ui: &mut Ui, palette: &Palette, label: &str, selected: bool) -> egui::Response {
    pill(
        ui,
        palette,
        label,
        if selected {
            PillStyle::Selected
        } else {
            PillStyle::Quiet
        },
    )
}

/// A chip with a colour dot ahead of its label — the scheme picker, where the label alone says
/// nothing about what choosing it does. Same two states as [`chip`], same sizing and press
/// feel, just wider by the dot.
pub fn swatch_chip(
    ui: &mut Ui,
    palette: &Palette,
    swatch: Color32,
    label: &str,
    selected: bool,
    chevron: bool,
) -> egui::Response {
    let font = theme::semibold(13.0);
    // See `pill`'s own comment: a galley laid out with a real colour ignores the colour handed
    // to `painter.galley`, which is how a selected chip once painted its label invisibly.
    let galley =
        ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER));
    const DOT: f32 = 9.0;
    // A chevron rides 20pt past the label's last letter and needs its own room — a dropdown
    // whose arrow lands on its label is not a dropdown, it is a smudge.
    const CHEVRON: f32 = 20.0;
    let size = vec2(
        galley.size().x + DOT + 34.0 + u8::from(chevron) as f32 * CHEVRON,
        32.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        let (fill, fg) = if selected {
            (mix(palette.text, Color32::WHITE, t * 0.4), palette.window)
        } else {
            (
                mix(palette.surface, palette.surface_hover, t),
                mix(palette.secondary, palette.text, t),
            )
        };
        let fill = if held(&response) {
            fill.gamma_multiply(0.85)
        } else {
            fill
        };
        ui.painter().rect_filled(rect, CornerRadius::same(16), fill);
        let dot_center = rect.left_center() + vec2(16.0, 0.0);
        ui.painter().circle_filled(dot_center, DOT / 2.0, swatch);
        let text_pos = dot_center + vec2(DOT / 2.0 + 8.0, -galley.size().y / 2.0);
        ui.painter()
            .galley(text_pos + press_offset(&response), galley, fg);
        if chevron {
            // A drawn chevron rather than a glyph: the icon set carries no down-pointing one,
            // and a character that depends on which face serves "▾" is a font question this
            // need not ask.
            let c = rect.right_center() - vec2(16.0, 1.0) + press_offset(&response);
            ui.painter().add(egui::Shape::convex_polygon(
                vec![c - vec2(4.5, 2.0), c + vec2(4.5, -2.0), c + vec2(0.0, 2.5)],
                fg,
                egui::Stroke::NONE,
            ));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// A dropdown: a pill button showing the current choice — its swatch, its label, a chevron —
/// opening our own styled popup with a swatch and label per option, a tick on the selected one.
///
/// Composed from the same pieces as every other control (the [`pill`] press feel, the
/// [`menu_item`]-shaped rows, the `egui::Popup` the track menus open through) rather than
/// egui's stock `ComboBox`, whose visuals would look like a visitor. Returns the index of the
/// option clicked this frame, if any; the popup toggles on the button and closes on a click
/// anywhere else.
pub fn dropdown(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    current_swatch: Color32,
    current: &str,
    options: &[(Color32, &str)],
) -> Option<usize> {
    let response = swatch_chip(ui, palette, current_swatch, current, false, true);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let mut choice = None;
    let popup = egui::Popup::from_response(&response)
        .id(id)
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .align(egui::RectAlign::BOTTOM_START)
        .width(DROPDOWN_WIDTH)
        .gap(4.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        // Toggle, like the page's ••• menu: the button opens from a primary click, and the
        // context-menu pattern (primary closes) would kill it on the very click that opened it.
        .open_memory(if response.clicked() {
            Some(egui::SetOpenCommand::Toggle)
        } else {
            None
        });
    popup.show(|mui| {
        egui::ScrollArea::vertical()
            .id_salt(("ytm-dropdown-scroll", id))
            .max_height(320.0)
            .show(mui, |mui| {
                for (i, (swatch, label)) in options.iter().enumerate() {
                    let row = dropdown_row(mui, palette, *swatch, label, current == *label);
                    if row.clicked() {
                        choice = Some(i);
                        mui.close();
                    }
                }
            });
    });
    choice
}

/// One row inside a [`dropdown`]: a swatch dot, a label, and a tick on the choice in force.
fn dropdown_row(
    ui: &mut Ui,
    palette: &Palette,
    swatch: Color32,
    label: &str,
    selected: bool,
) -> egui::Response {
    let height = 32.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        if let Some(fill) = state_fill(ui, palette, &response) {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS_SMALL + 2), fill);
        }
        let fg = mix(palette.secondary, palette.text, t);
        let nudge = press_offset(&response);
        ui.painter()
            .circle_filled(rect.left_center() + vec2(18.0, 0.0) + nudge, 4.5, swatch);
        crate::emoji::paint_line(
            ui,
            rect.left_center() + vec2(32.0, 0.0) + nudge,
            label,
            theme::medium(13.0),
            fg,
        );
        if selected {
            // A drawn tick rather than "✓": whether a face serves U+2713 is a font question
            // this need not ask, and two strokes are the mark everywhere else in the app too.
            let c = rect.right_center() - vec2(14.0, 1.0) + nudge;
            ui.painter().add(egui::Shape::line(
                vec![
                    c + vec2(-4.0, 0.0),
                    c + vec2(-1.0, 3.0),
                    c + vec2(4.0, -4.0),
                ],
                egui::Stroke::new(2.0, palette.accent),
            ));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// A dropdown triggered by a plain icon rather than a labelled pill — for a choice with no
/// natural swatch colour to show (a playlist's sort order, say), where painting one anyway
/// would invent a meaning that isn't there. Options are plain text, ticked on the current
/// choice; no swatch dot in front of them either, for the same reason. Returns the index of
/// the option clicked this frame, same contract as [`dropdown`].
pub fn icon_dropdown(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    icon: Icon,
    active: bool,
    current: &str,
    options: &[&str],
) -> Option<usize> {
    let response = icon_button(ui, palette, icon, 16.0, active);

    let mut choice = None;
    let popup = egui::Popup::from_response(&response)
        .id(id)
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .align(egui::RectAlign::BOTTOM_END)
        .width(DROPDOWN_WIDTH)
        .gap(4.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_memory(if response.clicked() {
            Some(egui::SetOpenCommand::Toggle)
        } else {
            None
        });
    popup.show(|mui| {
        for (i, label) in options.iter().enumerate() {
            let row = text_dropdown_row(mui, palette, label, current == *label);
            if row.clicked() {
                choice = Some(i);
                mui.close();
            }
        }
    });
    choice
}

/// One row inside an [`icon_dropdown`]: a label and a tick on the choice in force — no swatch,
/// unlike [`dropdown_row`], since there is nothing here for a colour to mean.
fn text_dropdown_row(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    selected: bool,
) -> egui::Response {
    let height = 32.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        if let Some(fill) = state_fill(ui, palette, &response) {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS_SMALL + 2), fill);
        }
        let fg = mix(palette.secondary, palette.text, t);
        let nudge = press_offset(&response);
        crate::emoji::paint_line(
            ui,
            rect.left_center() + vec2(14.0, 0.0) + nudge,
            label,
            theme::medium(13.0),
            fg,
        );
        if selected {
            // The same drawn tick `dropdown_row` uses, not "✓" — see its own comment for why.
            let c = rect.right_center() - vec2(14.0, 1.0) + nudge;
            ui.painter().add(egui::Shape::line(
                vec![
                    c + vec2(-4.0, 0.0),
                    c + vec2(-1.0, 3.0),
                    c + vec2(4.0, -4.0),
                ],
                egui::Stroke::new(2.0, palette.accent),
            ));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// [`toggle`], greyed and inert when `enabled` is false — for a setting that has nothing to
/// act on right now (no compositor blur, transparency off) rather than a switch that silently
/// does nothing when clicked.
pub fn toggle_enabled(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    on: &mut bool,
    enabled: bool,
) -> egui::Response {
    if enabled {
        return toggle(ui, palette, id, on);
    }
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(id, *on, 0.12);
        let track = palette.surface.lerp_to_gamma(palette.dim, t * 0.5);
        ui.painter()
            .rect_filled(rect, CornerRadius::same(11), track);
        let r = (rect.height() - 6.0) / 2.0;
        let cx = rect.left() + 3.0 + r + t * (rect.width() - 6.0 - 2.0 * r);
        ui.painter()
            .circle_filled(egui::pos2(cx, rect.center().y), r, palette.dim);
    }
    response
}

/// A sliding switch. Flips `on` when clicked and returns the response.
///
/// The knob travels over 120ms through `animate_bool_with_time` — bounded, pointer-started,
/// idle-safe like every M15 animation — and shrinks slightly while held.
pub fn toggle(ui: &mut Ui, palette: &Palette, id: egui::Id, on: &mut bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(id, *on, 0.12);
        let track = palette.surface_hover.lerp_to_gamma(palette.accent, t);
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(11),
            if response.is_pointer_button_down_on() {
                track.lerp_to_gamma(palette.surface_active, 0.35)
            } else {
                track
            },
        );
        let r = (rect.height() - 6.0) / 2.0;
        let cx = rect.left() + 3.0 + r + t * (rect.width() - 6.0 - 2.0 * r);
        ui.painter().circle_filled(
            egui::pos2(cx, rect.center().y),
            if response.is_pointer_button_down_on() {
                r * 0.9
            } else {
                r
            },
            palette.on_accent,
        );
    }
    response
}

/// A settings row: label and description on the left, the control right-aligned.
///
/// Reads as rows rather than a stack of labels, which is what the page was before.
pub fn setting_row(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    description: &str,
    control: impl FnOnce(&mut Ui),
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        control(ui);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(label)
                    .font(theme::medium(13.0))
                    .color(palette.text),
            );
            ui.label(
                egui::RichText::new(description)
                    .font(theme::regular(11.0))
                    .color(palette.dim),
            );
        });
    });
    ui.add_space(10.0);
}

enum PillStyle {
    /// The call to action: accent fill.
    Accent,
    /// An unselected chip.
    Quiet,
    /// A selected chip.
    Selected,
}

fn pill(ui: &mut Ui, palette: &Palette, label: &str, style: PillStyle) -> egui::Response {
    let font = theme::semibold(13.0);
    // Laid out with `PLACEHOLDER` so the colour passed to `painter.galley` is the one that
    // wins — a galley built with a real colour ignores the override, which is how the selected
    // chip ended up painting white text on a white pill.
    let galley =
        ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER));
    let size = vec2(galley.size().x + 28.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        let (fill, fg) = match style {
            PillStyle::Accent => (
                mix(palette.accent, palette.accent_hover, t),
                palette.on_accent,
            ),
            PillStyle::Quiet => (
                mix(palette.surface, palette.surface_hover, t),
                mix(palette.secondary, palette.text, t),
            ),
            PillStyle::Selected => (mix(palette.text, Color32::WHITE, t * 0.4), palette.window),
        };
        let fill = if held(&response) {
            fill.gamma_multiply(0.85)
        } else {
            fill
        };
        ui.painter().rect_filled(rect, CornerRadius::same(16), fill);
        ui.painter().galley(
            rect.center() - galley.size() / 2.0 + press_offset(&response),
            galley,
            fg,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// A thumbs-up / thumbs-down button, wherever a rating is offered.
///
/// Four states, and they are the point: at rest a dim outline with no backing; under the pointer
/// the glyph brightens and a round backing fades in; **set** it is a filled glyph in the accent
/// colour, which is what makes a rating readable across a room; held, it sinks and darkens.
/// A control that only ever changed colour is what made the old pair look bolted on.
pub fn rate_button(
    ui: &mut Ui,
    palette: &Palette,
    rect: Rect,
    id: egui::Id,
    up: bool,
    set: bool,
    draw: bool,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    if draw && ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        let backing = if held(&response) {
            Some(palette.surface_active)
        } else {
            (t > 0.001).then(|| palette.surface_hover.gamma_multiply(t))
        };
        if let Some(fill) = backing {
            ui.painter().circle_filled(rect.center(), 15.0, fill);
        }
        let icon = match (up, set) {
            (true, true) => Icon::ThumbsUpFilled,
            (true, false) => Icon::ThumbsUp,
            (false, true) => Icon::ThumbsDownFilled,
            (false, false) => Icon::ThumbsDown,
        };
        let fg = if set {
            palette.accent
        } else {
            mix(palette.dim, palette.text, t)
        };
        let size = 16.0;
        // The thumbs-down glyph hangs from its top edge; nudging it down centres the pair.
        let bias = if up { 0.0 } else { 1.0 };
        icon.image(fg, size).paint_at(
            ui,
            Rect::from_center_size(
                rect.center() + vec2(0.0, bias) + press_offset(&response),
                vec2(size, size),
            ),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// One row of a context menu: a leading glyph, a label, and an optional trailing mark.
///
/// Hand-drawn rather than `ui.button` so every row is the same height and the icon column lines
/// up — a menu of default buttons is what made the song menu read as an undifferentiated stack.
pub fn menu_item(
    ui: &mut Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    trailing: Option<&str>,
) -> egui::Response {
    let height = 32.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());

    if ui.is_rect_visible(rect) {
        let t = lit(ui, &response);
        if let Some(fill) = state_fill(ui, palette, &response) {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS_SMALL + 2), fill);
        }
        let fg = mix(palette.secondary, palette.text, t);
        let nudge = press_offset(&response);
        icon.image(fg, 15.0).paint_at(
            ui,
            Rect::from_center_size(
                rect.left_center() + vec2(18.0, 0.0) + nudge,
                vec2(15.0, 15.0),
            ),
        );
        crate::emoji::paint_line(
            ui,
            rect.left_center() + vec2(38.0, 0.0) + nudge,
            label,
            theme::medium(13.0),
            fg,
        );
        if let Some(mark) = trailing {
            ui.painter().text(
                rect.right_center() - vec2(10.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                mark,
                theme::regular(12.0),
                palette.dim,
            );
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

/// A seek bar. Returns the position to seek to, in seconds, when the user scrubs.
///
/// Drawn rather than composed from egui widgets so the hit area can be taller than the visible
/// 4px line — a hairline you have to hit exactly is miserable to use.
pub fn seek_bar(
    ui: &mut Ui,
    palette: &Palette,
    bar: Rect,
    fraction: f32,
    duration: f64,
    position: f64,
) -> Option<f64> {
    let hit = bar.expand2(vec2(0.0, 8.0));
    let response = ui.interact(hit, egui::Id::new("ytm-seek"), Sense::click_and_drag());
    let active = response.hovered() || response.dragged();

    let radius = CornerRadius::same(2);
    ui.painter()
        .rect_filled(bar, radius, palette.surface_active);
    if fraction > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height())),
            radius,
            if active {
                palette.accent_hover
            } else {
                palette.accent
            },
        );
    }
    if active && duration > 0.0 {
        ui.painter().circle_filled(
            egui::pos2(bar.min.x + bar.width() * fraction, bar.center().y),
            5.0,
            palette.accent,
        );
    }

    if duration > 0.0 {
        for (x, align, secs) in [
            (
                bar.min.x - 10.0,
                egui::Align2::RIGHT_CENTER,
                position as u64,
            ),
            (bar.max.x + 10.0, egui::Align2::LEFT_CENTER, duration as u64),
        ] {
            ui.painter().text(
                egui::pos2(x, bar.center().y),
                align,
                fmt_duration(secs),
                theme::regular(11.0),
                palette.dim,
            );
        }
    }

    if duration <= 0.0 {
        return None;
    }
    if response.clicked() || response.dragged() {
        let pos = response.interact_pointer_pos()?;
        let t = ((pos.x - bar.min.x) / bar.width()).clamp(0.0, 1.0) as f64;
        return Some(t * duration);
    }
    None
}

/// One scroll notch, in the same units as `smooth_scroll_delta`. Kept equal to the
/// `line_scroll_speed` the app sets, so a notch on the volume bar is one step by definition.
const SCROLL_PER_NOTCH: f32 = 120.0;

/// Speaker icon plus a track. Click, drag, or scroll to set the volume (0-100); unmuting
/// restores `unmute_to` — the volume you had, kept by the caller — rather than a hardcoded
/// guess.
/// A flat horizontal slider: a 4pt track with a filled portion and a knob that appears under
/// the pointer. The Settings page's transparency control — `volume_slider` keeps its own icon,
/// mute and scroll behaviour rather than growing flags.
pub fn slider(
    ui: &mut Ui,
    palette: &Palette,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    width: f32,
) -> egui::Response {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());

    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        *value = lo + t * (hi - lo);
    }

    if ui.is_rect_visible(rect) {
        let bar = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
        let t = if hi > lo {
            ((*value - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let radius = CornerRadius::same(2);
        ui.painter()
            .rect_filled(bar, radius, palette.surface_active);
        ui.painter().rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * t, bar.height())),
            radius,
            if response.hovered() || response.dragged() {
                palette.text
            } else {
                palette.secondary
            },
        );
        if response.hovered() || response.dragged() {
            ui.painter().circle_filled(
                egui::pos2(bar.min.x + bar.width() * t, bar.center().y),
                5.0,
                palette.text,
            );
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}

pub fn volume_slider(ui: &mut Ui, palette: &Palette, volume: f64, unmute_to: f64) -> Option<f64> {
    let icon = match volume {
        v if v <= 0.5 => Icon::VolumeX,
        v if v < 50.0 => Icon::Volume1,
        _ => Icon::Volume2,
    };

    let width = 76.0;
    let (track_rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    let bar = Rect::from_center_size(track_rect.center(), vec2(width, 4.0));

    let fraction = (volume / 100.0).clamp(0.0, 1.0) as f32;
    let radius = CornerRadius::same(2);
    ui.painter()
        .rect_filled(bar, radius, palette.surface_active);
    ui.painter().rect_filled(
        Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height())),
        radius,
        if response.hovered() || response.dragged() {
            palette.text
        } else {
            palette.secondary
        },
    );
    if response.hovered() || response.dragged() {
        ui.painter().circle_filled(
            egui::pos2(bar.min.x + bar.width() * fraction, bar.center().y),
            5.0,
            palette.text,
        );
    }

    // Mute toggle on the icon itself.
    let mut new_volume = None;
    if icon_button(ui, palette, icon, 16.0, false).clicked() {
        new_volume = Some(if volume > 0.5 { 0.0 } else { unmute_to });
    }

    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = ((pos.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
        new_volume = Some(f64::from(t) * 100.0);
    }

    // Scroll on the bar: one notch is one step. Normalised against the line-scroll speed so
    // the step does not inherit the faster list scrolling — and read off the bar only, so
    // the page underneath never moves.
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta != 0.0 {
            new_volume =
                Some((volume + f64::from(5.0 * delta / SCROLL_PER_NOTCH)).clamp(0.0, 100.0));
        }
    }
    new_volume
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trap that closed the song menu whenever the mouse moved: `egui::Memory` keeps
    /// **one** popup slot per viewport, so opening a second popup by id silently evicts the
    /// first. A submenu that used `Popup::open_id` therefore killed its own parent — and since
    /// the parent stopped calling `keep_popup_open`, the whole menu was gone the next frame.
    ///
    /// This asserts egui's behaviour rather than ours, because it is egui's behaviour that made
    /// the bug: anything reaching for `open_id` from inside a popup will hit it again.
    #[test]
    fn opening_a_popup_by_id_evicts_the_one_already_open() {
        let ctx = egui::Context::default();
        let parent = egui::Id::new("parent");
        let child = egui::Id::new("child");

        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Popup::open_id(ui.ctx(), parent);
        });
        frame.textures_delta.clear();
        assert!(egui::Popup::is_id_open(&ctx, parent));

        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Popup::open_id(ui.ctx(), child);
        });
        frame.textures_delta.clear();
        assert!(egui::Popup::is_id_open(&ctx, child));
        assert!(
            !egui::Popup::is_id_open(&ctx, parent),
            "one slot: the second open evicted the first"
        );
    }

    /// So no menu may reach for it. Submenus keep their own flag in `ctx.data` — see
    /// [`sub_menu`] — and the source is the only place this invariant can be checked, since a
    /// reintroduced call would look perfectly reasonable at the call site.
    #[test]
    fn no_menu_opens_a_popup_by_id() {
        let source = include_str!("widgets.rs");
        // Only the code, and only real calls: this file necessarily names the function in the
        // comments that explain why not to call it.
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        let offender = code
            .lines()
            .find(|l| !l.trim_start().starts_with("//") && l.contains("Popup::open_id("));
        assert!(
            offender.is_none(),
            "a menu is opening a popup through egui's single memory slot again: {offender:?}"
        );
    }

    #[test]
    fn durations_format_like_a_track_list() {
        assert_eq!(fmt_duration(0), "0:00");
        assert_eq!(fmt_duration(9), "0:09");
        assert_eq!(fmt_duration(62), "1:02");
        assert_eq!(fmt_duration(213), "3:33");
        assert_eq!(fmt_duration(3600), "1:00:00");
        assert_eq!(fmt_duration(3725), "1:02:05");
    }

    /// The scroll travel never leaves its rails, turns at both ends, and stands still on a
    /// zero advance — the frame that would otherwise jitter the line by a rounding error.
    #[test]
    fn marquee_travel_stays_on_its_rails() {
        // Mid-travel: advances, same heading.
        assert_eq!(pingpong(10.0, 1.0, 100.0, 5.0), (15.0, 1.0));
        // Past the far end: clamps and turns.
        assert_eq!(pingpong(98.0, 1.0, 100.0, 5.0), (100.0, -1.0));
        // Past the head: clamps and turns.
        assert_eq!(pingpong(2.0, -1.0, 100.0, 5.0), (0.0, 1.0));
        // Zero travel: still.
        assert_eq!(pingpong(0.0, 1.0, 0.0, 0.0), (0.0, -1.0));
    }

    /// A switch flips on click and only on click: hovering and pressing without releasing
    /// must leave it alone.
    #[test]
    fn a_toggle_flips_only_on_a_clean_click() {
        use egui::{Event, PointerButton, Pos2};
        use std::cell::Cell;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let palette = crate::theme::Palette::dark();
        let on = Cell::new(false);
        let at = Cell::new(Pos2::ZERO);
        let mut t = 0.0;
        let mut pass = |events: Vec<Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                time: Some(t),
                events,
                ..Default::default()
            };
            let mut flipped = on.get();
            ctx.run_ui(input, |ui| {
                let resp = toggle(ui, &palette, ui.id().with("test-toggle"), &mut flipped);
                at.set(resp.rect.center());
            })
            .drop_without_applying_deltas();
            on.set(flipped);
        };
        let click = |pos: Pos2, pressed: bool| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        pass(vec![]);
        pass(vec![Event::PointerMoved(at.get())]);
        assert!(!on.get(), "a hover flipped the switch");
        pass(vec![click(at.get(), true)]);
        assert!(!on.get(), "a press without release flipped the switch");
        pass(vec![click(at.get(), false)]);
        assert!(on.get(), "a clean click did not flip the switch");
        pass(vec![click(at.get(), true)]);
        pass(vec![click(at.get(), false)]);
        assert!(!on.get(), "a second click did not flip it back");
    }

    /// The ••• menu must open *visibly* on a primary click: open state alone is not enough.
    ///
    /// This is the regression test for a menu that toggled open on left click yet never
    /// rendered. `Toggle` flips the open flag but records no pointer position, so a
    /// pointer-anchored popup has no rect and `show` skips it — logically open, visibly
    /// absent. Button-anchored, both hold. A headless context with synthetic press/release
    /// events drives the real `page_menu`, and the test asserts open state *and* a resolved
    /// position.
    #[test]
    fn page_menu_opens_visibly_on_primary_click() {
        use egui::{Event, PointerButton, Pos2};
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        egui_extras::install_image_loaders(&ctx);
        crate::icons::register(&ctx);
        let palette = crate::theme::Palette::dark();
        let button_rect = egui::Rect::from_min_size(Pos2::new(600.0, 220.0), vec2(30.0, 30.0));
        let at = button_rect.center();
        let mut panel_id = None;
        let mut t = 0.0;
        let mut pass = |events: Vec<Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                time: Some(t),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                panel_id = Some(ui.id());
                let response = ui.interact(button_rect, ui.id().with("dots"), egui::Sense::click());
                let _ = page_menu(
                    ui,
                    &palette,
                    &response,
                    PageMenu {
                        page_id: "test",
                        tracks: &[],
                        artist_id: None,
                        playlists: &[],
                        signed_in: false,
                        share_url: String::new(),
                        has_info: false,
                        manage: false,
                        remove_from_library: false,
                        context_only: false,
                        save_as_new: None,
                    },
                );
            })
            .drop_without_applying_deltas();
        };
        let click = |pressed: bool| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        pass(vec![Event::PointerMoved(at)]);
        pass(vec![click(true)]);
        pass(vec![click(false)]);
        // One more quiet pass: the release frame toggles it open, and it must stay open.
        pass(vec![]);

        let id = panel_id
            .expect("the panel ran")
            .with(("ytm-page-menu", "test"));
        assert!(
            egui::Popup::is_id_open(&ctx, id),
            "a primary click on ••• must open the page menu"
        );
        // Rendered, not just open: a pointer-anchored popup with no recorded position
        // bails out of `show` before creating its area, so only a shown area leaves a
        // response behind.
        assert!(
            ctx.read_response(id).is_some(),
            "an open page menu that never renders is the bug"
        );
    }
}
