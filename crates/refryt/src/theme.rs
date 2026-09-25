//! Palette, typography, icons, metrics, and the base egui style.
//!
//! The structure here — a semantic `Palette` with light and dark variants, plus Inter registered
//! once per weight — is adapted from fastpotify's `src/theme.rs`, which is MIT
//! licensed (Copyright (c) Carmine Paolino). The colours, metrics and icon set are our own.
//!
//! Design intent: *dense content, fancy shell*. Rows are tight; the polish lives in the frameless
//! window, the shadows and the scheme. Transitions are bounded and pointer-started, so an app
//! nobody is touching still asks for no frames.

#![allow(dead_code)] // design system built ahead of the pages that consume it

use egui::{Color32, CornerRadius, Stroke};

pub mod custom;
mod ramp;

// ---------------------------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------------------------

/// Cards, buttons, cover art at rest sizes; the art's own radius grows from this with its
/// size, up to [`ART_RADIUS_MAX`].
pub const RADIUS: u8 = 8;
/// The most a cover's corners ever round. A 200px card at a fixed 8 reads nearly square —
/// the curve has to grow with the art, but a radius past this starts eating the image.
pub const ART_RADIUS_MAX: u8 = 16;
/// Inputs and chips.
pub const RADIUS_SMALL: u8 = 4;
/// The frameless window itself.
pub const WINDOW_RADIUS: u8 = 20;

pub const ROW_HEIGHT: f32 = 56.0;
pub const ART_SIZE: f32 = 40.0;
/// A track row's title. A step above the subtitle's 12 so the song reads first and the credits
/// under it second, without losing the list's density.
pub const ROW_TITLE_PT: f32 = 14.0;
/// A page's own title — "Your library", "Settings", "Results for …". Well clear of the section
/// headings under it, so the page reads title, then sections, then rows.
pub const PAGE_TITLE_PT: f32 = 26.0;
/// A section heading inside a page — "Listen again", "Top songs", "Albums". A clear step above
/// a row title; at the row title's own size the sections dissolved into their contents.
pub const SECTION_PT: f32 = 20.0;
/// Where a track row's number/art block starts, from the row's own left edge. Zero so the list
/// lines up with the page header's cover, which starts at the content origin.
pub const ROW_LEFT_INSET: f32 = 0.0;
/// The width of a numbered list's rank column before the art. Zero on unnumbered rows.
pub const ROW_NUMBER_W: f32 = 26.0;
/// The gap between a number and the cover beside it — the padding that keeps the two from
/// reading as one unit.
pub const ROW_NUMBER_GAP: f32 = 12.0;
pub const PLAYER_BAR_HEIGHT: f32 = 88.0;
pub const SIDEBAR_WIDTH: f32 = 280.0;
pub const UPNEXT_WIDTH: f32 = 288.0;
pub const TITLEBAR_HEIGHT: f32 = 56.0;

/// The gap between panels, through which the window background shows.
///
/// The shell is rounded cards floating on a near-black gutter rather than flat panels divided by
/// hairlines. It is the single strongest cue that separates this look from a plain toolkit
/// layout, and it costs an outer margin rather than any layout arithmetic.
pub const GUTTER: f32 = 8.0;

/// The corner radius of those cards — the page card, the sidebar's, the queue panel's.
///
/// Ten read as a chamfer on a card a thousand points wide; sixteen still read short at the top,
/// where the header wash meets the gutter at full contrast and the eye measures the curve
/// hardest. Twenty is where the curve survives that scrutiny. The header wash caps itself in
/// the same constant, so the tint never pokes past the card's own corners.
pub const PANEL_RADIUS: u8 = 20;

/// Every card's inner margins: a small side padding, and none top or bottom.
///
/// The sides keep text off the card's own edge. The top and bottom are zero on purpose: each
/// card's scroll viewport runs its full height, so content is clipped at the card's top and
/// bottom edges instead of at a margin inside them — the whole card is content, and the cut
/// lands on the card's silhouette. The rounded corners that clip cannot follow are covered by
/// [`crate::widgets::paint_corner_notches`].
pub const CARD_INSET: egui::Margin = egui::Margin {
    left: 8,
    right: 8,
    top: 0,
    bottom: 0,
};

