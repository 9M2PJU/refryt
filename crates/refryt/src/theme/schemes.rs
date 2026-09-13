//! Colour schemes: the palettes, and the one mapping that turns any of them into ours.
//!
//! A scheme is not sixteen hand-tuned slots per family — that is nine chances to drift. It is a
//! [`Ramp`]: the colours the upstream project actually publishes, in the vocabulary those
//! projects share, transcribed from their own palette files (each family's source is named
//! above its table). [`Ramp::palette`] then maps one ramp into the sixteen semantic slots, and
//! **fills every gap by derivation** — a family that publishes one background gets its gutter
//! by darkening it, a family with two text shades gets its third by fading toward the
//! background. Nothing here is a colour someone invented by eye.

use egui::Color32;

use super::Palette;

/// A scheme, as chosen in Settings. The [`Theme`](crate::settings::Theme) still decides dark or
/// light; a scheme offers one or more flavours in each polarity, and the chosen flavour paints.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum Scheme {
    /// The default family, and the app's own colours. Called `ytm` before it had a name worth
    /// showing; the alias keeps settings files and `--scheme ytm` working.
    #[default]
    #[serde(alias = "ytm")]
    Classic,
    Catppuccin,
    Nord,
    Gruvbox,
    TokyoNight,
    Dracula,
    RosePine,
    Everforest,
    Solarized,
}

/// One published flavour of one family — Catppuccin's Frappé, Rosé Pine's Moon — with its own
/// polarity, because the flavours are not symmetric: Mocha and Frappé are both dark, Latte is
/// the family's only light. Resolving a choice therefore means filtering to the flavours of the
/// polarity on screen, and the pick is remembered across polarity flips ([`Scheme::resolve`]).
#[derive(Clone, Copy)]
pub struct Variant {
    /// What the family calls it, as its users know it: "Mocha", not "dark".
    pub label: &'static str,
    /// This flavour's polarity. The [`Theme`](crate::settings::Theme) chooses between
    /// flavours; it does not re-derive them.
    pub dark: bool,
    ramp: Ramp,
}

impl Variant {
    /// The palette to paint with, derived for this flavour's own polarity — never the one on
    /// screen, which is a property of the shell, not of the flavour.
    pub fn palette(&self) -> Palette {
        self.ramp.palette(self.dark)
    }

    /// The colour a dropdown row shows for this flavour — its accent, the promise it is making.
    pub fn swatch(&self) -> Color32 {
        self.palette().accent
    }
}

impl Scheme {
    pub const ALL: [Scheme; 9] = [
        Scheme::Classic,
        Scheme::Catppuccin,
        Scheme::Nord,
        Scheme::Gruvbox,
        Scheme::TokyoNight,
        Scheme::Dracula,
        Scheme::RosePine,
        Scheme::Everforest,
        Scheme::Solarized,
    ];

    /// Parse a `--scheme` argument naming a family: the label, lowercased and stripped of
    /// spaces/hyphens, so `tokyo-night`, `tokyonight` and `Tokyo Night` all reach the same
    /// variant. See [`Scheme::parse_with_flavour`] for `catppuccin-macchiato`.
    pub fn parse(name: &str) -> Option<Self> {
        let name = Self::parse_norm(name);
        Self::ALL
            .into_iter()
            .find(|s| Self::parse_norm(s.label()) == name)
    }

    /// Parse a `--scheme` argument that may carry a flavour — `catppuccin-macchiato` — by the
    /// same loose rules; `catppuccin-frappe` and `catppuccin frappé` land on the same flavour.
    /// Returns the family alone when no flavour was named, and `None` when the text after the
    /// family label matches none of its flavours — `--scheme nord-frobnicate` must not
    /// silently become plain Nord.
    pub fn parse_with_flavour(name: &str) -> Option<(Self, Option<String>)> {
        let norm = Self::parse_norm(name);
        if let Some(family) = Self::ALL
            .into_iter()
            .find(|s| Self::parse_norm(s.label()) == norm)
        {
            return Some((family, None));
        }
        // Otherwise the longest family label that prefixes the text, with the rest naming one
        // of its flavours — longest first, so a family whose label were a prefix of another's
        // could not shadow it.
        let prefix = Self::ALL
            .into_iter()
            .filter_map(|family| {
                let label = Self::parse_norm(family.label());
                norm.strip_prefix(&label)
                    .filter(|rest| !rest.is_empty())
                    .map(|_| (family, label.len()))
            })
            .max_by_key(|(_, len)| *len)?;
        let (family, len) = prefix;
        let rest = &norm[len..];
        let flavour = family
            .variants()
            .iter()
            .find(|v| Self::parse_norm(v.label) == rest)
            .map(|v| v.label.to_owned())?;
        Some((family, Some(flavour)))
    }