/// [`CARD_INSET`]'s side padding as a scalar, for the corner repaint on the rail and queue cards.
pub const CARD_INSET_X: f32 = 8.0;

/// The page card's inner margins. Wider at the sides than the rail and queue cards: the page
/// holds titles, grids and long lists, and at 8 they sat cramped against the card's rounded
/// edge. At the card radius the content column clears the corners outright, so no content
/// ever reaches outside the silhouette.
pub const PAGE_INSET: egui::Margin = egui::Margin {
    left: PANEL_RADIUS as i8,
    right: PANEL_RADIUS as i8,
    top: 0,
    bottom: 0,
};

/// [`PAGE_INSET`]'s side padding as a scalar, for the geometry that grows the page's content
/// rect back out to the card's own edges — the header wash, the artist hero, the corner repaint.
pub const PAGE_INSET_X: f32 = PANEL_RADIUS as f32;

/// The page header's own top gap inside the content card.
///
/// The card has no top margin any more, so this is the whole distance from its top edge to the
/// header's art — the same distance the art has from the card's side, so the cover sits square
/// in the corner rather than closer to one edge than the other. Shared by the real header and
/// the loading skeleton, which would otherwise jump when the page arrives.
pub const HEADER_TOP_PAD: f32 = PAGE_INSET_X;

/// A rail row: cover, title, author.
pub const RAIL_ROW_HEIGHT: f32 = 60.0;
/// Cover size on a rail row.
pub const RAIL_ART_SIZE: f32 = 44.0;
/// A sidebar nav row — Home, Search, Library.
pub const NAV_HEIGHT: f32 = 44.0;

/// Width of the invisible band along each window edge that begins a resize.
pub const RESIZE_HANDLE: f32 = 6.0;

/// How long a hover or press takes to settle.
///
/// The speed budget originally forbade animation outright, on the grounds that it implies
/// continuous repaint — correct in general, and the reason the app felt inert. The rule that
/// replaces it: every transition is bounded, driven by `Context::animate_bool_with_time`, and
/// begins only from a pointer interaction. An app nobody is touching still asks for no frames,
/// which is exactly what the idle phase of `scripts/perf.sh` measures.
pub const HOVER_FADE_SECS: f32 = 0.12;

// ---------------------------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Behind everything, including the rounded window corners.
    pub window: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    pub outline: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub overlay: Color32,
    pub shadow: Color32,
    /// How opaque the gutter, the top bar and the player bar paint. 1.0 unless the acrylic
    /// mode is on.
    ///
    /// Two scalars rather than alpha in the colour slots, because the slots are read for more
    /// than backgrounds: `window` is also the *label* colour of a selected chip, and `surface`
    /// and `accent` are dimmed by `gamma_multiply` on a press. An alpha stored in either would
    /// paint half-transparent text and double-dim a press.
    pub gutter_alpha: f32,
    /// How opaque a card's own fill paints, given the gutter is already down underneath it.
    pub card_alpha: f32,
}

impl Palette {
    /// The default theme's dark palette. Kept as a name because half the app's tests and the
    /// `--decorated` fallback path reach for it directly.
    pub fn dark() -> Self {
        custom::fallback(true).palette()
    }

    pub fn light() -> Self {
        custom::fallback(false).palette()
    }

    /// A copy that paints its backgrounds at `opacity`.
    ///
    /// The gutter and a card are painted one on top of the other, so a single alpha would leave
    /// the card — the biggest surface on screen, and the one people mean by "acrylic" — at
    /// `1 - (1 - a)²`, near-opaque at any value the gutter can survive. So each level names what
    /// the two should *composite* to, and the card's own fill alpha is solved for.
    pub fn with_opacity(mut self, opacity: crate::settings::Opacity) -> Self {
        let (gutter, card) = (opacity.gutter(), opacity.card());
        self.gutter_alpha = gutter;
        self.card_alpha = if gutter >= 1.0 {
            1.0
        } else {
            ((card - gutter) / (1.0 - gutter)).clamp(0.0, 1.0)
        };
        self
    }

    /// The window background as painted: the gutter colour at the shell's opacity.
    pub fn window_fill(&self) -> Color32 {
        self.window.gamma_multiply(self.gutter_alpha)
    }

    /// A card's fill as painted. Composites with [`Self::window_fill`] underneath it.
    pub fn card_fill(&self) -> Color32 {
        self.panel.gamma_multiply(self.card_alpha)
    }

    /// Whether anything shows through. What the blur asks before it bothers.
    pub fn translucent(&self) -> bool {
        self.gutter_alpha < 1.0
    }
}

// ---------------------------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------------------------

const INTER_MEDIUM: &str = "inter-medium";
const INTER_SEMIBOLD: &str = "inter-semibold";
const INTER_BOLD: &str = "inter-bold";
/// The emoji face, shared by every weight — emoji have no weights to speak of.
const EMOJI: &str = "noto-emoji";

// `FontFamily::Name` wraps an `Arc<str>` precisely so it is cheap to clone, but `"…".into()`
// builds a fresh one — a heap allocation and copy — on every call, and `medium()`/`semibold()`
// are called a few times per visible row. Build each once; a clone is then a refcount bump.
static FAMILY_MEDIUM: std::sync::LazyLock<egui::FontFamily> =
    std::sync::LazyLock::new(|| egui::FontFamily::Name(INTER_MEDIUM.into()));
static FAMILY_SEMIBOLD: std::sync::LazyLock<egui::FontFamily> =
    std::sync::LazyLock::new(|| egui::FontFamily::Name(INTER_SEMIBOLD.into()));
static FAMILY_BOLD: std::sync::LazyLock<egui::FontFamily> =
    std::sync::LazyLock::new(|| egui::FontFamily::Name(INTER_BOLD.into()));

pub fn regular(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, FAMILY_MEDIUM.clone())
}

pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FAMILY_SEMIBOLD.clone())
}

pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FAMILY_BOLD.clone())
}

/// Inter is a variable font, but egui picks one instance per registered face. Registering the
/// same bytes several times, each pinned to a different `wght` coordinate, is how you get real
/// weights rather than faked ones.
pub(crate) fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::VariationCoords;
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    const INTER: &[u8] = include_bytes!("../../../assets/fonts/InterVariable.ttf");
    // Playlist names are full of emoji. egui's bundled emoji faces are a 2006-era subset — 🗿 and
    // ↖️ are there, 🫧 is not, and a codepoint no face covers is drawn as a replacement box. Noto
    // Emoji (OFL, monochrome) covers the modern set, including the VARIATION SELECTOR-16 that
    // follows half of them and would otherwise draw a box of its own.
    //
    // Monochrome is the ceiling, not a preference: epaint rasterises glyph *outlines* through
    // skrifa and reads neither COLR nor the CBDT bitmaps that `NotoColorEmoji.ttf` is made of, so
    // a colour face here would render nothing at all.
    const NOTO_EMOJI: &[u8] = include_bytes!("../../../assets/fonts/NotoEmoji.ttf");

    let weighted = |weight: f32| {
        let mut data = FontData::from_static(INTER);
        data.tweak.coords = VariationCoords::new([(b"wght", weight)]);
        Arc::new(data)
    };

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".to_owned(), weighted(400.0));
    fonts.font_data.insert(
        EMOJI.to_owned(),
        Arc::new(FontData::from_static(NOTO_EMOJI)),
    );
    fonts
        .font_data
        .insert(INTER_MEDIUM.to_owned(), weighted(500.0));
    fonts
        .font_data
        .insert(INTER_SEMIBOLD.to_owned(), weighted(600.0));
    fonts
        .font_data
        .insert(INTER_BOLD.to_owned(), weighted(700.0));

    // Inter leads the proportional family, then Noto Emoji, then egui's bundled faces as
    // fallbacks for scripts neither covers. Order is precedence: the first face with the glyph
    // wins, so Inter keeps the text-style arrows and dingbats it draws better itself.
    //
    // Between the two sits whatever CJK faces the system carries, loaded from disk rather than
    // bundled — Noto Sans CJK JP alone is ~19 MB across the weights, and a music app that
    // cannot show its users' own song titles is broken. Nothing here ships a font that large
    // for the one machine in ten that has no CJK installed at all; without one, titles fall
    // back to the boxes they were, which was never this code's making.
    let mut cjk_names = Vec::new();
    for (name, bytes) in system_cjk_fonts() {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        cjk_names.push(name);
    }
    let head = fonts.families.entry(FontFamily::Proportional).or_default();
    head.insert(0, "inter".to_owned());
    head.insert(1, EMOJI.to_owned());
    for (i, name) in cjk_names.iter().enumerate() {
        head.insert(2 + i, (*name).to_owned());
    }

    let fallbacks: Vec<String> = fonts.families[&FontFamily::Proportional]
        .iter()
        .skip(1)
        .cloned()
        .collect();
    for name in [INTER_MEDIUM, INTER_SEMIBOLD, INTER_BOLD] {
        let mut family = vec![name.to_owned()];
        family.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }

    ctx.set_fonts(fonts);
}