    fn parse_norm(name: &str) -> String {
        let norm: String = name
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .map(|c| if c == 'é' { 'e' } else { c })
            .flat_map(|c| c.to_lowercase())
            .collect();
        // The family's old name, before it was Classic: `--scheme ytm` and `ytm-dark` must keep
        // reaching it, exactly as the serde alias keeps old settings files loading.
        match norm.strip_prefix("ytm") {
            Some(rest) => format!("classic{rest}"),
            None => norm,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scheme::Classic => "Classic",
            Scheme::Catppuccin => "Catppuccin",
            Scheme::Nord => "Nord",
            Scheme::Gruvbox => "Gruvbox",
            Scheme::TokyoNight => "Tokyo Night",
            Scheme::Dracula => "Dracula",
            Scheme::RosePine => "Rosé Pine",
            Scheme::Everforest => "Everforest",
            Scheme::Solarized => "Solarized",
        }
    }

    /// Every published flavour of the family, each with its own polarity. The order is the
    /// family's own — flavours of the same polarity appear in the order the family lists them,
    /// and the first of each polarity is what [`Scheme::default_variant`] resolves to.
    pub fn variants(self) -> &'static [Variant] {
        match self {
            Scheme::Classic => &[
                Variant {
                    label: "Dark",
                    dark: true,
                    ramp: YTM_DARK,
                },
                Variant {
                    label: "Light",
                    dark: false,
                    ramp: YTM_LIGHT,
                },
            ],
            Scheme::Catppuccin => &[
                Variant {
                    label: "Latte",
                    dark: false,
                    ramp: LATTE,
                },
                Variant {
                    label: "Frappé",
                    dark: true,
                    ramp: FRAPPE,
                },
                Variant {
                    label: "Macchiato",
                    dark: true,
                    ramp: MACCHIATO,
                },
                Variant {
                    label: "Mocha",
                    dark: true,
                    ramp: MOCHA,
                },
            ],
            Scheme::Nord => &[
                Variant {
                    label: "Polar Night",
                    dark: true,
                    ramp: NORD_DARK,
                },
                Variant {
                    label: "Snow Storm",
                    dark: false,
                    ramp: NORD_LIGHT,
                },
            ],
            Scheme::Gruvbox => &[
                Variant {
                    label: "Dark",
                    dark: true,
                    ramp: GRUVBOX_DARK,
                },
                Variant {
                    label: "Light",
                    dark: false,
                    ramp: GRUVBOX_LIGHT,
                },
            ],
            Scheme::TokyoNight => &[
                Variant {
                    label: "Night",
                    dark: true,
                    ramp: TOKYO_NIGHT,
                },
                Variant {
                    label: "Storm",
                    dark: true,
                    ramp: TOKYO_STORM,
                },
                Variant {
                    label: "Day",
                    dark: false,
                    ramp: TOKYO_DAY,
                },
            ],
            Scheme::Dracula => &[
                Variant {
                    label: "Dracula",
                    dark: true,
                    ramp: DRACULA,
                },
                Variant {
                    label: "Alucard",
                    dark: false,
                    ramp: ALUCARD,
                },
            ],
            Scheme::RosePine => &[
                Variant {
                    label: "Main",
                    dark: true,
                    ramp: ROSE_PINE,
                },
                Variant {
                    label: "Moon",
                    dark: true,
                    ramp: ROSE_PINE_MOON,
                },
                Variant {
                    label: "Dawn",
                    dark: false,
                    ramp: ROSE_PINE_DAWN,
                },
            ],
            Scheme::Everforest => &[
                Variant {
                    label: "Dark",
                    dark: true,
                    ramp: EVERFOREST_DARK,
                },
                Variant {
                    label: "Light",
                    dark: false,
                    ramp: EVERFOREST_LIGHT,
                },
            ],
            Scheme::Solarized => &[
                Variant {
                    label: "Dark",
                    dark: true,
                    ramp: SOLARIZED_DARK,
                },
                Variant {
                    label: "Light",
                    dark: false,
                    ramp: SOLARIZED_LIGHT,
                },
            ],
        }
    }

    /// The flavour of this polarity that the family paints when nothing else was chosen — the
    /// first of its kind in [`Scheme::variants`].
    pub fn default_variant(self, dark: bool) -> &'static Variant {
        self.variants()
            .iter()
            .find(|v| v.dark == dark)
            .unwrap_or_else(|| self.variants().first().expect("every family has flavours"))
    }

    /// The flavour to paint, given the polarity on screen and what the user last chose — a
    /// flavour label stored in settings. A stored flavour that is not of this polarity (Macchiato
    /// while the shell is light) or does not belong to this family (Macchiato after a trip to
    /// Nord) is simply not offered, and the family's own default paints — but the stored choice
    /// is left alone, which is exactly the memory that makes a Light → Dark → Light round trip
    /// come back to Frappé.
    pub fn resolve(self, dark: bool, chosen: Option<&str>) -> &'static Variant {
        chosen
            .and_then(|c| {
                self.variants()
                    .iter()
                    .find(|v| v.dark == dark && v.label == c)
            })
            .unwrap_or_else(|| self.default_variant(dark))
    }

    /// The palette to paint with when no flavour was chosen.
    pub fn palette(self, dark: bool) -> Palette {
        self.default_variant(dark).palette()
    }
}