/// Every system CJK face worth registering, one per script, in fallback order.
///
/// Linux carries a single super-TTC that covers Japanese, Korean and Chinese, so one entry does
/// it. Windows ships a face per script and none covers the others' kana or hangul — Yu Gothic
/// for Japanese, Malgun Gothic for Korean, Microsoft YaHei for Chinese — so all three go in,
/// Japanese first to match the Linux preference. Each group takes the first path that exists;
/// a group with nothing installed simply contributes nothing.
///
/// Mapped, not read: `NotoSansCJK-Regular.ttc` is a ~20 MB super-TTC, and a music app that
/// cannot show its users' own song titles is broken — but almost none of those 20 MB needs to
/// be resident. `std::fs::read` would keep the whole file in a process-lifetime heap `Vec`
/// (the largest single line item in the idle figure on any machine with `noto-cjk`); a map is
/// file-backed and only the pages holding glyphs actually drawn are ever faulted in.
fn system_cjk_fonts() -> Vec<(&'static str, &'static [u8])> {
    #[cfg(target_os = "windows")]
    const GROUPS: &[(&str, &[&str])] = &[
        (
            "system-cjk-jp",
            &[
                // Yu Gothic ships on Windows 10 and 11; Meiryo and MS Gothic are the older
                // installs.
                r"C:\Windows\Fonts\YuGothM.ttc",
                r"C:\Windows\Fonts\meiryo.ttc",
                r"C:\Windows\Fonts\msgothic.ttc",
            ],
        ),
        (
            "system-cjk-kr",
            &[
                r"C:\Windows\Fonts\malgun.ttf",
                r"C:\Windows\Fonts\gulim.ttc",
                r"C:\Windows\Fonts\batang.ttc",
            ],
        ),
        (
            "system-cjk-sc",
            &[
                r"C:\Windows\Fonts\msyh.ttc",
                r"C:\Windows\Fonts\msjh.ttc",
                r"C:\Windows\Fonts\simsun.ttc",
            ],
        ),
    ];
    // Fedora and Arch keep Noto under `opentype/noto`, Debian and KDE under `noto-cjk`; the
    // Regular weight is preferred so titles match the UI's own voice, and the DemiLight
    // commonly-installed variant fills in where it does not exist.
    #[cfg(not(target_os = "windows"))]
    const GROUPS: &[(&str, &[&str])] = &[(
        "system-cjk",
        &[
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-DemiLight.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-DemiLight.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK.ttc",
        ],
    )];

    GROUPS
        .iter()
        .filter_map(|(name, paths)| {
            paths
                .iter()
                .find_map(|path| mapped_font(path).map(|bytes| (*name, bytes)))
        })
        .collect()
}

/// Map a font file and leak the mapping — see [`system_cjk_fonts`] for why a map, and why
/// leaking is the lifetime epaint wants: `&'static [u8]` has to outlive every glyph lookup
/// egui makes, i.e. the whole process, which is exactly what a leak gives it.
fn mapped_font(path: &str) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: as in `emoji::load_font` — a font file replaced under a running process is seen
    // as changed bytes; skrifa bounds-checks every read and a bad parse yields fallback boxes,
    // never UB.
    let map = (unsafe { memmap2::Mmap::map(&file) }).ok()?;
    Some(&**Box::leak(Box::new(map)))
}

// ---------------------------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------------------------

/// Install fonts, icons and image loaders. Call once at startup.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    crate::icons::register(ctx);
    egui_extras::install_image_loaders(ctx);
    // egui's native default is 40px per wheel line, which makes long lists feel sticky. It is
    // a constant — set it once here, not on every frame.
    ctx.options_mut(|o| o.input_options.line_scroll_speed = 120.0);
}

/// Push a palette into egui's own style, so built-in widgets agree with our custom ones.
pub fn apply(ctx: &egui::Context, palette: &Palette) {
    let mut style = (*ctx.global_style()).clone();

    style.visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let v = &mut style.visuals;

    v.dark_mode = palette.dark;
    v.panel_fill = palette.panel;
    v.window_fill = palette.overlay;
    v.extreme_bg_color = palette.surface;
    v.faint_bg_color = palette.surface;
    v.code_bg_color = palette.surface;
    v.override_text_color = Some(palette.text);
    v.weak_text_color = Some(palette.secondary);
    v.hyperlink_color = palette.accent;
    v.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, palette.accent);
    // No hairline borders on popups and menus: the overlay fill plus the shadow separates
    // them from the page, and every divider is supposed to be gone. Strokes drawn
    // explicitly — the toast, the window chrome, the focused search ring — are unaffected.
    v.window_stroke = Stroke::NONE;
    v.window_corner_radius = CornerRadius::same(RADIUS + 2);
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: palette.shadow,
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: palette.shadow,
    };

    let corner = CornerRadius::same(RADIUS_SMALL + 2);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = corner;
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.0, palette.text);
        w.expansion = 0.0;
    }
    // egui's own widgets grow a little under the pointer. Ours draw their own states — see
    // `widgets::lit` — so this only reaches the handful of stock widgets left (the settings
    // page, the sort row), and it keeps them feeling like the rest.
    v.widgets.hovered.expansion = 1.0;
    v.widgets.active.expansion = 0.0;
    v.widgets.inactive.bg_fill = palette.surface;
    v.widgets.inactive.weak_bg_fill = palette.surface;
    v.widgets.hovered.bg_fill = palette.surface_hover;
    v.widgets.hovered.weak_bg_fill = palette.surface_hover;
    v.widgets.active.bg_fill = palette.surface_active;
    v.widgets.active.weak_bg_fill = palette.surface_active;
    v.widgets.noninteractive.corner_radius = corner;
    v.widgets.noninteractive.bg_fill = palette.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.outline);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    // No scrollbars anywhere: floating so none is carved out of a list's width, allocating no
    // width even while floating, and every opacity at zero so nothing paints in any state.
    // Set on the style rather than per `ScrollArea`, so the ones egui builds inside its own
    // popups are covered too and a scroll area added later cannot bring a bar back. Wheel,
    // drag and keyboard scrolling are untouched — this hides the indicator, not the scrolling.
    style.spacing.scroll.floating = true;
    style.spacing.scroll.bar_width = 0.0;
    style.spacing.scroll.floating_width = 0.0;
    style.spacing.scroll.floating_allocated_width = 0.0;
    style.spacing.scroll.bar_inner_margin = 0.0;
    style.spacing.scroll.bar_outer_margin = 0.0;
    style.spacing.scroll.foreground_color = false;
    for opacity in [
        &mut style.spacing.scroll.dormant_background_opacity,
        &mut style.spacing.scroll.active_background_opacity,
        &mut style.spacing.scroll.interact_background_opacity,
        &mut style.spacing.scroll.dormant_handle_opacity,
        &mut style.spacing.scroll.active_handle_opacity,
        &mut style.spacing.scroll.interact_handle_opacity,
    ] {
        *opacity = 0.0;
    }
    // A sibling of the opacities above, on the same `scroll` struct but easy to miss: egui
    // paints its own 20px edge fade at the top/bottom of every `ScrollArea` once scrolled off
    // the top, at 50% strength by default, with no scrollbar involved. It fades to
    // `ui.stack().bg_color()` — a guess at what is behind the content — and content is clipped
    // at the card's own edges here anyway (`CARD_INSET` has no top or bottom), so there is
    // nothing for it to fade. The app's own dissolves (`widgets::fade_slices`) fade the text's
    // own alpha where a line is cut, which needs no background guess.
    style.spacing.scroll.fade.strength = 0.0;
    // Menus are drawn by `widgets::menu_item`, which paints its own padding.
    style.spacing.menu_margin = egui::Margin::symmetric(4, 6);
    style.spacing.menu_spacing = 0.0;

    style.text_styles = [
        (egui::TextStyle::Heading, semibold(20.0)),
        (egui::TextStyle::Body, regular(13.0)),
        (egui::TextStyle::Button, medium(13.0)),
        (egui::TextStyle::Small, regular(11.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(12.0)),
    ]
    .into();

    ctx.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Opacity::OFF` must be an identity: the default build paints exactly the colours it
    /// painted before opacity existed, for every one of the 256 values a channel can hold —
    /// not spot-checked, because `gamma_multiply(1.0)` being an identity is the whole argument
    /// for applying alpha at paint time rather than threading a flag through every call site.
    #[test]
    fn opacity_off_changes_nothing() {
        for v in 0..=255u8 {
            assert_eq!(
                Color32::from_rgb(v, v, v).gamma_multiply(1.0),
                Color32::from_rgb(v, v, v)
            );
        }
        let p = Palette::dark().with_opacity(crate::settings::Opacity::OFF);
        assert_eq!(p.window_fill(), p.window);
        assert_eq!(p.card_fill(), p.panel);
        assert!(!p.translucent());
    }

    /// Every shipped theme, opaque, is exactly what it was before opacity applied to it — the
    /// property the transparency slider exists not to disturb for anyone who leaves it at the
    /// default.
    #[test]
    fn opacity_off_is_identity_for_every_theme() {
        for (name, css) in custom::BUILTIN {
            let base = custom::parse(name, css).unwrap().palette();
            let opaque = base.with_opacity(crate::settings::Opacity::OFF);
            assert_eq!(opaque.window_fill(), base.window);
            assert_eq!(opaque.card_fill(), base.panel);
        }
    }

    /// Whether the installed stack can draw a character.
    ///
    /// Not `Fonts::has_glyph`: epaint answers that by comparing the resolved face against the
    /// face it draws the replacement box with, and that face is the *emoji* face here (Inter has
    /// no `◻`, Noto Emoji does) — so it reports every emoji as missing, which is epaint's own
    /// documented false negative. A zero advance is the honest signal: a face that does not have
    /// the character measures it as nothing.
    fn can_draw(ctx: &egui::Context, c: char) -> bool {
        ctx.fonts_mut(|f| f.glyph_width(&regular(13.0), c)) > 0.0
    }

    fn fonts_ready() -> egui::Context {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        // Fonts are built during a frame, not at `set_fonts`, and the frame's atlas delta is a
        // panic if dropped unhandled.
        let mut frame = ctx.run_ui(egui::RawInput::default(), |_| {});
        frame.textures_delta.clear();
        ctx
    }

    /// An emoji written with VARIATION SELECTOR-16 takes its image's width plus a point, whatever
    /// the text face's own advance for the character — Inter's arrows are much narrower than the
    /// image, and counting the selector as a box put a wide gap after each. Invisible characters
    /// take no room.
    #[test]
    fn emoji_arrows_sit_side_by_side() {
        let ctx = fonts_ready();
        let font = medium(13.5);
        let one = crate::emoji::emoji_side(13.5) + 1.0;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            let w = |s: &str| crate::widgets::text_width(ui, s, &font);
            assert!((w("↖\u{FE0F}") - one).abs() < 0.01, "{}", w("↖\u{FE0F}"));
            assert!((w("↖\u{FE0F}↗\u{FE0F}↘\u{FE0F}") - 3.0 * one).abs() < 0.01);
            assert_eq!(w("\u{200D}\u{FE0E}\u{2060}"), 0.0);
            // A character wider than its image keeps its own advance.
            assert!(w("🫧\u{FE0F}") >= w("🫧"));
        });
        frame.textures_delta.clear();
    }

    /// Real playlist names from a real library. Every character has to be drawable — the
    /// alternative is not a missing emoji but a replacement box, which is how these read before
    /// Noto Emoji was bundled: egui's own emoji faces are a 2006 subset with no 🫧 in them, and
    /// no face at all covered the VARIATION SELECTOR-16 that follows half of these.
    #[test]
    fn the_font_stack_covers_the_emoji_in_playlist_names() {
        let ctx = fonts_ready();
        for name in [
            "Χάουζ 🫧",
            "🕶️",
            "↖️↗️↘️↗️",
            "Playlist 🗿",
            "Τι ακούω ενώ παίζω Forza Horizon 5",
        ] {
            // Variation selectors are zero-width by design and are covered by their own test.
            for c in name
                .chars()
                .filter(|c| !matches!(c, '\u{FE0E}' | '\u{FE0F}'))
            {
                assert!(
                    can_draw(&ctx, c),
                    "no glyph for {c:?} (U+{:04X}) in {name:?}",
                    c as u32
                );
            }
        }
    }

    /// Whatever CJK face the system carries must actually resolve the scripts it was loaded
    /// for. Skipped when the machine has none — that fallback is best-effort by design — but
    /// with one installed, kana and hangul must not come out as boxes. Windows is the case this
    /// pins: its faces are one per script, so a missing path list (the bug) renders Japanese
    /// and Korean song titles as replacement glyphs.
    #[test]
    fn the_system_cjk_font_covers_kana_and_hangul() {
        if system_cjk_fonts().is_empty() {
            return;
        }
        let ctx = fonts_ready();
        for c in ['あ', 'ア', '漢', '한', '글'] {
            assert!(
                can_draw(&ctx, c),
                "no glyph for {c:?} (U+{:04X}) despite a system CJK face",
                c as u32
            );
        }
    }

    /// An unassigned code point, as the negative control: without one, the coverage test above
    /// would pass even if `can_draw` always said yes.
    #[test]
    fn an_uncovered_character_is_reported_as_such() {
        let ctx = fonts_ready();
        assert!(!can_draw(&ctx, '\u{0378}'));
    }

    /// Half of these names carry a VARIATION SELECTOR-16 after the emoji, asking for the colour
    /// presentation we cannot give it. It must contribute nothing — no face covering it means a
    /// replacement box drawn after every single emoji.
    #[test]
    fn a_variation_selector_adds_no_box() {
        let ctx = fonts_ready();
        let width = |s: &str| {
            ctx.fonts_mut(|f| {
                f.layout_no_wrap(s.to_owned(), regular(13.0), egui::Color32::WHITE)
                    .rect
                    .width()
            })
        };
        assert_eq!(width("\u{1F576}\u{FE0F}"), width("\u{1F576}"));
    }
}