/// What a scheme publishes. Seven colours are required; the rest are filled in by
/// [`Ramp::palette`] when the family does not publish them, so nobody has to invent a hex.
#[derive(Clone, Copy)]
struct Ramp {
    /// The card: the background most of the app is.
    base: u32,
    /// The gutter behind the cards. Derived by darkening (dark) or lightening (light) `base`.
    crust: Option<u32>,
    /// Menu and popup bodies. Derived as the step between `crust` and `base`.
    mantle: Option<u32>,
    /// A raised surface inside a card: chips, inputs, row hovers at rest. Derived by stepping
    /// `base` toward the text where the family publishes only backgrounds.
    surface: Option<u32>,
    /// Hovered and pressed surfaces. Derived by stepping `surface` toward the text colour.
    surface_hi: Option<u32>,
    surface_hi2: Option<u32>,
    /// Hairlines. Derived as a small step from `surface`.
    outline: Option<u32>,
    text: u32,
    /// Second-rank text. Derived by fading `text` toward `base`.
    subtext: Option<u32>,
    /// Third-rank text. Derived by fading `text` further.
    muted: Option<u32>,
    accent: u32,
    /// The hover shade of the accent. Derived by lightening (dark) or darkening (light).
    accent_hi: Option<u32>,
    /// What is legible *on* the accent — stated, never guessed; the contrast test checks it.
    on_accent: u32,
    red: u32,
    yellow: u32,
}

/// Everything optional unset, so each table writes only what its family publishes.
const RAMP: Ramp = Ramp {
    base: 0,
    crust: None,
    mantle: None,
    surface: None,
    surface_hi: None,
    surface_hi2: None,
    outline: None,
    text: 0,
    subtext: None,
    muted: None,
    accent: 0,
    accent_hi: None,
    on_accent: 0,
    red: 0,
    yellow: 0,
};

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Ramp {
    fn palette(&self, dark: bool) -> Palette {
        let published = rgb(self.base);
        let text = rgb(self.text);
        let surface = self
            .surface
            .map(rgb)
            .unwrap_or_else(|| published.lerp_to_gamma(text, 0.10));
        let accent = rgb(self.accent);
        // Darker than the card in *both* polarities — the gutter is a recess, not a highlight.
        // A light scheme lightened instead put Alucard's gutter in front of its own cards.
        // A big step down in the dark, a small one in the light, where the range is shallow.
        let base = published;
        let crust = self.crust.map(rgb).unwrap_or_else(|| {
            // 3% in the light: enough to read as a recess, shallow enough that a family whose
            // body text sits at the AA line (Solarized Light does, at 4.68:1 here) still clears
            // it on the gutter, where the now-playing title is painted.
            base.lerp_to_gamma(Color32::BLACK, if dark { 0.35 } else { 0.03 })
        });
        Palette {
            dark,
            window: crust,
            panel: base,
            surface,
            surface_hover: self
                .surface_hi
                .map(rgb)
                .unwrap_or_else(|| surface.lerp_to_gamma(text, 0.10)),
            surface_active: self
                .surface_hi2
                .map(rgb)
                .unwrap_or_else(|| surface.lerp_to_gamma(text, 0.18)),
            outline: self
                .outline
                .map(rgb)
                .unwrap_or_else(|| surface.lerp_to_gamma(text, 0.06)),
            text,
            secondary: self
                .subtext
                .map(rgb)
                .unwrap_or_else(|| text.lerp_to_gamma(base, 0.22)),
            dim: self
                .muted
                .map(rgb)
                .unwrap_or_else(|| text.lerp_to_gamma(base, 0.48)),
            accent,
            accent_hover: self.accent_hi.map(rgb).unwrap_or_else(|| {
                if dark {
                    accent.lerp_to_gamma(Color32::WHITE, 0.18)
                } else {
                    accent.lerp_to_gamma(Color32::BLACK, 0.15)
                }
            }),
            on_accent: rgb(self.on_accent),
            danger: rgb(self.red),
            warning: rgb(self.yellow),
            overlay: self
                .mantle
                .map(rgb)
                .unwrap_or_else(|| crust.lerp_to_gamma(base, 0.5)),
            shadow: Color32::from_black_alpha(if dark { 150 } else { 45 }),
            // Opaque until `Palette::with_opacity` says otherwise.
            gutter_alpha: 1.0,
            card_alpha: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The families
// ---------------------------------------------------------------------------------------------

/// Ours: a near-black gutter with YouTube Music red. Written out in full rather than derived,
/// because this is the palette every screenshot in the plan was taken against.
const YTM_DARK: Ramp = Ramp {
    base: 0x141619,
    crust: Some(0x07080a),
    mantle: Some(0x1f2126),
    surface: Some(0x1d2025),
    surface_hi: Some(0x25282d),
    surface_hi2: Some(0x2e3238),
    outline: Some(0x282b31),
    text: 0xf3f4f6,
    subtext: Some(0xa8aeb8),
    muted: Some(0x6b727d),
    accent: 0xff333d,
    accent_hi: Some(0xff555e),
    on_accent: 0xffffff,
    red: 0xf5717f,
    yellow: 0xf2b85c,
};

const YTM_LIGHT: Ramp = Ramp {
    base: 0xffffff,
    crust: Some(0xf7f8fa),
    mantle: Some(0xffffff),
    surface: Some(0xedeff2),
    surface_hi: Some(0xe2e5ea),
    surface_hi2: Some(0xd6dae0),
    outline: Some(0xdce0e5),
    text: 0x131619,
    subtext: Some(0x525a65),
    muted: Some(0x8a929d),
    accent: 0xe0212b,
    accent_hi: Some(0xc41720),
    on_accent: 0xffffff,
    red: 0xd63b4c,
    yellow: 0xb87a14,
};

// Catppuccin — github.com/catppuccin/palette, `palette.json` (v1.7.1). Four flavours, three
// dark; the slot mapping is the family's own ramp, not a guess: surface0/1/2 are the card's
// raised steps and outline, subtext1 the second-rank text, overlay0 the muted one, and the
// accent is mauve in every flavour.
const FRAPPE: Ramp = Ramp {
    base: 0x303446,
    crust: Some(0x232634),
    mantle: Some(0x292c3c),
    surface: Some(0x414559),     // surface0
    surface_hi: Some(0x51576d),  // surface1
    surface_hi2: Some(0x626880), // surface2
    outline: Some(0x51576d),
    text: 0xc6d0f5,
    subtext: Some(0xb5bfe2), // subtext1
    muted: Some(0x737994),   // overlay0
    accent: 0xca9ee6,        // mauve
    on_accent: 0x232634,
    red: 0xe78284,
    yellow: 0xe5c890,
    ..RAMP
};

const MACCHIATO: Ramp = Ramp {
    base: 0x24273a,
    crust: Some(0x181926),
    mantle: Some(0x1e2030),
    surface: Some(0x363a4f),     // surface0
    surface_hi: Some(0x494d64),  // surface1
    surface_hi2: Some(0x5b6078), // surface2
    outline: Some(0x494d64),
    text: 0xcad3f5,
    subtext: Some(0xb8c0e0), // subtext1
    muted: Some(0x6e738d),   // overlay0
    accent: 0xc6a0f6,        // mauve
    on_accent: 0x181926,
    red: 0xed8796,
    yellow: 0xeed49f,
    ..RAMP
};

const MOCHA: Ramp = Ramp {
    base: 0x1e1e2e,
    crust: Some(0x11111b),
    mantle: Some(0x181825),
    surface: Some(0x313244),
    surface_hi: Some(0x45475a),
    surface_hi2: Some(0x585b70),
    outline: Some(0x45475a),
    text: 0xcdd6f4,
    subtext: Some(0xa6adc8),
    muted: Some(0x6c7086),
    accent: 0xcba6f7, // mauve
    on_accent: 0x11111b,
    red: 0xf38ba8,
    yellow: 0xf9e2af,
    ..RAMP
};

const LATTE: Ramp = Ramp {
    base: 0xeff1f5,
    crust: Some(0xdce0e8),
    mantle: Some(0xe6e9ef),
    surface: Some(0xccd0da),
    surface_hi: Some(0xbcc0cc),
    surface_hi2: Some(0xacb0be),
    outline: Some(0xccd0da),
    text: 0x4c4f69,
    subtext: Some(0x6c6f85),
    muted: Some(0x8c8fa1),
    accent: 0x8839ef,
    on_accent: 0xeff1f5,
    red: 0xd20f39,
    yellow: 0xdf8e1d,
    ..RAMP
};

// Nord — nordtheme.com/docs/colors-and-palettes. Polar Night is the dark family; there is no
// official light theme, so the Snow Storm greys stand in for one, as they conventionally do.
const NORD_DARK: Ramp = Ramp {
    base: 0x3b4252, // nord1
    crust: Some(0x2e3440),
    mantle: Some(0x2e3440),
    surface: Some(0x434c5e), // nord2
    surface_hi: Some(0x4c566a),
    outline: Some(0x434c5e),
    text: 0xeceff4,
    subtext: Some(0xd8dee9),
    accent: 0x88c0d0, // frost
    on_accent: 0x2e3440,
    red: 0xbf616a,
    yellow: 0xebcb8b,
    ..RAMP
};

const NORD_LIGHT: Ramp = Ramp {
    base: 0xeceff4, // nord6
    crust: Some(0xd8dee9),
    mantle: Some(0xe5e9f0),
    surface: Some(0xe5e9f0),
    surface_hi: Some(0xd8dee9),
    outline: Some(0xd8dee9),
    text: 0x2e3440,
    subtext: Some(0x434c5e),
    muted: Some(0x4c566a),
    accent: 0x5e81ac,
    on_accent: 0xeceff4,
    red: 0xbf616a,
    yellow: 0xd08770,
    ..RAMP
};

// Gruvbox — github.com/morhetz/gruvbox.
const GRUVBOX_DARK: Ramp = Ramp {
    base: 0x282828, // bg0
    crust: Some(0x1d2021),
    mantle: Some(0x1d2021),
    surface: Some(0x3c3836), // bg1
    surface_hi: Some(0x504945),
    surface_hi2: Some(0x665c54),
    outline: Some(0x3c3836),
    text: 0xebdbb2, // fg1
    subtext: Some(0xd5c4a1),
    muted: Some(0x928374),
    accent: 0xfe8019, // orange
    on_accent: 0x282828,
    red: 0xfb4934,
    yellow: 0xfabd2f,
    ..RAMP
};

const GRUVBOX_LIGHT: Ramp = Ramp {
    base: 0xfbf1c7,
    crust: Some(0xebdbb2),
    mantle: Some(0xf9f5d7),
    surface: Some(0xebdbb2),
    surface_hi: Some(0xd5c4a1),
    surface_hi2: Some(0xbdae93),
    outline: Some(0xd5c4a1),
    text: 0x3c3836,
    subtext: Some(0x504945),
    muted: Some(0x7c6f64),
    accent: 0xaf3a03,
    on_accent: 0xfbf1c7,
    red: 0x9d0006,
    yellow: 0xb57614,
    ..RAMP
};

// Tokyo Night — github.com/folke/tokyonight.nvim.
const TOKYO_NIGHT: Ramp = Ramp {
    base: 0x1a1b26,
    crust: Some(0x16161e),
    mantle: Some(0x16161e),
    surface: Some(0x292e42), // bg_highlight
    surface_hi: Some(0x3b4261),
    surface_hi2: Some(0x414868), // terminal_black
    outline: Some(0x292e42),
    text: 0xc0caf5,
    subtext: Some(0xa9b1d6),
    muted: Some(0x565f89), // comment
    accent: 0x7aa2f7,
    on_accent: 0x1a1b26,
    red: 0xf7768e,
    yellow: 0xe0af68,
    ..RAMP
};

// Storm, the family's middle flavour: the same accents as Night over a lighter blue-grey, so
// the mapping carries over wholesale — bg, bg_dark, bg_highlight, terminal_black and the
// comment/fg_dark text ramp all swap one-for-one from `colors/storm.lua`.
const TOKYO_STORM: Ramp = Ramp {
    base: 0x24283b,        // bg
    crust: Some(0x1f2335), // bg_dark
    mantle: Some(0x1f2335),
    surface: Some(0x292e42),     // bg_highlight
    surface_hi: Some(0x3b4261),  // fg_gutter
    surface_hi2: Some(0x414868), // terminal_black
    outline: Some(0x292e42),
    text: 0xc0caf5,
    subtext: Some(0xa9b1d6), // fg_dark
    muted: Some(0x565f89),   // comment
    accent: 0x7aa2f7,
    on_accent: 0x24283b,
    red: 0xf7768e,
    yellow: 0xe0af68,
    ..RAMP
};

const TOKYO_DAY: Ramp = Ramp {
    base: 0xe1e2e7,
    crust: Some(0xd0d5e3),
    mantle: Some(0xd0d5e3),
    surface: Some(0xc4c8da), // bg_highlight
    surface_hi: Some(0xa1a6c5),
    outline: Some(0xc4c8da),
    // Its published `fg` #3760bf is a mid blue that clears AA on `bg` alone (4.52:1) and misses
    // it on both raised steps (3.99 and 3.52). We carry prose on all three, so the text is that
    // blue deepened 20% toward black — the smallest step that clears 4.5 everywhere, measured
    // at 4.80 / 5.44 / 6.18. The only slot in the nine families that is not a published hex,
    // and it is a derivation from one rather than a colour picked by eye.
    text: 0x2c4d99,
    subtext: Some(0x6172b0),
    muted: Some(0x848cb5),
    accent: 0x2e7de9,
    on_accent: 0xe1e2e7,
    red: 0xf52a65,
    yellow: 0x8c6c3e,
    ..RAMP
};

// Dracula — spec.draculatheme.com. It publishes one background and one raised surface, so the
// gutter and the menu body are derived from them.
const DRACULA: Ramp = Ramp {
    base: 0x282a36,
    surface: Some(0x44475a), // current line
    outline: Some(0x44475a),
    text: 0xf8f8f2,
    muted: Some(0x6272a4), // comment
    accent: 0xbd93f9,      // purple
    on_accent: 0x282a36,
    red: 0xff5555,
    yellow: 0xf1fa8c,
    ..RAMP
};

const ALUCARD: Ramp = Ramp {
    base: 0xfffbeb,
    surface: Some(0xcfcfde), // selection
    outline: Some(0xcfcfde),
    text: 0x1f1f1f,
    muted: Some(0x6c664b), // comment
    accent: 0x644ac9,
    on_accent: 0xfffbeb,
    red: 0xcb3a2a,
    yellow: 0x846e15,
    ..RAMP
};

// Rosé Pine — github.com/rose-pine/palette, `palette.json`.
const ROSE_PINE: Ramp = Ramp {
    base: 0x1f1d2e, // surface
    crust: Some(0x191724),
    mantle: Some(0x26233a),
    surface: Some(0x26233a), // overlay
    text: 0xe0def4,
    subtext: Some(0x908caa),
    muted: Some(0x6e6a86),
    // Iris, not rose: rose is the family's prettiest colour but its Dawn shade cannot carry a
    // legible label, and a scheme whose accent changes hue between polarities is two schemes.
    accent: 0xc4a7e7, // iris
    on_accent: 0x191724,
    red: 0xeb6f92, // love
    yellow: 0xf6c177,
    ..RAMP
};

// Moon, the family's darker-leaning second flavour — same roles, same iris accent as Main,
// each colour read from its own flavour table in `palette.json`.
const ROSE_PINE_MOON: Ramp = Ramp {
    base: 0x2a273f,          // surface
    crust: Some(0x232136),   // base
    mantle: Some(0x393552),  // overlay
    surface: Some(0x393552), // overlay
    text: 0xe0def4,
    subtext: Some(0x908caa), // subtle
    muted: Some(0x6e6a86),
    accent: 0xc4a7e7, // iris
    on_accent: 0x232136,
    red: 0xeb6f92,    // love
    yellow: 0xf6c177, // gold
    ..RAMP
};

const ROSE_PINE_DAWN: Ramp = Ramp {
    base: 0xfffaf3, // surface
    crust: Some(0xf2e9e1),
    mantle: Some(0xfaf4ed),
    surface: Some(0xf2e9e1),
    text: 0x464261,
    subtext: Some(0x797593),
    muted: Some(0x9893a5),
    accent: 0x907aa9, // iris
    on_accent: 0xfffaf3,
    red: 0xb4637a,
    yellow: 0xea9d34,
    ..RAMP
};

// Everforest — github.com/sainnhe/everforest, `palette.md`, medium contrast.
const EVERFOREST_DARK: Ramp = Ramp {
    base: 0x2d353b, // bg0
    crust: Some(0x232a2e),
    mantle: Some(0x232a2e),
    surface: Some(0x343f44), // bg1
    surface_hi: Some(0x3d484d),
    surface_hi2: Some(0x475258),
    outline: Some(0x3d484d),
    text: 0xd3c6aa,
    subtext: Some(0x9da9a0),
    muted: Some(0x859289),
    accent: 0xa7c080, // green
    on_accent: 0x2d353b,
    red: 0xe67e80,
    yellow: 0xdbbc7f,
    ..RAMP
};

const EVERFOREST_LIGHT: Ramp = Ramp {
    base: 0xfdf6e3, // bg0
    crust: Some(0xefebd4),
    mantle: Some(0xefebd4),
    surface: Some(0xf4f0d9), // bg1
    surface_hi: Some(0xefebd4),
    surface_hi2: Some(0xe6e2cc),
    outline: Some(0xe6e2cc),
    text: 0x5c6a72,
    subtext: Some(0x829181),
    muted: Some(0x939f91),
    // Everforest Light is a pastel family: none of its published accents can carry a label at
    // 3:1 (its green is 2.69, its best is blue at 3.13). So the green is deepened 12% toward
    // black — 3.40:1 — the same treatment Tokyo Night Day's text gets, and for the same reason.
    accent: 0x7c8e01,
    on_accent: 0xfdf6e3,
    red: 0xf85552,
    yellow: 0xdfa000,
    ..RAMP
};

// Solarized — ethanschoonover.com/solarized. Its two backgrounds are base03/base02 (dark) and
// base3/base2 (light); the text ramp is base0/base1 and base00/base01.
const SOLARIZED_DARK: Ramp = Ramp {
    base: 0x073642, // base02
    crust: Some(0x002b36),
    mantle: Some(0x002b36),
    // Solarized publishes two dark backgrounds and no raised step, so the surfaces are derived.
    // The text is base2 rather than base1: base1 is the editor's emphasis grey and clears AA on
    // the two backgrounds only, which leaves nothing legible on a hovered row.
    text: 0xeee8d5, // base2
    subtext: Some(0x93a1a1),
    muted: Some(0x839496),
    accent: 0x268bd2,
    on_accent: 0x002b36,
    red: 0xdc322f,
    yellow: 0xb58900,
    ..RAMP
};

const SOLARIZED_LIGHT: Ramp = Ramp {
    base: 0xfdf6e3, // base3
    // base2 as the gutter would be the obvious mapping, but base01 on it is 4.39:1 and the
    // player bar's title sits there. A derived step down from base3 keeps the recess and the
    // contrast both.
    mantle: Some(0xeee8d5),
    surface: Some(0xeee8d5), // base2
    surface_hi: Some(0xe3dcc6),
    outline: Some(0xe3dcc6),
    // base02 for the text, not base01: Solarized's palette is symmetric and names base02/base03
    // as light mode's emphasised content, and base01 — its body grey — is 4.39:1 on base2, its
    // own raised surface. This is the family's own step up in weight, not a colour of ours.
    text: 0x073642,          // base02
    subtext: Some(0x586e75), // base01
    muted: Some(0x657b83),   // base00
    accent: 0x268bd2,
    on_accent: 0xfdf6e3,
    red: 0xdc322f,
    yellow: 0xb58900,
    ..RAMP
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance per WCAG, on straight (non-premultiplied) sRGB.
    fn luminance(c: Color32) -> f32 {
        let ch = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Nine families copied by hand from nine upstreams is nine chances to pick a text colour
    /// that cannot be read on its own background. Every flavour of every family, in the
    /// polarity its own table declares, one loop.
    #[test]
    fn every_flavour_is_readable() {
        for scheme in Scheme::ALL {
            for variant in scheme.variants() {
                let p = variant.palette();
                let name = format!("{} {}", scheme.label(), variant.label);
                let check = |what: &str, fg: Color32, bg: Color32, min: f32| {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= min,
                        "{name}: {what} is {ratio:.2}:1, under {min}:1 ({fg:?} on {bg:?})"
                    );
                };
                check("text on the card", p.text, p.panel, 4.5);
                check("text on the gutter", p.text, p.window, 4.5);
                check("text on a surface", p.text, p.surface, 4.5);
                // Second-rank text is smaller but still prose; the large-text bar is 3:1.
                check("secondary on the card", p.secondary, p.panel, 3.0);
                // The accent carries the play button's glyph and the primary pill's label. 3:1
                // is WCAG's bar for a UI component rather than prose, and it is the bar the
                // published accents actually meet: white on Nord's darkest frost — which is
                // what Nord's own interfaces use — lands at exactly 3.5.
                check("on_accent on the accent", p.on_accent, p.accent, 3.0);
                // A chip inverts: its label is painted in the window colour on a text-coloured
                // pill, which only works while those two are far apart.
                check("a selected chip's label", p.window, p.text, 4.5);
            }
        }
    }

    /// A flavour that paints the same as another is a menu entry that does nothing.
    #[test]
    fn no_two_flavours_are_the_same() {
        let all: Vec<(String, Palette)> = Scheme::ALL
            .iter()
            .flat_map(|s| {
                s.variants()
                    .iter()
                    .map(|v| (format!("{} {}", s.label(), v.label), v.palette()))
            })
            .collect();
        for (i, (a_name, a)) in all.iter().enumerate() {
            for (b_name, b) in &all[i + 1..] {
                assert_ne!(a, b, "{a_name} and {b_name} are identical");
            }
        }
    }

    /// Every flavour is what its table declares — dark is dark, light is light. The polarity
    /// flag drives `Visuals` and the shadows, so a flavour whose flag lied would look inverted
    /// rather than wrong.
    #[test]
    fn the_polarity_matches_the_colours() {
        for scheme in Scheme::ALL {
            for variant in scheme.variants() {
                let p = variant.palette();
                if variant.dark {
                    assert!(
                        luminance(p.panel) < 0.2,
                        "{} {}: the dark flavour is not dark",
                        scheme.label(),
                        variant.label
                    );
                } else {
                    assert!(
                        luminance(p.panel) > 0.5,
                        "{} {}: the light flavour is not light",
                        scheme.label(),
                        variant.label
                    );
                }
            }
        }
    }

    /// The gutter reads as *behind* the cards in both polarities: darker in a dark scheme,
    /// darker in a light one too — it is a recess either way, never brighter than the card.
    #[test]
    fn the_gutter_sits_behind_the_cards() {
        for scheme in Scheme::ALL {
            for variant in scheme.variants() {
                let p = variant.palette();
                assert!(
                    luminance(p.window) < luminance(p.panel),
                    "{} {}: the gutter is not behind the card",
                    scheme.label(),
                    variant.label
                );
            }
        }
    }

    /// The default scheme is exactly what the app painted before schemes existed.
    #[test]
    fn the_default_scheme_is_unchanged() {
        assert_eq!(Scheme::default(), Scheme::Classic);
        assert_eq!(Scheme::Classic.label(), "Classic");
        assert_eq!(
            Scheme::parse("ytm"),
            Some(Scheme::Classic),
            "the family's old name still reaches it"
        );
        let dark = Scheme::Classic.palette(true);
        assert_eq!(dark.window, Color32::from_rgb(0x07, 0x08, 0x0a));
        assert_eq!(dark.panel, Color32::from_rgb(0x14, 0x16, 0x19));
        assert_eq!(dark.accent, Color32::from_rgb(0xff, 0x33, 0x3d));
        assert_eq!(dark.accent_hover, Color32::from_rgb(0xff, 0x55, 0x5e));
        let light = Scheme::Classic.palette(false);
        assert_eq!(light.panel, Color32::WHITE);
        assert_eq!(light.accent, Color32::from_rgb(0xe0, 0x21, 0x2b));
    }
}
